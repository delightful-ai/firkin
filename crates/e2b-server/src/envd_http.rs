//! envd http — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::HttpBody;
#[allow(unused_imports)]
use crate::auth_error_to_http;
#[allow(unused_imports)]
use crate::backend::BoxError;
#[allow(unused_imports)]
use crate::envd_data_event_proto;
#[allow(unused_imports)]
use crate::envd_filesystem_watch_dir_response_proto;
#[allow(unused_imports)]
use crate::envd_process_event_proto;
#[allow(unused_imports)]
use crate::envd_process_input_proto;
#[allow(unused_imports)]
use crate::envd_process_selector_proto;
#[allow(unused_imports)]
use crate::full_body;
#[allow(unused_imports)]
use crate::header_matches;
#[allow(unused_imports)]
use crate::registry::normalize_volume_path;
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
#[allow(unused_imports)]
use bytes::Bytes;
#[allow(unused_imports)]
use firkin_e2b_contract::BackendError;
#[allow(unused_imports)]
use firkin_e2b_wire::{LogLevel, SandboxLogEntry};
#[allow(unused_imports)]
use firkin_envd::process_output_events;
#[allow(unused_imports)]
use firkin_envd::{
    EnvdFilesystemAdapter, EnvdFilesystemEntry, EnvdFilesystemEvent, EnvdFilesystemEventStream,
    EnvdFilesystemEventType, EnvdFilesystemFileType, EnvdFilesystemWriteInfo, EnvdProcessAdapter,
    EnvdProcessEventStream, EnvdProcessInfo, EnvdProcessInput, EnvdProcessOutput,
    EnvdProcessSelector, EnvdProcessSignal, EnvdProcessStartRequest, EnvdProcessStreamEvent,
    EnvdPtySize,
};
#[allow(unused_imports)]
use flate2::read::GzDecoder;
use http_body_util::BodyExt as _;
#[allow(unused_imports)]
use http_body_util::channel::Channel;
#[allow(unused_imports)]
use hyper::HeaderMap;
#[allow(unused_imports)]
use hyper::Method;
#[allow(unused_imports)]
use hyper::body::Incoming;
#[allow(unused_imports)]
use hyper::header::CONTENT_ENCODING;
#[allow(unused_imports)]
use hyper::header::CONTENT_TYPE;
#[allow(unused_imports)]
use hyper::service::service_fn;
#[allow(unused_imports)]
use hyper::{Request, Response, StatusCode};
#[allow(unused_imports)]
use hyper_util::rt::TokioIo;
#[allow(unused_imports)]
use prost::Message;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use std::convert::Infallible;
use std::io::Read as _;
#[allow(unused_imports)]
use std::net::SocketAddr;
#[allow(unused_imports)]
use std::path::Component;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use std::time::UNIX_EPOCH;
#[allow(unused_imports)]
use time::OffsetDateTime;
#[allow(unused_imports)]
use time::format_description::well_known::Rfc3339;
#[allow(unused_imports)]
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
#[allow(unused_imports)]
use tokio::net::TcpListener;
#[allow(unused_imports)]
use tokio::process::{Child, Command};
#[allow(unused_imports)]
use tokio::sync::Mutex;
#[allow(unused_imports)]
use tokio::sync::mpsc;
/// Hyper-backed HTTP server for the envd process and filesystem APIs.
#[derive(Clone, Debug)]
pub struct EnvdProcessHttpServer<A> {
    pub(crate) adapter: A,
    access_token: Option<String>,
}
impl<A> EnvdProcessHttpServer<A>
where
    A: EnvdProcessAdapter<Error = BackendError> + EnvdFilesystemAdapter<Error = BackendError>,
{
    /// Construct an envd HTTP server around an adapter.
    #[must_use]
    pub const fn new(adapter: A) -> Self {
        Self {
            adapter,
            access_token: None,
        }
    }
    /// Require SDK `x-access-token` envd authentication.
    #[must_use]
    pub fn with_access_token(mut self, access_token: impl Into<String>) -> Self {
        self.access_token = Some(access_token.into());
        self
    }
    /// Return the envd adapter.
    #[must_use]
    pub const fn adapter(&self) -> &A {
        &self.adapter
    }
    /// Serve envd process and filesystem HTTP requests on a listener.
    ///
    /// # Errors
    ///
    /// Returns listener accept errors.
    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, _) = listener.accept().await?;
            let adapter = self.adapter.clone();
            let access_token = self.access_token.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let adapter = adapter.clone();
                    let access_token = access_token.clone();
                    async move {
                        Ok::<_, Infallible>(
                            handle_envd_process_request_authenticated(
                                adapter,
                                access_token.as_deref(),
                                request,
                            )
                            .await,
                        )
                    }
                });
                let stream = TokioIo::new(stream);
                if let Err(_error) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(stream, service)
                    .await
                {}
            });
        }
    }
    /// Bind and serve envd process and filesystem HTTP requests.
    ///
    /// # Errors
    ///
    /// Returns listener bind or accept errors.
    pub async fn bind_and_serve(addr: SocketAddr, adapter: A) -> std::io::Result<()> {
        let listener = TcpListener::bind(addr).await?;
        Self::new(adapter).serve(listener).await
    }
}
/// Host-backed envd adapter rooted at one directory.
///
/// This adapter is useful for local SDK compatibility: process working
/// directories and filesystem paths are resolved under `root`, and path
/// traversal outside that root is rejected. It is not a VM sandbox or a Cube
/// runtime adapter.
#[derive(Clone, Debug)]
pub struct HostEnvdAdapter {
    pub(crate) root: Arc<PathBuf>,
    base_envs: Arc<BTreeMap<String, String>>,
    #[allow(missing_docs)]
    pub state: Arc<Mutex<HostEnvdState>>,
}
impl HostEnvdAdapter {
    /// Construct a host-backed envd adapter rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the root directory cannot be created.
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        Self::new_with_envs(root, BTreeMap::new()).await
    }
    /// Construct a host-backed envd adapter with sandbox-level environment.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the root directory cannot be created.
    pub async fn new_with_envs(
        root: impl Into<PathBuf>,
        base_envs: BTreeMap<String, String>,
    ) -> Result<Self, BackendError> {
        let root = root.into();
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|error| BackendError::Runtime(format!("create envd root: {error}")))?;
        Ok(Self {
            root: Arc::new(root),
            base_envs: Arc::new(base_envs),
            state: Arc::new(Mutex::new(HostEnvdState::default())),
        })
    }
    /// Return the host root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_ref()
    }
    fn resolve_path(&self, path: &str) -> Result<PathBuf, BackendError> {
        let mut resolved = (*self.root).clone();
        for component in Path::new(path).components() {
            match component {
                Component::Prefix(_) | Component::ParentDir => {
                    return Err(BackendError::Runtime(format!(
                        "envd path `{path}` escapes adapter root"
                    )));
                }
                Component::RootDir | Component::CurDir => {}
                Component::Normal(part) => resolved.push(part),
            }
        }
        Ok(resolved)
    }
    fn display_path(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }
    fn effective_envs(&self, request: &EnvdProcessStartRequest) -> BTreeMap<String, String> {
        let mut envs = self.base_envs.as_ref().clone();
        envs.extend(request.envs.clone());
        envs
    }
    fn entry_from_metadata(
        path: String,
        metadata: &std::fs::Metadata,
    ) -> Result<EnvdFilesystemEntry, BackendError> {
        let name = path
            .rsplit('/')
            .find(|part| !part.is_empty())
            .unwrap_or("/")
            .to_owned();
        let file_type = if metadata.is_dir() {
            EnvdFilesystemFileType::Directory
        } else {
            EnvdFilesystemFileType::File
        };
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt as _;
            metadata.permissions().mode()
        };
        #[cfg(not(unix))]
        let mode = if metadata.is_dir() {
            0o040755
        } else {
            0o100644
        };
        let permissions = match file_type {
            EnvdFilesystemFileType::Directory => "drwxr-xr-x",
            EnvdFilesystemFileType::File => "-rw-r--r--",
        };
        let size = i64::try_from(metadata.len())
            .map_err(|_| BackendError::Runtime(format!("file `{path}` is too large")))?;
        Ok(EnvdFilesystemEntry {
            name,
            path,
            file_type,
            size,
            mode,
            permissions: permissions.to_owned(),
            owner: "host".to_owned(),
            group: "host".to_owned(),
            symlink_target: None,
        })
    }
    #[allow(missing_docs)]
    pub async fn entry_for(&self, envd_path: &str) -> Result<EnvdFilesystemEntry, BackendError> {
        let host_path = self.resolve_path(envd_path)?;
        let metadata =
            tokio::fs::symlink_metadata(&host_path)
                .await
                .map_err(|error| match error.kind() {
                    std::io::ErrorKind::NotFound => BackendError::NotFound(envd_path.to_owned()),
                    _ => BackendError::Runtime(format!("stat `{envd_path}`: {error}")),
                })?;
        Self::entry_from_metadata(normalize_envd_path(envd_path), &metadata)
    }
    async fn run_command(
        &self,
        request: &EnvdProcessStartRequest,
    ) -> Result<EnvdProcessOutput, BackendError> {
        if host_mcp_gateway_request(request) {
            return self.run_host_mcp_gateway_stub(request).await;
        }
        if request.cmd.is_empty() {
            return Err(BackendError::Runtime(
                "process command is required".to_owned(),
            ));
        }
        let program = &request.cmd;
        let cwd = match request.cwd.as_deref().filter(|cwd| !cwd.is_empty()) {
            Some(cwd) => self.resolve_path(cwd)?,
            None => (*self.root).clone(),
        };
        tokio::fs::create_dir_all(&cwd)
            .await
            .map_err(|error| BackendError::Runtime(format!("create process cwd: {error}")))?;
        let mut command = Command::new(program);
        command.args(&request.args).current_dir(cwd);
        let envs = self.effective_envs(request);
        for (key, value) in &envs {
            command.env(key, value);
        }
        let output = command
            .output()
            .await
            .map_err(|error| BackendError::Runtime(format!("run process `{program}`: {error}")))?;
        let exit_code = output.status.code().unwrap_or(128);
        Ok(EnvdProcessOutput {
            pid: 0,
            stdout: output.stdout,
            stderr: output.stderr,
            pty: Vec::new(),
            exit_code,
            exited: true,
            status: if output.status.success() {
                "exited".to_owned()
            } else {
                "errored".to_owned()
            },
            error: (!output.status.success()).then(|| format!("process exited {exit_code}")),
        })
    }
    async fn run_host_mcp_gateway_stub(
        &self,
        request: &EnvdProcessStartRequest,
    ) -> Result<EnvdProcessOutput, BackendError> {
        let token = request.envs.get("GATEWAY_ACCESS_TOKEN").ok_or_else(|| {
            BackendError::Runtime("MCP gateway request missing GATEWAY_ACCESS_TOKEN".to_owned())
        })?;
        let token_path = self.resolve_path("/etc/mcp-gateway/.token")?;
        if let Some(parent) = token_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| BackendError::Runtime(format!("create MCP token dir: {error}")))?;
        }
        tokio::fs::write(&token_path, token)
            .await
            .map_err(|error| BackendError::Runtime(format!("write MCP token: {error}")))?;
        Ok(EnvdProcessOutput {
            pid: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
            pty: Vec::new(),
            exit_code: 0,
            exited: true,
            status: "exited".to_owned(),
            error: None,
        })
    }
    async fn spawn_command(
        &self,
        request: &EnvdProcessStartRequest,
    ) -> Result<Child, BackendError> {
        if request.cmd.is_empty() {
            return Err(BackendError::Runtime(
                "process command is required".to_owned(),
            ));
        }
        let cwd = match request.cwd.as_deref().filter(|cwd| !cwd.is_empty()) {
            Some(cwd) => self.resolve_path(cwd)?,
            None => (*self.root).clone(),
        };
        tokio::fs::create_dir_all(&cwd)
            .await
            .map_err(|error| BackendError::Runtime(format!("create process cwd: {error}")))?;
        let mut command = Command::new(&request.cmd);
        command
            .args(&request.args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let envs = self.effective_envs(request);
        for (key, value) in &envs {
            command.env(key, value);
        }
        command.spawn().map_err(|error| {
            BackendError::Runtime(format!("spawn process `{}`: {error}", request.cmd))
        })
    }
    async fn allocate_process(
        &self,
        request: &EnvdProcessStartRequest,
        stdin: Option<mpsc::Sender<Vec<u8>>>,
        child: Option<Arc<Mutex<Child>>>,
    ) -> u32 {
        let mut state = self.state.lock().await;
        state.next_pid = state.next_pid.saturating_add(1).max(1);
        let pid = state.next_pid;
        state.processes.insert(
            pid,
            HostEnvdProcessRecord {
                tag: request.tag.clone(),
                cmd: request.cmd.clone(),
                args: request.args.clone(),
                envs: self.effective_envs(request),
                cwd: request.cwd.clone(),
                output: EnvdProcessOutput {
                    pid,
                    status: "running".to_owned(),
                    ..EnvdProcessOutput::default()
                },
                stdin,
                child,
            },
        );
        pid
    }
    async fn append_process_output(&self, pid: u32, output: ProcessOutputKind, bytes: &[u8]) {
        let mut state = self.state.lock().await;
        let Some(record) = state.processes.get_mut(&pid) else {
            return;
        };
        match output {
            ProcessOutputKind::Stdout => record.output.stdout.extend_from_slice(bytes),
            ProcessOutputKind::Stderr => record.output.stderr.extend_from_slice(bytes),
        }
    }
    async fn finish_process(
        &self,
        pid: u32,
        exit_code: i32,
        status: String,
        error: Option<String>,
    ) {
        let mut state = self.state.lock().await;
        let Some(record) = state.processes.get_mut(&pid) else {
            return;
        };
        record.output.exit_code = exit_code;
        record.output.exited = true;
        record.output.status = status;
        record.output.error = error;
        record.stdin = None;
        record.child = None;
    }
    pub(crate) async fn captured_process_logs(&self) -> Vec<SandboxLogEntry> {
        let state = self.state.lock().await;
        let timestamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("RFC3339 formatting current UTC time is infallible");
        let mut logs = Vec::new();
        for (pid, record) in &state.processes {
            if !record.output.stdout.is_empty() {
                logs.push(SandboxLogEntry {
                    timestamp: timestamp.clone(),
                    level: LogLevel::Info,
                    message: String::from_utf8_lossy(&record.output.stdout).into_owned(),
                    fields: process_log_fields(*pid, "stdout", record),
                });
            }
            if !record.output.stderr.is_empty() {
                logs.push(SandboxLogEntry {
                    timestamp: timestamp.clone(),
                    level: LogLevel::Error,
                    message: String::from_utf8_lossy(&record.output.stderr).into_owned(),
                    fields: process_log_fields(*pid, "stderr", record),
                });
            }
            if let Some(error) = &record.output.error {
                logs.push(SandboxLogEntry {
                    timestamp: timestamp.clone(),
                    level: LogLevel::Error,
                    message: error.clone(),
                    fields: process_log_fields(*pid, "exit", record),
                });
            }
        }
        logs
    }
}
#[async_trait]
impl EnvdProcessAdapter for HostEnvdAdapter {
    type Error = BackendError;

    async fn list_processes(&self) -> Result<Vec<EnvdProcessInfo>, BackendError> {
        let state = self.state.lock().await;
        Ok(state
            .processes
            .iter()
            .map(|(pid, record)| EnvdProcessInfo {
                pid: *pid,
                tag: record.tag.clone(),
                cmd: record.cmd.clone(),
                args: record.args.clone(),
                envs: record.envs.clone(),
                cwd: record.cwd.clone(),
            })
            .collect())
    }
    async fn send_process_input(
        &self,
        selector: EnvdProcessSelector,
        input: EnvdProcessInput,
    ) -> Result<(), BackendError> {
        let pid = self.require_process(selector).await?;
        let sender = {
            let state = self.state.lock().await;
            state
                .processes
                .get(&pid)
                .and_then(|record| record.stdin.clone())
        }
        .ok_or_else(|| BackendError::Runtime(format!("process {pid} stdin is closed")))?;
        let bytes = match input {
            EnvdProcessInput::Stdin(bytes) | EnvdProcessInput::Pty(bytes) => bytes,
        };
        sender
            .send(bytes)
            .await
            .map_err(|_| BackendError::Runtime(format!("process {pid} stdin is closed")))
    }
    async fn close_process_stdin(&self, selector: EnvdProcessSelector) -> Result<(), BackendError> {
        let pid = self.require_process(selector).await?;
        let mut state = self.state.lock().await;
        let record = state
            .processes
            .get_mut(&pid)
            .ok_or_else(|| BackendError::NotFound(pid.to_string()))?;
        record.stdin = None;
        Ok(())
    }
    async fn signal_process(
        &self,
        selector: EnvdProcessSelector,
        signal: EnvdProcessSignal,
    ) -> Result<(), BackendError> {
        let pid = self.require_process(selector).await?;
        let child = {
            let state = self.state.lock().await;
            state
                .processes
                .get(&pid)
                .and_then(|record| record.child.clone())
        };
        let Some(child) = child else {
            return Ok(());
        };
        let mut child = child.lock().await;
        match signal {
            EnvdProcessSignal::Unspecified => {}
            EnvdProcessSignal::Sigterm
            | EnvdProcessSignal::Sigkill
            | EnvdProcessSignal::Unknown(_) => {
                child.start_kill().map_err(|error| {
                    BackendError::Runtime(format!("kill process {pid}: {error}"))
                })?;
            }
        }
        Ok(())
    }
    async fn connect_process(
        &self,
        selector: EnvdProcessSelector,
    ) -> Result<EnvdProcessOutput, BackendError> {
        let pid = self.require_process(selector).await?;
        let state = self.state.lock().await;
        state
            .processes
            .get(&pid)
            .map(|record| record.output.clone())
            .ok_or_else(|| BackendError::NotFound(pid.to_string()))
    }
    async fn update_process_pty(
        &self,
        selector: EnvdProcessSelector,
        _pty: Option<EnvdPtySize>,
    ) -> Result<(), BackendError> {
        self.require_process(selector).await.map(|_| ())
    }
    async fn start_process(
        &self,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessOutput, BackendError> {
        let envs = self.effective_envs(&request);
        let mut output = self.run_command(&request).await?;
        let mut state = self.state.lock().await;
        state.next_pid = state.next_pid.saturating_add(1).max(1);
        let pid = state.next_pid;
        output.pid = pid;
        state.processes.insert(
            pid,
            HostEnvdProcessRecord {
                tag: request.tag,
                cmd: request.cmd,
                args: request.args,
                envs,
                cwd: request.cwd,
                output: output.clone(),
                stdin: None,
                child: None,
            },
        );
        Ok(output)
    }
    async fn start_process_stream(
        &self,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<BackendError>, BackendError> {
        if host_mcp_gateway_request(&request) {
            let output = self.start_process(request).await?;
            return Ok(EnvdProcessEventStream::from_output(&output));
        }
        let mut child = self.spawn_command(&request).await?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdin = child.stdin.take();
        let (input_sender, mut input_receiver) = mpsc::channel::<Vec<u8>>(16);
        if let Some(mut stdin) = stdin {
            tokio::spawn(async move {
                while let Some(bytes) = input_receiver.recv().await {
                    if stdin.write_all(&bytes).await.is_err() {
                        break;
                    }
                    if stdin.flush().await.is_err() {
                        break;
                    }
                }
            });
        }
        let child = Arc::new(Mutex::new(child));
        let pid = self
            .allocate_process(&request, Some(input_sender), Some(child.clone()))
            .await;
        let (sender, receiver) = mpsc::channel(32);
        sender
            .try_send(Ok(EnvdProcessStreamEvent::Start { pid }))
            .expect("fresh process event stream channel has capacity");
        if let Some(stdout) = stdout {
            spawn_host_output_reader(
                self.clone(),
                pid,
                stdout,
                ProcessOutputKind::Stdout,
                sender.clone(),
            );
        }
        if let Some(stderr) = stderr {
            spawn_host_output_reader(
                self.clone(),
                pid,
                stderr,
                ProcessOutputKind::Stderr,
                sender.clone(),
            );
        }
        let adapter = self.clone();
        tokio::spawn(async move {
            let status = child.lock().await.wait().await;
            let (exit_code, status_text, error) = match status {
                Ok(status) => {
                    let exit_code = status.code().unwrap_or(128);
                    let status_text = if status.success() {
                        "exited"
                    } else {
                        "errored"
                    };
                    (
                        exit_code,
                        status_text.to_owned(),
                        (!status.success()).then(|| format!("process exited {exit_code}")),
                    )
                }
                Err(error) => (
                    128,
                    "errored".to_owned(),
                    Some(format!("wait failed: {error}")),
                ),
            };
            adapter
                .finish_process(pid, exit_code, status_text.clone(), error.clone())
                .await;
            let _ = sender
                .send(Ok(EnvdProcessStreamEvent::End {
                    exit_code,
                    exited: true,
                    status: status_text,
                    error,
                }))
                .await;
        });
        Ok(EnvdProcessEventStream { receiver })
    }
}
impl HostEnvdAdapter {
    async fn require_process(&self, selector: EnvdProcessSelector) -> Result<u32, BackendError> {
        let state = self.state.lock().await;
        match selector {
            EnvdProcessSelector::Pid(pid) if state.processes.contains_key(&pid) => Ok(pid),
            EnvdProcessSelector::Pid(pid) => Err(BackendError::NotFound(pid.to_string())),
            EnvdProcessSelector::Tag(tag) => state
                .processes
                .iter()
                .find_map(|(pid, record)| (record.tag.as_deref() == Some(&tag)).then_some(*pid))
                .ok_or(BackendError::NotFound(tag)),
        }
    }
}
#[async_trait]
impl EnvdFilesystemAdapter for HostEnvdAdapter {
    type Error = BackendError;

    async fn read_file(&self, path: String) -> Result<Vec<u8>, BackendError> {
        let host_path = self.resolve_path(&path)?;
        tokio::fs::read(&host_path)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => BackendError::NotFound(path),
                _ => BackendError::Runtime(format!(
                    "read `{}`: {error}",
                    Self::display_path(&host_path)
                )),
            })
    }
    async fn write_file(
        &self,
        path: String,
        data: Vec<u8>,
    ) -> Result<EnvdFilesystemWriteInfo, BackendError> {
        let host_path = self.resolve_path(&path)?;
        if let Some(parent) = host_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| BackendError::Runtime(format!("create parent: {error}")))?;
        }
        let mut file = tokio::fs::File::create(&host_path)
            .await
            .map_err(|error| BackendError::Runtime(format!("create `{path}`: {error}")))?;
        file.write_all(&data)
            .await
            .map_err(|error| BackendError::Runtime(format!("write `{path}`: {error}")))?;
        file.flush()
            .await
            .map_err(|error| BackendError::Runtime(format!("flush `{path}`: {error}")))?;
        Ok(EnvdFilesystemWriteInfo {
            name: path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(&path)
                .to_owned(),
            file_type: "file".to_owned(),
            path: normalize_envd_path(&path),
        })
    }
    async fn list_dir(
        &self,
        path: String,
        depth: u32,
    ) -> Result<Vec<EnvdFilesystemEntry>, BackendError> {
        let start = self.resolve_path(&path)?;
        let mut entries = Vec::new();
        let mut pending = vec![(normalize_envd_path(&path), start, 0_u32)];
        while let Some((envd_dir, host_dir, current_depth)) = pending.pop() {
            let mut read_dir = tokio::fs::read_dir(&host_dir)
                .await
                .map_err(|error| match error.kind() {
                    std::io::ErrorKind::NotFound => BackendError::NotFound(envd_dir.clone()),
                    _ => BackendError::Runtime(format!("list `{envd_dir}`: {error}")),
                })?;
            while let Some(entry) = read_dir
                .next_entry()
                .await
                .map_err(|error| BackendError::Runtime(format!("list `{envd_dir}`: {error}")))?
            {
                let name = entry.file_name().to_string_lossy().into_owned();
                let envd_path = join_envd_path(&envd_dir, &name);
                let metadata = entry.metadata().await.map_err(|error| {
                    BackendError::Runtime(format!("stat `{envd_path}`: {error}"))
                })?;
                let is_dir = metadata.is_dir();
                entries.push(Self::entry_from_metadata(envd_path.clone(), &metadata)?);
                if is_dir && (depth == 0 || current_depth + 1 < depth) {
                    pending.push((envd_path, entry.path(), current_depth + 1));
                }
            }
        }
        Ok(entries)
    }
    async fn make_dir(&self, path: String) -> Result<EnvdFilesystemEntry, BackendError> {
        let host_path = self.resolve_path(&path)?;
        tokio::fs::create_dir_all(&host_path)
            .await
            .map_err(|error| BackendError::Runtime(format!("mkdir `{path}`: {error}")))?;
        self.entry_for(&path).await
    }
    async fn move_entry(
        &self,
        source: String,
        destination: String,
    ) -> Result<EnvdFilesystemEntry, BackendError> {
        let source_path = self.resolve_path(&source)?;
        let destination_path = self.resolve_path(&destination)?;
        if let Some(parent) = destination_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| BackendError::Runtime(format!("create parent: {error}")))?;
        }
        tokio::fs::rename(&source_path, &destination_path)
            .await
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => BackendError::NotFound(source),
                _ => BackendError::Runtime(format!("move to `{destination}`: {error}")),
            })?;
        self.entry_for(&destination).await
    }
    async fn remove_entry(&self, path: String) -> Result<(), BackendError> {
        let host_path = self.resolve_path(&path)?;
        let metadata =
            tokio::fs::symlink_metadata(&host_path)
                .await
                .map_err(|error| match error.kind() {
                    std::io::ErrorKind::NotFound => BackendError::NotFound(path.clone()),
                    _ => BackendError::Runtime(format!("stat `{path}`: {error}")),
                })?;
        let result = if metadata.is_dir() {
            tokio::fs::remove_dir_all(&host_path).await
        } else {
            tokio::fs::remove_file(&host_path).await
        };
        result.map_err(|error| BackendError::Runtime(format!("remove `{path}`: {error}")))
    }
    async fn stat_entry(&self, path: String) -> Result<EnvdFilesystemEntry, BackendError> {
        self.entry_for(&path).await
    }
    async fn watch_dir(
        &self,
        path: String,
        recursive: bool,
    ) -> Result<Vec<EnvdFilesystemEvent>, BackendError> {
        self.entry_for(&path).await?;
        Ok(vec![EnvdFilesystemEvent {
            name: normalize_envd_path(&path),
            event_type: if recursive {
                EnvdFilesystemEventType::Write
            } else {
                EnvdFilesystemEventType::Create
            },
        }])
    }
    async fn watch_dir_stream(
        &self,
        path: String,
        recursive: bool,
    ) -> Result<EnvdFilesystemEventStream<BackendError>, BackendError> {
        self.entry_for(&path).await?;
        let root = self.resolve_path(&path)?;
        let base = normalize_envd_path(&path);
        let mut previous = host_watch_snapshot(&root, &base, recursive)?;
        let (sender, receiver) = mpsc::channel(32);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(25));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let current = match host_watch_snapshot(&root, &base, recursive) {
                    Ok(current) => current,
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                };
                for event in diff_host_watch_snapshots(&previous, &current) {
                    if sender.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
                previous = current;
            }
        });
        Ok(EnvdFilesystemEventStream { receiver })
    }
}
#[derive(Debug, Default)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct HostEnvdState {
    next_pid: u32,
    #[allow(missing_docs)]
    pub processes: BTreeMap<u32, HostEnvdProcessRecord>,
}
#[derive(Clone, Debug)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct HostEnvdProcessRecord {
    #[allow(missing_docs)]
    pub tag: Option<String>,
    #[allow(missing_docs)]
    pub cmd: String,
    #[allow(missing_docs)]
    pub args: Vec<String>,
    #[allow(missing_docs)]
    pub envs: BTreeMap<String, String>,
    #[allow(missing_docs)]
    pub cwd: Option<String>,
    #[allow(missing_docs)]
    pub output: EnvdProcessOutput,
    #[allow(missing_docs)]
    pub stdin: Option<mpsc::Sender<Vec<u8>>>,
    child: Option<Arc<Mutex<Child>>>,
}
#[derive(Clone, Copy)]
enum ProcessOutputKind {
    Stdout,
    Stderr,
}
fn process_log_fields(
    pid: u32,
    stream: &'static str,
    record: &HostEnvdProcessRecord,
) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::from([
        ("pid".to_owned(), pid.to_string()),
        ("stream".to_owned(), stream.to_owned()),
        ("cmd".to_owned(), record.cmd.clone()),
        ("status".to_owned(), record.output.status.clone()),
    ]);
    if let Some(tag) = &record.tag {
        fields.insert("tag".to_owned(), tag.clone());
    }
    fields
}
fn spawn_host_output_reader<R>(
    adapter: HostEnvdAdapter,
    pid: u32,
    mut reader: R,
    kind: ProcessOutputKind,
    sender: mpsc::Sender<Result<EnvdProcessStreamEvent, BackendError>>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buffer = [0_u8; 8192];
        loop {
            let bytes_read = match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(bytes_read) => bytes_read,
                Err(error) => {
                    let _ = sender
                        .send(Err(BackendError::Runtime(format!(
                            "read process output: {error}"
                        ))))
                        .await;
                    break;
                }
            };
            let bytes = buffer[..bytes_read].to_vec();
            adapter.append_process_output(pid, kind, &bytes).await;
            let event = match kind {
                ProcessOutputKind::Stdout => EnvdProcessStreamEvent::Stdout(bytes),
                ProcessOutputKind::Stderr => EnvdProcessStreamEvent::Stderr(bytes),
            };
            if sender.send(Ok(event)).await.is_err() {
                break;
            }
        }
    });
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct HostWatchEntry {
    len: u64,
    is_dir: bool,
    modified_ns: u128,
}
fn host_watch_snapshot(
    root: &Path,
    base: &str,
    recursive: bool,
) -> Result<BTreeMap<String, HostWatchEntry>, BackendError> {
    let mut entries = BTreeMap::new();
    collect_host_watch_entries(root, base, recursive, &mut entries)?;
    Ok(entries)
}
fn collect_host_watch_entries(
    root: &Path,
    base: &str,
    recursive: bool,
    entries: &mut BTreeMap<String, HostWatchEntry>,
) -> Result<(), BackendError> {
    for entry in std::fs::read_dir(root)
        .map_err(|error| BackendError::Runtime(format!("watch read dir `{base}`: {error}")))?
    {
        let entry =
            entry.map_err(|error| BackendError::Runtime(format!("watch read entry: {error}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let envd_path = join_envd_path(base, &name);
        let metadata = entry
            .metadata()
            .map_err(|error| BackendError::Runtime(format!("watch stat `{envd_path}`: {error}")))?;
        let is_dir = metadata.is_dir();
        entries.insert(
            envd_path.clone(),
            HostWatchEntry {
                len: metadata.len(),
                is_dir,
                modified_ns: metadata_modified_ns(&metadata),
            },
        );
        if recursive && is_dir {
            collect_host_watch_entries(&entry.path(), &envd_path, recursive, entries)?;
        }
    }
    Ok(())
}
fn metadata_modified_ns(metadata: &std::fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos())
}
fn diff_host_watch_snapshots(
    previous: &BTreeMap<String, HostWatchEntry>,
    current: &BTreeMap<String, HostWatchEntry>,
) -> Vec<EnvdFilesystemEvent> {
    let mut events = Vec::new();
    for path in current.keys() {
        if !previous.contains_key(path) {
            events.push(EnvdFilesystemEvent {
                name: path.clone(),
                event_type: EnvdFilesystemEventType::Create,
            });
        }
    }
    for path in previous.keys() {
        if !current.contains_key(path) {
            events.push(EnvdFilesystemEvent {
                name: path.clone(),
                event_type: EnvdFilesystemEventType::Remove,
            });
        }
    }
    for (path, current_entry) in current {
        if previous
            .get(path)
            .is_some_and(|previous_entry| previous_entry != current_entry)
        {
            events.push(EnvdFilesystemEvent {
                name: path.clone(),
                event_type: EnvdFilesystemEventType::Write,
            });
        }
    }
    events
}
fn normalize_envd_path(path: &str) -> String {
    normalize_volume_path(path)
}
fn host_mcp_gateway_request(request: &EnvdProcessStartRequest) -> bool {
    request.cmd == "mcp-gateway"
        || request
            .args
            .iter()
            .any(|arg| arg.trim_start().starts_with("mcp-gateway --config"))
}
fn join_envd_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{name}", parent.trim_end_matches('/'))
    }
}
#[derive(Clone, PartialEq, Message)]
struct EnvdListRequestProto {}
#[derive(Clone, PartialEq, Message)]
struct EnvdListResponseProto {
    #[prost(message, repeated, tag = "1")]
    #[allow(missing_docs)]
    pub processes: Vec<EnvdProcessInfoProto>,
}
#[derive(Clone, PartialEq, Message)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct EnvdProcessInfoProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub config: Option<EnvdProcessConfigProto>,
    #[prost(uint32, tag = "2")]
    #[allow(missing_docs)]
    pub pid: u32,
    #[prost(string, optional, tag = "3")]
    #[allow(missing_docs)]
    pub tag: Option<String>,
}
#[derive(Clone, PartialEq, Message)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct EnvdProcessSelectorProto {
    #[prost(oneof = "envd_process_selector_proto::Selector", tags = "1, 2")]
    selector: Option<envd_process_selector_proto::Selector>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdSendInputRequestProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub process: Option<EnvdProcessSelectorProto>,
    #[prost(message, optional, tag = "2")]
    input: Option<EnvdProcessInputProto>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdProcessInputProto {
    #[prost(oneof = "envd_process_input_proto::Input", tags = "1, 2")]
    input: Option<envd_process_input_proto::Input>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdSendInputResponseProto {}
#[derive(Clone, PartialEq, Message)]
struct EnvdCloseStdinRequestProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub process: Option<EnvdProcessSelectorProto>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdCloseStdinResponseProto {}
#[derive(Clone, PartialEq, Message)]
struct EnvdSendSignalRequestProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub process: Option<EnvdProcessSelectorProto>,
    #[prost(enumeration = "EnvdSignalProto", tag = "2")]
    signal: i32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, prost::Enumeration)]
#[repr(i32)]
enum EnvdSignalProto {
    Unspecified = 0,
    Sigterm = 15,
    Sigkill = 9,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdSendSignalResponseProto {}
#[derive(Clone, PartialEq, Message)]
struct EnvdConnectRequestProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub process: Option<EnvdProcessSelectorProto>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdConnectResponseProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub event: Option<EnvdProcessEventProto>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdUpdateRequestProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub process: Option<EnvdProcessSelectorProto>,
    #[prost(message, optional, tag = "2")]
    #[allow(missing_docs)]
    pub pty: Option<EnvdPtyProto>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdUpdateResponseProto {}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemMoveRequestProto {
    #[prost(string, tag = "1")]
    source: String,
    #[prost(string, tag = "2")]
    destination: String,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemMoveResponseProto {
    #[prost(message, optional, tag = "1")]
    entry: Option<EnvdFilesystemEntryInfoProto>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemMakeDirRequestProto {
    #[prost(string, tag = "1")]
    #[allow(missing_docs)]
    pub path: String,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemMakeDirResponseProto {
    #[prost(message, optional, tag = "1")]
    entry: Option<EnvdFilesystemEntryInfoProto>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemRemoveRequestProto {
    #[prost(string, tag = "1")]
    #[allow(missing_docs)]
    pub path: String,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemRemoveResponseProto {}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemStatRequestProto {
    #[prost(string, tag = "1")]
    #[allow(missing_docs)]
    pub path: String,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemStatResponseProto {
    #[prost(message, optional, tag = "1")]
    entry: Option<EnvdFilesystemEntryInfoProto>,
}
#[derive(Clone, PartialEq, Message)]
pub(crate) struct EnvdFilesystemEntryInfoProto {
    #[prost(string, tag = "1")]
    #[allow(missing_docs)]
    pub name: String,
    #[prost(enumeration = "EnvdFilesystemFileTypeProto", tag = "2")]
    #[allow(missing_docs)]
    pub file_type: i32,
    #[prost(string, tag = "3")]
    #[allow(missing_docs)]
    pub path: String,
    #[prost(int64, tag = "4")]
    #[allow(missing_docs)]
    pub size: i64,
    #[prost(uint32, tag = "5")]
    #[allow(missing_docs)]
    pub mode: u32,
    #[prost(string, tag = "6")]
    #[allow(missing_docs)]
    pub permissions: String,
    #[prost(string, tag = "7")]
    #[allow(missing_docs)]
    pub owner: String,
    #[prost(string, tag = "8")]
    #[allow(missing_docs)]
    pub group: String,
    #[prost(string, optional, tag = "10")]
    #[allow(missing_docs)]
    pub symlink_target: Option<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, prost::Enumeration)]
#[repr(i32)]
enum EnvdFilesystemFileTypeProto {
    Unspecified = 0,
    File = 1,
    Directory = 2,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemListDirRequestProto {
    #[prost(string, tag = "1")]
    #[allow(missing_docs)]
    pub path: String,
    #[prost(uint32, tag = "2")]
    depth: u32,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemListDirResponseProto {
    #[prost(message, repeated, tag = "1")]
    pub(crate) entries: Vec<EnvdFilesystemEntryInfoProto>,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemWatchDirRequestProto {
    #[prost(string, tag = "1")]
    #[allow(missing_docs)]
    pub path: String,
    #[prost(bool, tag = "2")]
    #[allow(missing_docs)]
    pub recursive: bool,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdFilesystemWatchDirResponseProto {
    #[prost(
        oneof = "envd_filesystem_watch_dir_response_proto::Event",
        tags = "1, 2, 3"
    )]
    #[allow(missing_docs)]
    pub event: Option<envd_filesystem_watch_dir_response_proto::Event>,
}
#[derive(Clone, PartialEq, Message)]
pub(crate) struct EnvdFilesystemStartEventProto {}
#[derive(Clone, PartialEq, Message)]
pub(crate) struct EnvdFilesystemEventProto {
    #[prost(string, tag = "1")]
    #[allow(missing_docs)]
    pub name: String,
    #[prost(enumeration = "EnvdFilesystemEventTypeProto", tag = "2")]
    #[allow(missing_docs)]
    pub event_type: i32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, prost::Enumeration)]
#[repr(i32)]
enum EnvdFilesystemEventTypeProto {
    Unspecified = 0,
    Create = 1,
    Write = 2,
    Remove = 3,
    Rename = 4,
    Chmod = 5,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdStartRequestProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub process: Option<EnvdProcessConfigProto>,
    #[prost(message, optional, tag = "2")]
    #[allow(missing_docs)]
    pub pty: Option<EnvdPtyProto>,
    #[prost(string, optional, tag = "3")]
    #[allow(missing_docs)]
    pub tag: Option<String>,
    #[prost(bool, optional, tag = "4")]
    #[allow(missing_docs)]
    pub stdin: Option<bool>,
}
#[derive(Clone, PartialEq, Message)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct EnvdProcessConfigProto {
    #[prost(string, tag = "1")]
    #[allow(missing_docs)]
    pub cmd: String,
    #[prost(string, repeated, tag = "2")]
    #[allow(missing_docs)]
    pub args: Vec<String>,
    #[prost(map = "string, string", tag = "3")]
    #[allow(missing_docs)]
    pub envs: HashMap<String, String>,
    #[prost(string, optional, tag = "4")]
    #[allow(missing_docs)]
    pub cwd: Option<String>,
}
#[derive(Clone, PartialEq, Message)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct EnvdPtyProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub size: Option<EnvdPtySizeProto>,
}
#[derive(Clone, PartialEq, Message)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct EnvdPtySizeProto {
    #[prost(uint32, tag = "1")]
    #[allow(missing_docs)]
    pub cols: u32,
    #[prost(uint32, tag = "2")]
    #[allow(missing_docs)]
    pub rows: u32,
}
#[derive(Clone, PartialEq, Message)]
struct EnvdStartResponseProto {
    #[prost(message, optional, tag = "1")]
    #[allow(missing_docs)]
    pub event: Option<EnvdProcessEventProto>,
}
#[derive(Clone, PartialEq, Message)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct EnvdProcessEventProto {
    #[prost(oneof = "envd_process_event_proto::Event", tags = "1, 2, 3, 4")]
    #[allow(missing_docs)]
    pub event: Option<envd_process_event_proto::Event>,
}
#[derive(Clone, PartialEq, Message)]
pub(crate) struct EnvdStartEventProto {
    #[prost(uint32, tag = "1")]
    #[allow(missing_docs)]
    pub pid: u32,
}
#[derive(Clone, PartialEq, Message)]
pub(crate) struct EnvdDataEventProto {
    #[prost(oneof = "envd_data_event_proto::Output", tags = "1, 2, 3")]
    #[allow(missing_docs)]
    pub output: Option<envd_data_event_proto::Output>,
}
#[derive(Clone, PartialEq, Message)]
pub(crate) struct EnvdEndEventProto {
    #[prost(sint32, tag = "1")]
    #[allow(missing_docs)]
    pub exit_code: i32,
    #[prost(bool, tag = "2")]
    #[allow(missing_docs)]
    pub exited: bool,
    #[prost(string, tag = "3")]
    #[allow(missing_docs)]
    pub status: String,
    #[prost(string, optional, tag = "4")]
    #[allow(missing_docs)]
    pub error: Option<String>,
}
async fn handle_envd_process_request<A>(
    adapter: A,
    request: Request<Incoming>,
) -> Response<HttpBody>
where
    A: EnvdProcessAdapter<Error = BackendError> + EnvdFilesystemAdapter<Error = BackendError>,
{
    let rpc_encoding = EnvdRpcEncoding::from_headers(request.headers());
    if request.method() == Method::GET && request.uri().path() == "/health" {
        return Response::builder()
            .status(StatusCode::OK)
            .body(full_body(b"ok".to_vec()))
            .expect("static envd health response is valid");
    }
    if request.method() != Method::POST {
        if request.method() == Method::GET && request.uri().path() == "/files" {
            return handle_envd_files_read(adapter, request).await;
        }
        return connect_error_to_http(
            StatusCode::METHOD_NOT_ALLOWED,
            "envd process method not allowed",
        );
    }
    let path = request.uri().path().to_owned();
    if path == "/files" {
        return handle_envd_files_write(adapter, request).await;
    }
    let body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to read envd request body: {error}"),
            );
        }
    };
    if path == "/process.Process/List" {
        return handle_envd_process_list(adapter, &body, rpc_encoding).await;
    }
    if path == "/process.Process/SendInput" {
        return handle_envd_process_send_input(adapter, &body, rpc_encoding).await;
    }
    if path == "/process.Process/CloseStdin" {
        return handle_envd_process_close_stdin(adapter, &body, rpc_encoding).await;
    }
    if path == "/process.Process/SendSignal" {
        return handle_envd_process_send_signal(adapter, &body, rpc_encoding).await;
    }
    if path == "/process.Process/Connect" {
        return handle_envd_process_connect(adapter, &body, rpc_encoding).await;
    }
    if path == "/process.Process/Update" {
        return handle_envd_process_update(adapter, &body, rpc_encoding).await;
    }
    if path == "/filesystem.Filesystem/ListDir" {
        return handle_envd_filesystem_list_dir(adapter, &body, rpc_encoding).await;
    }
    if path == "/filesystem.Filesystem/MakeDir" {
        return handle_envd_filesystem_make_dir(adapter, &body, rpc_encoding).await;
    }
    if path == "/filesystem.Filesystem/Move" {
        return handle_envd_filesystem_move(adapter, &body, rpc_encoding).await;
    }
    if path == "/filesystem.Filesystem/Remove" {
        return handle_envd_filesystem_remove(adapter, &body, rpc_encoding).await;
    }
    if path == "/filesystem.Filesystem/Stat" {
        return handle_envd_filesystem_stat(adapter, &body, rpc_encoding).await;
    }
    if path == "/filesystem.Filesystem/WatchDir" {
        return handle_envd_filesystem_watch_dir(adapter, &body, rpc_encoding).await;
    }
    if path != "/process.Process/Start" {
        return connect_error_to_http(StatusCode::NOT_FOUND, "envd route not found");
    }
    let request = match decode_envd_start_request(&body, rpc_encoding) {
        Ok(request) => request,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let stream = match adapter.start_process_stream(request).await {
        Ok(stream) => stream,
        Err(error) => {
            return connect_error_to_http(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, rpc_encoding.streaming_content_type())
        .body(envd_process_stream_body(
            stream,
            ProcessStreamEncoding::Start,
            rpc_encoding,
        ))
        .expect("static envd response is valid")
}
async fn handle_envd_process_request_authenticated<A>(
    adapter: A,
    access_token: Option<&str>,
    request: Request<Incoming>,
) -> Response<HttpBody>
where
    A: EnvdProcessAdapter<Error = BackendError> + EnvdFilesystemAdapter<Error = BackendError>,
{
    if let Some(access_token) = access_token
        && !header_matches(request.headers().get("x-access-token"), access_token)
    {
        return auth_error_to_http("missing or invalid x-access-token");
    }
    handle_envd_process_request(adapter, request).await
}
async fn handle_envd_process_connect<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdProcessAdapter<Error = BackendError>,
{
    let selector = match decode_envd_connect_selector(body, encoding) {
        Ok(selector) => selector,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    match adapter.connect_process(selector).await {
        Ok(output) => Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, encoding.streaming_content_type())
            .body(full_body(encode_envd_process_connect_output(
                &output, encoding,
            )))
            .expect("static envd response is valid"),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
async fn handle_envd_process_update<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdProcessAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdUpdateRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode process UpdateRequest: {error}"),
            );
        }
    };
    let selector = match decode_envd_selector(request.process) {
        Ok(selector) => selector,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let pty = request.pty.and_then(|pty| {
        pty.size.map(|size| EnvdPtySize {
            cols: size.cols,
            rows: size.rows,
        })
    });
    match adapter.update_process_pty(selector, pty).await {
        Ok(()) => encode_unary_proto_response(&EnvdUpdateResponseProto {}, encoding),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
async fn handle_envd_files_read<A>(adapter: A, request: Request<Incoming>) -> Response<HttpBody>
where
    A: EnvdFilesystemAdapter<Error = BackendError>,
{
    let Some(path) = query_param(request.uri().query(), "path") else {
        return connect_error_to_http(StatusCode::BAD_REQUEST, "missing file path");
    };
    match adapter.read_file(path).await {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(full_body(bytes))
            .expect("static envd file response is valid"),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
async fn handle_envd_files_write<A>(adapter: A, request: Request<Incoming>) -> Response<HttpBody>
where
    A: EnvdFilesystemAdapter<Error = BackendError>,
{
    let path = query_param(request.uri().query(), "path");
    let content_type = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let is_gzip = request
        .headers()
        .get(CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("gzip"));
    let body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes().to_vec(),
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to read file upload body: {error}"),
            );
        }
    };
    let writes = if let Some(boundary) = content_type
        .as_deref()
        .and_then(multipart_boundary_from_content_type)
    {
        match parse_multipart_files(&body, &boundary) {
            Ok(files) => files,
            Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
        }
    } else {
        let Some(path) = path else {
            return connect_error_to_http(StatusCode::BAD_REQUEST, "missing file path");
        };
        let data = if is_gzip {
            match gunzip_bytes(&body) {
                Ok(data) => data,
                Err(error) => {
                    return connect_error_to_http(StatusCode::BAD_REQUEST, &error);
                }
            }
        } else {
            body
        };
        vec![(path, data)]
    };
    let mut infos = Vec::with_capacity(writes.len());
    for (path, data) in writes {
        match adapter.write_file(path, data).await {
            Ok(info) => infos.push(info),
            Err(error) => return backend_error_to_connect_http(&error),
        }
    }
    let body = serde_json::to_vec(&infos).expect("file write response serializes");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/json")
        .body(full_body(body))
        .expect("static envd file response is valid")
}
async fn handle_envd_process_list<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdProcessAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    if let Err(error) = EnvdListRequestProto::decode(body.as_slice()) {
        return connect_error_to_http(
            StatusCode::BAD_REQUEST,
            &format!("failed to decode process ListRequest: {error}"),
        );
    }
    let processes = match adapter.list_processes().await {
        Ok(processes) => processes,
        Err(error) => {
            return connect_error_to_http(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
        }
    };
    let body = encode_envd_process_list(&processes);
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, encoding.unary_content_type())
        .body(full_body(encoding.encode_unary_response(&body)))
        .expect("static envd list response is valid")
}
async fn handle_envd_process_send_input<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdProcessAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdSendInputRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode process SendInputRequest: {error}"),
            );
        }
    };
    let selector = match decode_envd_selector(request.process) {
        Ok(selector) => selector,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let input = match decode_envd_input(request.input) {
        Ok(input) => input,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    if let Err(error) = adapter.send_process_input(selector, input).await {
        return connect_error_to_http(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    encode_unary_proto_response(&EnvdSendInputResponseProto {}, encoding)
}
async fn handle_envd_process_close_stdin<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdProcessAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdCloseStdinRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode process CloseStdinRequest: {error}"),
            );
        }
    };
    let selector = match decode_envd_selector(request.process) {
        Ok(selector) => selector,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    if let Err(error) = adapter.close_process_stdin(selector).await {
        return connect_error_to_http(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    encode_unary_proto_response(&EnvdCloseStdinResponseProto {}, encoding)
}
async fn handle_envd_process_send_signal<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdProcessAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdSendSignalRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode process SendSignalRequest: {error}"),
            );
        }
    };
    let selector = match decode_envd_selector(request.process) {
        Ok(selector) => selector,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    if let Err(error) = adapter
        .signal_process(selector, decode_envd_signal(request.signal))
        .await
    {
        return connect_error_to_http(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    encode_unary_proto_response(&EnvdSendSignalResponseProto {}, encoding)
}
async fn handle_envd_filesystem_list_dir<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdFilesystemAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdFilesystemListDirRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode filesystem ListDirRequest: {error}"),
            );
        }
    };
    match adapter.list_dir(request.path, request.depth).await {
        Ok(entries) => encode_unary_proto_response(
            &EnvdFilesystemListDirResponseProto {
                entries: entries.iter().map(encode_envd_filesystem_entry).collect(),
            },
            encoding,
        ),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
async fn handle_envd_filesystem_make_dir<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdFilesystemAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdFilesystemMakeDirRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode filesystem MakeDirRequest: {error}"),
            );
        }
    };
    match adapter.make_dir(request.path).await {
        Ok(entry) => encode_unary_proto_response(
            &EnvdFilesystemMakeDirResponseProto {
                entry: Some(encode_envd_filesystem_entry(&entry)),
            },
            encoding,
        ),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
async fn handle_envd_filesystem_move<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdFilesystemAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdFilesystemMoveRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode filesystem MoveRequest: {error}"),
            );
        }
    };
    match adapter
        .move_entry(request.source, request.destination)
        .await
    {
        Ok(entry) => encode_unary_proto_response(
            &EnvdFilesystemMoveResponseProto {
                entry: Some(encode_envd_filesystem_entry(&entry)),
            },
            encoding,
        ),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
async fn handle_envd_filesystem_remove<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdFilesystemAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdFilesystemRemoveRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode filesystem RemoveRequest: {error}"),
            );
        }
    };
    match adapter.remove_entry(request.path).await {
        Ok(()) => encode_unary_proto_response(&EnvdFilesystemRemoveResponseProto {}, encoding),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
async fn handle_envd_filesystem_stat<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdFilesystemAdapter<Error = BackendError>,
{
    let body = match encoding.unary_request_body(body) {
        Ok(body) => body,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    let request = match EnvdFilesystemStatRequestProto::decode(body.as_slice()) {
        Ok(request) => request,
        Err(error) => {
            return connect_error_to_http(
                StatusCode::BAD_REQUEST,
                &format!("failed to decode filesystem StatRequest: {error}"),
            );
        }
    };
    match adapter.stat_entry(request.path).await {
        Ok(entry) => encode_unary_proto_response(
            &EnvdFilesystemStatResponseProto {
                entry: Some(encode_envd_filesystem_entry(&entry)),
            },
            encoding,
        ),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
async fn handle_envd_filesystem_watch_dir<A>(
    adapter: A,
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Response<HttpBody>
where
    A: EnvdFilesystemAdapter<Error = BackendError>,
{
    let request = match decode_envd_filesystem_watch_dir_request(body, encoding) {
        Ok(request) => request,
        Err(error) => return connect_error_to_http(StatusCode::BAD_REQUEST, &error),
    };
    match adapter
        .watch_dir_stream(request.path, request.recursive)
        .await
    {
        Ok(stream) => Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, encoding.streaming_content_type())
            .body(envd_filesystem_watch_stream_body(stream, encoding))
            .expect("static envd filesystem response is valid"),
        Err(error) => backend_error_to_connect_http(&error),
    }
}
fn decode_envd_filesystem_watch_dir_request(
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Result<EnvdFilesystemWatchDirRequestProto, String> {
    let body = encoding.framed_request_body(body)?;
    let envelopes = decode_connect_envelopes(body.as_slice())?;
    let envelope = envelopes
        .first()
        .ok_or_else(|| "missing Connect request envelope".to_owned())?;
    EnvdFilesystemWatchDirRequestProto::decode(envelope.data.as_slice())
        .map_err(|error| format!("failed to decode filesystem WatchDirRequest: {error}"))
}
fn query_param(query: Option<&str>, key: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then(|| percent_decode_query_value(value))
    })
}
fn percent_decode_query_value(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'+' {
            output.push(b' ');
            index += 1;
            continue;
        }
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (
                query_hex_value(bytes[index + 1]),
                query_hex_value(bytes[index + 2]),
            )
        {
            output.push(high * 16 + low);
            index += 3;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}
const fn query_hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
fn multipart_boundary_from_content_type(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    let media_type = parts.next()?.trim();
    if !media_type.eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }
    parts.find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name.eq_ignore_ascii_case("boundary")).then(|| value.trim_matches('"').to_owned())
    })
}
fn parse_multipart_files(body: &[u8], boundary: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let text = String::from_utf8_lossy(body);
    let delimiter = format!("--{boundary}");
    let mut files = Vec::new();
    for part in text.split(&delimiter).skip(1) {
        let part = part.trim_start_matches("\r\n");
        if part.starts_with("--") {
            break;
        }
        let Some((headers, data)) = part.split_once("\r\n\r\n") else {
            continue;
        };
        let Some(path) = headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .and_then(filename_from_content_disposition)
        else {
            continue;
        };
        let data = data
            .strip_suffix("\r\n")
            .unwrap_or(data)
            .as_bytes()
            .to_vec();
        files.push((path, data));
    }
    if files.is_empty() {
        return Err("multipart upload did not include any file parts".to_owned());
    }
    Ok(files)
}
fn filename_from_content_disposition(header: &str) -> Option<String> {
    header.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name.eq_ignore_ascii_case("filename")).then(|| value.trim_matches('"').to_owned())
    })
}
fn gunzip_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .read_to_end(&mut output)
        .map_err(|error| format!("failed to decode gzip upload: {error}"))?;
    Ok(output)
}
fn encode_envd_filesystem_entry(entry: &EnvdFilesystemEntry) -> EnvdFilesystemEntryInfoProto {
    EnvdFilesystemEntryInfoProto {
        name: entry.name.clone(),
        file_type: match entry.file_type {
            EnvdFilesystemFileType::File => EnvdFilesystemFileTypeProto::File as i32,
            EnvdFilesystemFileType::Directory => EnvdFilesystemFileTypeProto::Directory as i32,
        },
        path: entry.path.clone(),
        size: entry.size,
        mode: entry.mode,
        permissions: entry.permissions.clone(),
        owner: entry.owner.clone(),
        group: entry.group.clone(),
        symlink_target: entry.symlink_target.clone(),
    }
}
fn encode_filesystem_watch_response(
    event: envd_filesystem_watch_dir_response_proto::Event,
) -> Vec<u8> {
    let mut data = Vec::new();
    EnvdFilesystemWatchDirResponseProto { event: Some(event) }
        .encode(&mut data)
        .expect("envd filesystem watch response protobuf encodes");
    encode_connect_envelope(0, &data)
}
fn encode_filesystem_watch_start_response() -> Vec<u8> {
    encode_filesystem_watch_response(envd_filesystem_watch_dir_response_proto::Event::Start(
        EnvdFilesystemStartEventProto {},
    ))
}
fn encode_filesystem_watch_event_response(event: EnvdFilesystemEvent) -> Vec<u8> {
    encode_filesystem_watch_response(envd_filesystem_watch_dir_response_proto::Event::Filesystem(
        EnvdFilesystemEventProto {
            name: event.name,
            event_type: match event.event_type {
                EnvdFilesystemEventType::Create => EnvdFilesystemEventTypeProto::Create as i32,
                EnvdFilesystemEventType::Write => EnvdFilesystemEventTypeProto::Write as i32,
                EnvdFilesystemEventType::Remove => EnvdFilesystemEventTypeProto::Remove as i32,
                EnvdFilesystemEventType::Rename => EnvdFilesystemEventTypeProto::Rename as i32,
                EnvdFilesystemEventType::Chmod => EnvdFilesystemEventTypeProto::Chmod as i32,
            },
        },
    ))
}
fn envd_filesystem_watch_stream_body(
    mut stream: EnvdFilesystemEventStream<BackendError>,
    rpc_encoding: EnvdRpcEncoding,
) -> HttpBody {
    let (mut sender, body) = Channel::<Bytes, BoxError>::new(16);
    tokio::spawn(async move {
        if sender
            .send_data(Bytes::from(
                rpc_encoding.encode_stream_frame(&encode_filesystem_watch_start_response()),
            ))
            .await
            .is_err()
        {
            return;
        }
        while let Some(event) = stream.recv().await {
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    sender.abort(Box::new(error));
                    return;
                }
            };
            let body = encode_filesystem_watch_event_response(event);
            if sender
                .send_data(Bytes::from(rpc_encoding.encode_stream_frame(&body)))
                .await
                .is_err()
            {
                return;
            }
        }
        let _ = sender
            .send_data(Bytes::from(
                rpc_encoding.encode_stream_frame(&rpc_encoding.end_frame()),
            ))
            .await;
    });
    body.boxed()
}
fn backend_error_to_connect_http(error: &BackendError) -> Response<HttpBody> {
    let status = match error {
        BackendError::NotFound(_) => StatusCode::NOT_FOUND,
        BackendError::AlreadyExists(_) => StatusCode::CONFLICT,
        BackendError::Runtime(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    connect_error_to_http(status, &error.to_string())
}
fn decode_envd_selector(
    selector: Option<EnvdProcessSelectorProto>,
) -> Result<EnvdProcessSelector, String> {
    match selector.and_then(|selector| selector.selector) {
        Some(envd_process_selector_proto::Selector::Pid(pid)) => Ok(EnvdProcessSelector::Pid(pid)),
        Some(envd_process_selector_proto::Selector::Tag(tag)) => Ok(EnvdProcessSelector::Tag(tag)),
        None => Err("missing process selector".to_owned()),
    }
}
fn decode_envd_input(input: Option<EnvdProcessInputProto>) -> Result<EnvdProcessInput, String> {
    match input.and_then(|input| input.input) {
        Some(envd_process_input_proto::Input::Stdin(data)) => Ok(EnvdProcessInput::Stdin(data)),
        Some(envd_process_input_proto::Input::Pty(data)) => Ok(EnvdProcessInput::Pty(data)),
        None => Err("missing process input".to_owned()),
    }
}
fn decode_envd_signal(signal: i32) -> EnvdProcessSignal {
    match EnvdSignalProto::try_from(signal) {
        Ok(EnvdSignalProto::Unspecified) => EnvdProcessSignal::Unspecified,
        Ok(EnvdSignalProto::Sigterm) => EnvdProcessSignal::Sigterm,
        Ok(EnvdSignalProto::Sigkill) => EnvdProcessSignal::Sigkill,
        Err(_) => EnvdProcessSignal::Unknown(signal),
    }
}
fn decode_envd_start_request(
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Result<EnvdProcessStartRequest, String> {
    let body = encoding.framed_request_body(body)?;
    let envelopes = decode_connect_envelopes(body.as_slice())?;
    let envelope = envelopes
        .first()
        .ok_or_else(|| "missing Connect request envelope".to_owned())?;
    let request = EnvdStartRequestProto::decode(envelope.data.as_slice())
        .map_err(|error| format!("failed to decode process StartRequest: {error}"))?;
    let process = request
        .process
        .ok_or_else(|| "missing process config".to_owned())?;
    Ok(EnvdProcessStartRequest {
        cmd: process.cmd,
        args: process.args,
        envs: process.envs.into_iter().collect(),
        cwd: process.cwd,
        tag: request.tag,
        stdin: request.stdin,
        pty: request
            .pty
            .and_then(|pty| pty.size)
            .map(|size| EnvdPtySize {
                cols: size.cols,
                rows: size.rows,
            }),
    })
}
fn decode_envd_connect_selector(
    body: &[u8],
    encoding: EnvdRpcEncoding,
) -> Result<EnvdProcessSelector, String> {
    let body = encoding.framed_request_body(body)?;
    let envelopes = decode_connect_envelopes(body.as_slice())?;
    let envelope = envelopes
        .first()
        .ok_or_else(|| "missing Connect request envelope".to_owned())?;
    let request = EnvdConnectRequestProto::decode(envelope.data.as_slice())
        .map_err(|error| format!("failed to decode process ConnectRequest: {error}"))?;
    decode_envd_selector(request.process)
}
fn encode_envd_process_list(processes: &[EnvdProcessInfo]) -> Vec<u8> {
    let mut data = Vec::new();
    EnvdListResponseProto {
        processes: processes
            .iter()
            .map(|process| EnvdProcessInfoProto {
                config: Some(EnvdProcessConfigProto {
                    cmd: process.cmd.clone(),
                    args: process.args.clone(),
                    envs: process.envs.clone().into_iter().collect(),
                    cwd: process.cwd.clone(),
                }),
                pid: process.pid,
                tag: process.tag.clone(),
            })
            .collect(),
    }
    .encode(&mut data)
    .expect("envd process list response protobuf encodes");
    data
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnvdRpcEncoding {
    Connect,
    GrpcWebBinary,
    GrpcWebText,
}
impl EnvdRpcEncoding {
    fn from_headers(headers: &HeaderMap) -> Self {
        let media_type = headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        match media_type {
            Some(media) if media.eq_ignore_ascii_case("application/grpc-web+proto") => {
                Self::GrpcWebBinary
            }
            Some(media) if media.eq_ignore_ascii_case("application/grpc-web-text+proto") => {
                Self::GrpcWebText
            }
            _ => Self::Connect,
        }
    }
    const fn unary_content_type(self) -> &'static str {
        match self {
            Self::Connect => "application/proto",
            Self::GrpcWebBinary => "application/grpc-web+proto",
            Self::GrpcWebText => "application/grpc-web-text+proto",
        }
    }
    const fn streaming_content_type(self) -> &'static str {
        match self {
            Self::Connect => "application/connect+proto",
            Self::GrpcWebBinary => "application/grpc-web+proto",
            Self::GrpcWebText => "application/grpc-web-text+proto",
        }
    }
    fn unary_request_body(self, body: &[u8]) -> Result<Vec<u8>, String> {
        match self {
            Self::Connect => Ok(body.to_vec()),
            Self::GrpcWebBinary | Self::GrpcWebText => {
                let body = self.framed_request_body(body)?;
                let envelopes = decode_connect_envelopes(body.as_slice())?;
                let envelope = envelopes
                    .first()
                    .ok_or_else(|| "missing gRPC-web request frame".to_owned())?;
                if envelope.flags != 0 {
                    return Err(format!(
                        "unsupported gRPC-web request frame flags {}",
                        envelope.flags
                    ));
                }
                Ok(envelope.data.clone())
            }
        }
    }
    fn framed_request_body(self, body: &[u8]) -> Result<Vec<u8>, String> {
        match self {
            Self::Connect | Self::GrpcWebBinary => Ok(body.to_vec()),
            Self::GrpcWebText => BASE64_STANDARD
                .decode(body)
                .map_err(|error| format!("failed to decode gRPC-web-text body: {error}")),
        }
    }
    fn encode_unary_response(self, body: &[u8]) -> Vec<u8> {
        match self {
            Self::Connect => body.to_vec(),
            Self::GrpcWebBinary | Self::GrpcWebText => {
                let mut response = encode_connect_envelope(0, body);
                response.extend(self.end_frame());
                self.encode_response_bytes(&response)
            }
        }
    }
    fn encode_stream_frame(self, frame: &[u8]) -> Vec<u8> {
        self.encode_response_bytes(frame)
    }
    fn encode_response_bytes(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Connect | Self::GrpcWebBinary => bytes.to_vec(),
            Self::GrpcWebText => BASE64_STANDARD.encode(bytes).into_bytes(),
        }
    }
    fn end_frame(self) -> Vec<u8> {
        match self {
            Self::Connect => encode_connect_envelope(0b0000_0010, b"{}"),
            Self::GrpcWebBinary | Self::GrpcWebText => {
                encode_connect_envelope(0x80, b"grpc-status: 0\r\n")
            }
        }
    }
}
fn encode_unary_proto_response<T>(message: &T, encoding: EnvdRpcEncoding) -> Response<HttpBody>
where
    T: Message,
{
    let mut body = Vec::new();
    message
        .encode(&mut body)
        .expect("envd unary response protobuf encodes");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, encoding.unary_content_type())
        .body(full_body(encoding.encode_unary_response(&body)))
        .expect("static envd unary response is valid")
}
struct ConnectEnvelope {
    flags: u8,
    #[allow(missing_docs)]
    pub data: Vec<u8>,
}
fn decode_connect_envelopes(mut bytes: &[u8]) -> Result<Vec<ConnectEnvelope>, String> {
    let mut envelopes = Vec::new();
    while !bytes.is_empty() {
        if bytes.len() < 5 {
            return Err("truncated Connect envelope header".to_owned());
        }
        let flags = bytes[0];
        let len = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
        bytes = &bytes[5..];
        if bytes.len() < len {
            return Err("truncated Connect envelope body".to_owned());
        }
        envelopes.push(ConnectEnvelope {
            flags,
            data: bytes[..len].to_vec(),
        });
        bytes = &bytes[len..];
    }
    Ok(envelopes)
}
fn encode_envd_process_connect_output(
    output: &EnvdProcessOutput,
    rpc_encoding: EnvdRpcEncoding,
) -> Vec<u8> {
    let mut body = Vec::new();
    for event in process_output_events(output) {
        body.extend(encode_process_stream_event(
            &event,
            ProcessStreamEncoding::Connect,
        ));
    }
    body.extend(rpc_encoding.end_frame());
    body
}
#[derive(Clone, Copy)]
enum ProcessStreamEncoding {
    Start,
    Connect,
}
fn envd_process_stream_body(
    mut stream: EnvdProcessEventStream<BackendError>,
    encoding: ProcessStreamEncoding,
    rpc_encoding: EnvdRpcEncoding,
) -> HttpBody {
    let (mut sender, body) = Channel::<Bytes, BoxError>::new(16);
    tokio::spawn(async move {
        while let Some(event) = stream.recv().await {
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    sender.abort(Box::new(error));
                    return;
                }
            };
            let frame = Bytes::from(
                rpc_encoding.encode_stream_frame(&encode_process_stream_event(&event, encoding)),
            );
            if sender.send_data(frame).await.is_err() {
                return;
            }
        }
        let _ = sender
            .send_data(Bytes::from(
                rpc_encoding.encode_stream_frame(&rpc_encoding.end_frame()),
            ))
            .await;
    });
    body.boxed()
}
fn encode_process_stream_event(
    event: &EnvdProcessStreamEvent,
    encoding: ProcessStreamEncoding,
) -> Vec<u8> {
    let event = match event {
        EnvdProcessStreamEvent::Start { pid } => {
            envd_process_event_proto::Event::Start(EnvdStartEventProto { pid: *pid })
        }
        EnvdProcessStreamEvent::Stdout(bytes) => {
            envd_process_event_proto::Event::Data(EnvdDataEventProto {
                output: Some(envd_data_event_proto::Output::Stdout(bytes.clone())),
            })
        }
        EnvdProcessStreamEvent::Stderr(bytes) => {
            envd_process_event_proto::Event::Data(EnvdDataEventProto {
                output: Some(envd_data_event_proto::Output::Stderr(bytes.clone())),
            })
        }
        EnvdProcessStreamEvent::Pty(bytes) => {
            envd_process_event_proto::Event::Data(EnvdDataEventProto {
                output: Some(envd_data_event_proto::Output::Pty(bytes.clone())),
            })
        }
        EnvdProcessStreamEvent::End {
            exit_code,
            exited,
            status,
            error,
        } => envd_process_event_proto::Event::End(EnvdEndEventProto {
            exit_code: *exit_code,
            exited: *exited,
            status: status.clone(),
            error: error.clone(),
        }),
    };
    match encoding {
        ProcessStreamEncoding::Start => encode_start_response(event),
        ProcessStreamEncoding::Connect => encode_connect_response(event),
    }
}
fn encode_start_response(event: envd_process_event_proto::Event) -> Vec<u8> {
    let mut data = Vec::new();
    EnvdStartResponseProto {
        event: Some(EnvdProcessEventProto { event: Some(event) }),
    }
    .encode(&mut data)
    .expect("envd process response protobuf encodes");
    encode_connect_envelope(0, &data)
}
fn encode_connect_response(event: envd_process_event_proto::Event) -> Vec<u8> {
    let mut data = Vec::new();
    EnvdConnectResponseProto {
        event: Some(EnvdProcessEventProto { event: Some(event) }),
    }
    .encode(&mut data)
    .expect("envd process connect response protobuf encodes");
    encode_connect_envelope(0, &data)
}
fn encode_connect_envelope(flags: u8, data: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(5 + data.len());
    encoded.push(flags);
    encoded.extend_from_slice(
        &u32::try_from(data.len())
            .expect("Connect envelope fits in u32")
            .to_be_bytes(),
    );
    encoded.extend_from_slice(data);
    encoded
}
fn connect_error_to_http(status: StatusCode, message: &str) -> Response<HttpBody> {
    let code = match status {
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "already_exists",
        StatusCode::BAD_REQUEST => "invalid_argument",
        StatusCode::METHOD_NOT_ALLOWED => "unimplemented",
        _ => "internal",
    };
    let body = serde_json::to_vec(&BTreeMap::from([("code", code), ("message", message)]))
        .expect("string Connect error body serializes");
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(full_body(body))
        .expect("static Connect error response is valid")
}
