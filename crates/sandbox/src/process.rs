use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use tokio::sync::Mutex;

use crate::backend::BoxBackend;
use crate::capability::CapabilityName;
use crate::error::{Error, ProcessFailure, Result, RetryClass};
use crate::ids::{ProcessId, ProcessTag, SandboxId};
use crate::sandbox::unsupported;

pub type ProcessEventStream = Pin<Box<dyn Stream<Item = Result<ProcessEvent>> + Send + 'static>>;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    mode: CommandMode,
    cwd: Option<String>,
    env: BTreeMap<String, String>,
    user: Option<String>,
    stdin: Option<Bytes>,
    pty: Option<PtySize>,
    tag: Option<ProcessTag>,
    timeout: Option<Duration>,
}

impl Command {
    pub fn shell(command: impl Into<String>) -> Self {
        Self::new(CommandMode::Shell(command.into()))
    }

    pub fn argv(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::new(CommandMode::Argv {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        })
    }

    fn new(mode: CommandMode) -> Self {
        Self {
            mode,
            cwd: None,
            env: BTreeMap::new(),
            user: None,
            stdin: None,
            pty: None,
            tag: None,
            timeout: None,
        }
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    pub fn stdin(mut self, bytes: impl Into<Bytes>) -> Self {
        self.stdin = Some(bytes.into());
        self
    }

    pub fn pty(mut self, size: PtySize) -> Self {
        self.pty = Some(size);
        self
    }

    pub fn tag(mut self, tag: ProcessTag) -> Self {
        self.tag = Some(tag);
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub const fn mode(&self) -> &CommandMode {
        &self.mode
    }

    pub fn cwd_ref(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    pub fn env_ref(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    pub fn user_ref(&self) -> Option<&str> {
        self.user.as_deref()
    }

    pub fn stdin_ref(&self) -> Option<&Bytes> {
        self.stdin.as_ref()
    }

    pub const fn pty_ref(&self) -> Option<PtySize> {
        self.pty
    }

    pub fn tag_ref(&self) -> Option<&ProcessTag> {
        self.tag.as_ref()
    }

    pub const fn timeout_ref(&self) -> Option<Duration> {
        self.timeout
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandMode {
    Shell(String),
    Argv { program: String, args: Vec<String> },
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: CommandStatus,
    pub stdout: Bytes,
    pub stderr: Bytes,
}

impl CommandOutput {
    pub fn success(stdout: impl Into<Bytes>) -> Self {
        Self {
            status: CommandStatus::Exited(CommandExit::success()),
            stdout: stdout.into(),
            stderr: Bytes::new(),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandStatus {
    Exited(CommandExit),
    Signaled(Signal),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandExit {
    pub code: i32,
}

impl CommandExit {
    pub const fn success() -> Self {
        Self { code: 0 }
    }

    pub const fn nonzero(code: i32) -> Self {
        Self { code }
    }

    pub const fn is_success(self) -> bool {
        self.code == 0
    }
}

#[derive(Clone)]
pub struct Process {
    id: ProcessId,
    sandbox_id: SandboxId,
    backend: BoxBackend,
}

impl Process {
    pub(crate) fn new(id: ProcessId, sandbox_id: SandboxId, backend: BoxBackend) -> Self {
        Self {
            id,
            sandbox_id,
            backend,
        }
    }

    pub fn id(&self) -> ProcessId {
        self.id.clone()
    }

    pub async fn info(&self) -> Result<ProcessInfo> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("process info", CapabilityName::ProcessStart));
        };
        control
            .connect_process(&self.sandbox_id, ProcessSelector::Id(self.id.clone()))
            .await
    }

    #[allow(clippy::unused_async)]
    pub async fn next_event(&mut self) -> Option<Result<ProcessEvent>> {
        None
    }

    pub async fn send_input(&self, input: ProcessInput) -> Result<()> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("process stdin", CapabilityName::ProcessStdin));
        };
        control
            .send_process_input(
                &self.sandbox_id,
                ProcessSelector::Id(self.id.clone()),
                input,
            )
            .await
    }

    pub async fn close_stdin(&self) -> Result<()> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported(
                "close process stdin",
                CapabilityName::ProcessStdin,
            ));
        };
        control
            .close_process_stdin(&self.sandbox_id, ProcessSelector::Id(self.id.clone()))
            .await
    }

    pub async fn signal(&self, signal: Signal) -> Result<()> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("signal process", CapabilityName::ProcessSignal));
        };
        control
            .signal_process(
                &self.sandbox_id,
                ProcessSelector::Id(self.id.clone()),
                signal,
            )
            .await
    }

    pub async fn resize_pty(&self, size: PtySize) -> Result<()> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("resize pty", CapabilityName::ProcessPty));
        };
        control
            .resize_process_pty(&self.sandbox_id, ProcessSelector::Id(self.id.clone()), size)
            .await
    }

    pub async fn wait(self) -> Result<CommandOutput> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("wait process", CapabilityName::ProcessStart));
        };
        control
            .wait_process(&self.sandbox_id, ProcessSelector::Id(self.id))
            .await
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessInfo {
    pub id: ProcessId,
    pub tag: Option<ProcessTag>,
    pub status: ProcessStatus,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    Starting,
    Running,
    Exited(CommandExit),
    Signaled(Signal),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessSelector {
    Id(ProcessId),
    Tag(ProcessTag),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessEvent {
    Started(ProcessInfo),
    Stdout(Bytes),
    Stderr(Bytes),
    Exited(CommandExit),
    Signaled(Signal),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessInput {
    Bytes(Bytes),
    Eof,
}

impl From<Bytes> for ProcessInput {
    fn from(bytes: Bytes) -> Self {
        Self::Bytes(bytes)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Signal {
    #[default]
    Term,
    Kill,
    Interrupt,
    Number(i32),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl PtySize {
    pub const fn new(rows: u16, cols: u16) -> Self {
        Self { rows, cols }
    }
}

pub type Pty = PtySize;

#[derive(Clone)]
pub struct ProcessClient {
    backend: BoxBackend,
    sandbox_id: SandboxId,
}

impl ProcessClient {
    pub(crate) fn new(backend: BoxBackend, sandbox_id: SandboxId) -> Self {
        Self {
            backend,
            sandbox_id,
        }
    }

    pub async fn run(&self, command: Command) -> Result<CommandOutput> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("run process", CapabilityName::ProcessRun));
        };
        control.run_process(&self.sandbox_id, command).await
    }

    pub async fn start(&self, command: Command) -> Result<Process> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("start process", CapabilityName::ProcessStart));
        };
        let info = control.start_process(&self.sandbox_id, command).await?;
        Ok(Process::new(
            info.id,
            self.sandbox_id.clone(),
            self.backend.clone(),
        ))
    }

    pub async fn start_stream(&self, command: Command) -> Result<ProcessEventStream> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("stream process", CapabilityName::ProcessStream));
        };
        control
            .start_process_stream(&self.sandbox_id, command)
            .await
    }

    pub async fn list(&self) -> Result<Vec<ProcessInfo>> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("list processes", CapabilityName::ProcessStart));
        };
        control.list_processes(&self.sandbox_id).await
    }

    pub async fn connect(&self, selector: ProcessSelector) -> Result<Process> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("connect process", CapabilityName::ProcessStart));
        };
        let info = control.connect_process(&self.sandbox_id, selector).await?;
        Ok(Process::new(
            info.id,
            self.sandbox_id.clone(),
            self.backend.clone(),
        ))
    }

    pub async fn signal(&self, selector: ProcessSelector, signal: Signal) -> Result<()> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("signal process", CapabilityName::ProcessSignal));
        };
        control
            .signal_process(&self.sandbox_id, selector, signal)
            .await
    }

    pub async fn send_input(&self, selector: ProcessSelector, input: ProcessInput) -> Result<()> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("process stdin", CapabilityName::ProcessStdin));
        };
        control
            .send_process_input(&self.sandbox_id, selector, input)
            .await
    }

    pub async fn close_stdin(&self, selector: ProcessSelector) -> Result<()> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported(
                "close process stdin",
                CapabilityName::ProcessStdin,
            ));
        };
        control
            .close_process_stdin(&self.sandbox_id, selector)
            .await
    }

    pub async fn resize_pty(&self, selector: ProcessSelector, size: PtySize) -> Result<()> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported("resize pty", CapabilityName::ProcessPty));
        };
        control
            .resize_process_pty(&self.sandbox_id, selector, size)
            .await
    }

    pub async fn shell(&self) -> Result<Shell> {
        self.shell_with(ShellOpts::default()).await
    }

    pub async fn shell_with(&self, opts: ShellOpts) -> Result<Shell> {
        let Some(control) = self.backend.processes() else {
            return Err(unsupported(
                "open retained shell",
                CapabilityName::ProcessStream,
            ));
        };
        let tag = opts.tag()?;
        let stream = control
            .start_process_stream(&self.sandbox_id, opts.start_command(tag.clone()))
            .await?;
        Ok(Shell::new(
            self.backend.clone(),
            self.sandbox_id.clone(),
            tag,
            stream,
        ))
    }
}

impl From<(Arc<dyn crate::backend::SandboxBackend>, SandboxId)> for ProcessClient {
    fn from((backend, sandbox_id): (Arc<dyn crate::backend::SandboxBackend>, SandboxId)) -> Self {
        Self::new(backend, sandbox_id)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellOpts {
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    user: Option<String>,
    cwd: Option<String>,
    tag_prefix: String,
}

impl ShellOpts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn program(mut self, program: impl Into<String>) -> Self {
        self.program = program.into();
        self
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn tag_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tag_prefix = prefix.into();
        self
    }

    fn tag(&self) -> Result<ProcessTag> {
        let uuid = uuid::Uuid::new_v4().simple().to_string();
        ProcessTag::new(format!("{}-{uuid}", self.tag_prefix)).map_err(|err| {
            Error::ProcessFailure(ProcessFailure {
                operation: "open retained shell",
                sandbox_id: None,
                process_id: None,
                reason: format!("invalid retained shell tag: {err}"),
                retry: RetryClass::NotRetryable,
            })
        })
    }

    fn start_command(&self, tag: ProcessTag) -> Command {
        let mut command = Command::argv(self.program.clone(), self.args.clone()).tag(tag);
        for (key, value) in &self.env {
            command = command.env(key.clone(), value.clone());
        }
        if let Some(user) = &self.user {
            command = command.user(user.clone());
        }
        if let Some(cwd) = &self.cwd {
            command = command.cwd(cwd.clone());
        }
        command
    }
}

impl Default for ShellOpts {
    fn default() -> Self {
        Self {
            program: "/bin/bash".to_owned(),
            args: vec!["-l".to_owned()],
            env: BTreeMap::new(),
            user: None,
            cwd: None,
            tag_prefix: "fk-shell".to_owned(),
        }
    }
}

#[derive(Clone)]
pub struct Shell {
    inner: Arc<ShellInner>,
}

struct ShellInner {
    backend: BoxBackend,
    sandbox_id: SandboxId,
    tag: ProcessTag,
    stream: Mutex<ProcessEventStream>,
    closed: AtomicBool,
}

impl Shell {
    fn new(
        backend: BoxBackend,
        sandbox_id: SandboxId,
        tag: ProcessTag,
        stream: ProcessEventStream,
    ) -> Self {
        Self {
            inner: Arc::new(ShellInner {
                backend,
                sandbox_id,
                tag,
                stream: Mutex::new(stream),
                closed: AtomicBool::new(false),
            }),
        }
    }

    pub async fn run(&self, command: Command) -> Result<CommandOutput> {
        let timeout = command.timeout_ref();
        let plan = ShellCommandPlan::new(&command)?;
        if let Some(timeout) = timeout {
            return tokio::time::timeout(timeout, self.run_plan(plan))
                .await
                .map_err(|_| {
                    Error::ProcessFailure(ProcessFailure {
                        operation: "run retained shell command",
                        sandbox_id: Some(self.inner.sandbox_id.clone()),
                        process_id: None,
                        reason: "retained shell dispatch timed out".to_owned(),
                        retry: RetryClass::Unknown,
                    })
                })?;
        }
        self.run_plan(plan).await
    }

    #[allow(clippy::unused_async)]
    pub async fn run_stream(&self, _command: Command) -> Result<ProcessEventStream> {
        Err(process_failure(
            "stream retained shell command",
            Some(self.inner.sandbox_id.clone()),
            "retained shell streaming is not implemented; use Shell::run for exact captured output",
            RetryClass::NotRetryable,
        ))
    }

    pub async fn send_input(&self, input: ProcessInput) -> Result<()> {
        self.send_to_shell(input).await
    }

    pub async fn cancel(&self) -> Result<()> {
        let Some(control) = self.inner.backend.processes() else {
            return Err(unsupported(
                "interrupt retained shell",
                CapabilityName::ProcessSignal,
            ));
        };
        control
            .signal_process(
                &self.inner.sandbox_id,
                ProcessSelector::Tag(self.inner.tag.clone()),
                Signal::Interrupt,
            )
            .await
    }

    pub async fn close(self) -> Result<()> {
        self.close_inner().await
    }

    async fn run_plan(&self, plan: ShellCommandPlan) -> Result<CommandOutput> {
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        self.run_plan_with_nonce(plan, nonce).await
    }

    async fn run_plan_with_nonce(
        &self,
        plan: ShellCommandPlan,
        nonce: String,
    ) -> Result<CommandOutput> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(process_failure(
                "run retained shell command",
                Some(self.inner.sandbox_id.clone()),
                "retained shell is closed",
                RetryClass::NotRetryable,
            ));
        }
        let script = plan.dispatch_script(&nonce);
        let mut stream = self.inner.stream.lock().await;
        self.send_to_shell(ProcessInput::Bytes(Bytes::from(script)))
            .await?;
        let mut capture = ShellDispatchCapture::new(nonce);
        while let Some(event) = stream.next().await {
            match event? {
                ProcessEvent::Started(_) => {}
                ProcessEvent::Stdout(bytes) => {
                    if let Some(output) = capture.push_stdout(&bytes)? {
                        return Ok(output);
                    }
                }
                ProcessEvent::Stderr(bytes) => capture.push_protocol_stderr(&bytes),
                ProcessEvent::Exited(exit) => {
                    return Err(process_failure(
                        "run retained shell command",
                        Some(self.inner.sandbox_id.clone()),
                        format!(
                            "retained shell exited before dispatch completed: {}",
                            exit.code
                        ),
                        RetryClass::NotRetryable,
                    ));
                }
                ProcessEvent::Signaled(signal) => {
                    return Err(process_failure(
                        "run retained shell command",
                        Some(self.inner.sandbox_id.clone()),
                        format!(
                            "retained shell was signaled before dispatch completed: {signal:?}"
                        ),
                        RetryClass::NotRetryable,
                    ));
                }
            }
        }
        Err(process_failure(
            "run retained shell command",
            Some(self.inner.sandbox_id.clone()),
            "retained shell event stream ended before dispatch completed",
            RetryClass::NotRetryable,
        ))
    }

    async fn send_to_shell(&self, input: ProcessInput) -> Result<()> {
        let Some(control) = self.inner.backend.processes() else {
            return Err(unsupported(
                "send retained shell input",
                CapabilityName::ProcessStdin,
            ));
        };
        control
            .send_process_input(
                &self.inner.sandbox_id,
                ProcessSelector::Tag(self.inner.tag.clone()),
                input,
            )
            .await
    }

    async fn close_inner(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let Some(control) = self.inner.backend.processes() else {
            return Err(unsupported(
                "close retained shell",
                CapabilityName::ProcessSignal,
            ));
        };
        control
            .signal_process(
                &self.inner.sandbox_id,
                ProcessSelector::Tag(self.inner.tag.clone()),
                Signal::Kill,
            )
            .await
    }
}

#[derive(Clone)]
pub struct ShellPool {
    shells: Arc<[Shell]>,
    next: Arc<std::sync::atomic::AtomicUsize>,
}

impl ShellPool {
    pub fn new(shells: impl Into<Vec<Shell>>) -> Result<Self> {
        let shells = shells.into();
        if shells.is_empty() {
            return Err(process_failure(
                "create retained shell pool",
                None,
                "retained shell pool requires at least one shell",
                RetryClass::NotRetryable,
            ));
        }
        Ok(Self {
            shells: Arc::from(shells),
            next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    pub async fn open(client: &ProcessClient, size: usize) -> Result<Self> {
        if size == 0 {
            return Err(process_failure(
                "open retained shell pool",
                Some(client.sandbox_id.clone()),
                "retained shell pool size must be greater than zero",
                RetryClass::NotRetryable,
            ));
        }
        let mut shells = Vec::with_capacity(size);
        for _ in 0..size {
            shells.push(client.shell().await?);
        }
        Self::new(shells)
    }

    pub fn len(&self) -> usize {
        self.shells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shells.is_empty()
    }

    pub fn lease(&self) -> Shell {
        let index = self.next.fetch_add(1, Ordering::AcqRel) % self.shells.len();
        self.shells[index].clone()
    }

    pub fn slots(&self) -> Vec<Shell> {
        self.shells.iter().cloned().collect()
    }

    pub fn slot(&self, index: usize) -> Option<Shell> {
        self.shells.get(index).cloned()
    }

    pub async fn run(&self, command: Command) -> Result<CommandOutput> {
        self.lease().run(command).await
    }

    pub async fn close(self) -> Result<()> {
        let mut first_error = None;
        for shell in self.shells.iter().cloned() {
            if let Err(error) = shell.close().await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}

impl ProcessClient {
    pub async fn shell_pool(&self, size: usize) -> Result<ShellPool> {
        ShellPool::open(self, size).await
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShellCommandPlan {
    mode: CommandMode,
    cwd: Option<String>,
    env: BTreeMap<String, String>,
}

impl ShellCommandPlan {
    fn new(command: &Command) -> Result<Self> {
        if command.pty.is_some() {
            return Err(process_failure(
                "prepare retained shell command",
                None,
                "pty commands are unsupported for retained shell dispatch",
                RetryClass::NotRetryable,
            ));
        }
        if command.stdin.is_some() {
            return Err(process_failure(
                "prepare retained shell command",
                None,
                "initial stdin is unsupported for retained shell dispatch; use Shell::send_input for an active command",
                RetryClass::NotRetryable,
            ));
        }
        Ok(Self {
            mode: command.mode.clone(),
            cwd: command.cwd.clone(),
            env: command.env.clone(),
        })
    }

    fn dispatch_script(&self, nonce: &str) -> String {
        let stdout_path = format!("/tmp/fk-shell-{nonce}.stdout");
        let stderr_path = format!("/tmp/fk-shell-{nonce}.stderr");
        let mut script = String::new();
        write!(
            script,
            "__fk_out={}; __fk_err={}; rm -f \"$__fk_out\" \"$__fk_err\"; ",
            shell_quote(&stdout_path),
            shell_quote(&stderr_path)
        )
        .expect("write to string");
        script.push_str("{ ");
        script.push_str(&self.invocation_script());
        script.push_str("; __fk_rc=$?; } >\"$__fk_out\" 2>\"$__fk_err\"; ");
        script.push_str("__fk_b64_out=$(base64 < \"$__fk_out\" | tr -d '\\n'); ");
        script.push_str("__fk_b64_err=$(base64 < \"$__fk_err\" | tr -d '\\n'); ");
        write!(
            script,
            "printf '\\036FK_STDOUT:{nonce}:%s\\n' \"$__fk_b64_out\"; "
        )
        .expect("write to string");
        write!(
            script,
            "printf '\\036FK_STDERR:{nonce}:%s\\n' \"$__fk_b64_err\"; "
        )
        .expect("write to string");
        write!(script, "printf '\\036FK_END:{nonce}:%d\\n' \"$__fk_rc\"; ")
            .expect("write to string");
        script.push_str("rm -f \"$__fk_out\" \"$__fk_err\"\n");
        script
    }

    fn invocation_script(&self) -> String {
        let command = match &self.mode {
            CommandMode::Shell(command) => command.clone(),
            CommandMode::Argv { program, args } => {
                let mut pieces = Vec::with_capacity(args.len() + 1);
                pieces.push(shell_quote(program));
                pieces.extend(args.iter().map(|arg| shell_quote(arg)));
                pieces.join(" ")
            }
        };
        if self.cwd.is_none() && self.env.is_empty() {
            return command;
        }
        let mut prefix = String::from("( ");
        if let Some(cwd) = &self.cwd {
            prefix.push_str("cd ");
            prefix.push_str(&shell_quote(cwd));
            prefix.push_str(" && ");
        }
        for (key, value) in &self.env {
            prefix.push_str(key);
            prefix.push('=');
            prefix.push_str(&shell_quote(value));
            prefix.push(' ');
        }
        prefix.push_str(&command);
        prefix.push_str(" )");
        prefix
    }
}

struct ShellDispatchCapture {
    nonce: String,
    stdout_line: Vec<u8>,
    stdout: Option<Bytes>,
    stderr: Option<Bytes>,
    protocol_stderr: Vec<u8>,
}

impl ShellDispatchCapture {
    fn new(nonce: String) -> Self {
        Self {
            nonce,
            stdout_line: Vec::new(),
            stdout: None,
            stderr: None,
            protocol_stderr: Vec::new(),
        }
    }

    fn push_stdout(&mut self, bytes: &Bytes) -> Result<Option<CommandOutput>> {
        self.stdout_line.extend_from_slice(bytes);
        while let Some(newline) = self.stdout_line.iter().position(|byte| *byte == b'\n') {
            let line = self.stdout_line.drain(..=newline).collect::<Vec<_>>();
            if let Some(output) = self.handle_protocol_line(&line)? {
                return Ok(Some(output));
            }
        }
        Ok(None)
    }

    fn push_protocol_stderr(&mut self, bytes: &Bytes) {
        self.protocol_stderr.extend_from_slice(bytes);
    }

    fn handle_protocol_line(&mut self, line: &[u8]) -> Result<Option<CommandOutput>> {
        let Some(stripped) = line.strip_prefix(b"\x1e") else {
            return Ok(None);
        };
        let line = std::str::from_utf8(stripped).map_err(|err| {
            process_failure(
                "decode retained shell protocol",
                None,
                format!("protocol line was not utf-8: {err}"),
                RetryClass::NotRetryable,
            )
        })?;
        let line = line.trim_end_matches('\n');
        if let Some(encoded) = line.strip_prefix(&format!("FK_STDOUT:{}:", self.nonce)) {
            self.stdout = Some(decode_shell_payload(encoded, "stdout")?);
            return Ok(None);
        }
        if let Some(encoded) = line.strip_prefix(&format!("FK_STDERR:{}:", self.nonce)) {
            self.stderr = Some(decode_shell_payload(encoded, "stderr")?);
            return Ok(None);
        }
        if let Some(code) = line.strip_prefix(&format!("FK_END:{}:", self.nonce)) {
            let code = code.parse::<i32>().map_err(|err| {
                process_failure(
                    "decode retained shell exit status",
                    None,
                    format!("invalid exit status `{code}`: {err}"),
                    RetryClass::NotRetryable,
                )
            })?;
            return Ok(Some(CommandOutput {
                status: CommandStatus::Exited(CommandExit { code }),
                stdout: self.stdout.take().unwrap_or_default(),
                stderr: self.stderr.take().unwrap_or_default(),
            }));
        }
        Ok(None)
    }
}

fn decode_shell_payload(encoded: &str, stream: &'static str) -> Result<Bytes> {
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map(Bytes::from)
        .map_err(|err| {
            process_failure(
                "decode retained shell payload",
                None,
                format!("invalid {stream} payload: {err}"),
                RetryClass::NotRetryable,
            )
        })
}

fn shell_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn process_failure(
    operation: &'static str,
    sandbox_id: Option<SandboxId>,
    reason: impl Into<String>,
    retry: RetryClass,
) -> Error {
    Error::ProcessFailure(ProcessFailure {
        operation,
        sandbox_id,
        process_id: None,
        reason: reason.into(),
        retry,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        Command, CommandMode, ProcessEvent, ProcessEventStream, ProcessInfo, ProcessInput,
        ProcessSelector, ProcessStatus, PtySize, ShellCommandPlan, ShellPool, Signal,
    };
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use bytes::Bytes;
    use futures_util::stream;

    use crate::backend::{
        BackendInfo, EventControl, FilesystemControl, LogControl, MetricControl, PauseControl,
        PortControl, ProcessControl, SandboxBackend, SandboxControl, SnapshotControl,
        TemplateControl, WarmPoolControl,
    };
    use crate::capability::Capabilities;
    use crate::contract::FakeBackend;
    use crate::error::{Error, ProcessFailure, Result};
    use crate::ids::{BackendName, SandboxId};
    use crate::runtime::RuntimePreflight;

    #[test]
    fn retained_shell_rejects_pty_command_before_dispatch() {
        let error = ShellCommandPlan::new(&Command::shell("printf hi").pty(PtySize::new(24, 80)))
            .expect_err("pty is unsupported for retained dispatch");

        assert!(matches!(
            error,
            Error::ProcessFailure(ProcessFailure {
                operation: "prepare retained shell command",
                ..
            })
        ));
        assert!(error.to_string().contains("pty"));
    }

    #[test]
    fn retained_shell_preserves_argv_boundaries_in_dispatch_script() {
        let plan = ShellCommandPlan::new(&Command::argv(
            "/usr/bin/printf",
            ["%s|%s", "hello world", "$HOME;rm"],
        ))
        .expect("argv command is supported");

        let script = plan.dispatch_script("abc123");
        assert!(script.contains("'/usr/bin/printf' '%s|%s' 'hello world' '$HOME;rm'"));
        assert!(script.contains("FK_END:abc123:%d"));
        assert!(script.contains("\"$__fk_rc\""));
    }

    #[test]
    fn retained_shell_per_call_overrides_run_in_subshell() {
        let plan = ShellCommandPlan::new(
            &Command::shell("pwd && printf %s \"$FK_TEST\"")
                .cwd("/work tree")
                .env("FK_TEST", "value with spaces"),
        )
        .expect("shell command is supported");

        let script = plan.dispatch_script("abc123");
        assert!(script.contains("( cd '/work tree' && FK_TEST='value with spaces' "));
        assert!(matches!(&plan.mode, CommandMode::Shell(_)));
    }

    #[test]
    fn retained_shell_protocol_decodes_stdout_stderr_and_exit_status() {
        let mut capture = super::ShellDispatchCapture::new("abc123".to_owned());

        assert!(
            capture
                .push_stdout(&Bytes::from_static(b"\x1eFK_STDOUT:abc123:aGkA\n"))
                .expect("stdout decodes")
                .is_none()
        );
        assert!(
            capture
                .push_stdout(&Bytes::from_static(b"\x1eFK_STDERR:abc123:ZXJyCg==\n"))
                .expect("stderr decodes")
                .is_none()
        );
        let output = capture
            .push_stdout(&Bytes::from_static(b"\x1eFK_END:abc123:7\n"))
            .expect("end decodes")
            .expect("end returns output");

        assert_eq!(output.stdout, Bytes::from_static(b"hi\0"));
        assert_eq!(output.stderr, Bytes::from_static(b"err\n"));
        assert_eq!(
            output.status,
            super::CommandStatus::Exited(super::CommandExit { code: 7 })
        );
    }

    #[test]
    fn retained_shell_protocol_ignores_user_nonce_lookalike_with_wrong_nonce() {
        let mut capture = super::ShellDispatchCapture::new("real".to_owned());

        assert!(
            capture
                .push_stdout(&Bytes::from_static(b"\x1eFK_END:fake:0\n"))
                .expect("wrong nonce ignored")
                .is_none()
        );
        assert!(
            capture
                .push_stdout(&Bytes::from_static(b"\x1eFK_STDOUT:real:b2s=\n"))
                .expect("stdout decodes")
                .is_none()
        );
        assert!(
            capture
                .push_stdout(&Bytes::from_static(b"\x1eFK_STDERR:real:\n"))
                .expect("stderr decodes")
                .is_none()
        );
        let output = capture
            .push_stdout(&Bytes::from_static(b"\x1eFK_END:real:0\n"))
            .expect("end decodes")
            .expect("end returns output");

        assert_eq!(output.stdout, Bytes::from_static(b"ok"));
    }

    #[tokio::test]
    async fn retained_shell_client_starts_sends_and_decodes_command_output() {
        let backend = Arc::new(RetainedShellBackend::new());
        let client = super::ProcessClient::from((
            backend.clone() as Arc<dyn SandboxBackend>,
            SandboxId::new("sbx_retained").expect("sandbox id"),
        ));

        let shell = client.shell().await.expect("open shell");
        let plan = ShellCommandPlan::new(&Command::shell("printf hi && printf err >&2"))
            .expect("shell command plan");
        let output = shell
            .run_plan_with_nonce(plan, "00000000000000000000000000000000".to_owned())
            .await
            .expect("run retained shell command");

        assert_eq!(output.stdout, Bytes::from_static(b"hi"));
        assert_eq!(output.stderr, Bytes::from_static(b"err"));
        assert_eq!(
            output.status,
            super::CommandStatus::Exited(super::CommandExit::success())
        );
        let sent = backend.sent.lock().expect("sent lock");
        assert_eq!(sent.len(), 1);
        assert!(String::from_utf8_lossy(&sent[0]).contains("printf hi && printf err >&2"));
    }

    #[tokio::test]
    async fn retained_shell_clone_drop_does_not_close_shared_shell() {
        let backend = Arc::new(RetainedShellBackend::new());
        let client = super::ProcessClient::from((
            backend.clone() as Arc<dyn SandboxBackend>,
            SandboxId::new("sbx_retained_clone").expect("sandbox id"),
        ));
        let shell = client.shell().await.expect("open shell");
        let lease = shell.clone();
        drop(lease);

        let plan = ShellCommandPlan::new(&Command::shell("printf hi")).expect("shell command plan");
        let output = shell
            .run_plan_with_nonce(plan, "00000000000000000000000000000000".to_owned())
            .await
            .expect("run retained shell command after clone drop");

        assert_eq!(output.stdout, Bytes::from_static(b"hi"));
        assert_eq!(
            output.status,
            super::CommandStatus::Exited(super::CommandExit::success())
        );
    }

    #[test]
    fn retained_shell_pool_rejects_empty_pool() {
        let Err(error) = ShellPool::new(Vec::new()) else {
            panic!("empty pool should be rejected");
        };

        assert!(matches!(
            error,
            Error::ProcessFailure(ProcessFailure {
                operation: "create retained shell pool",
                ..
            })
        ));
        assert!(error.to_string().contains("at least one shell"));
    }

    #[test]
    fn retained_shell_pool_exposes_stable_slots_for_agent_assignment() {
        let backend = Arc::new(RetainedShellBackend::new()) as Arc<dyn SandboxBackend>;
        let sandbox_id = SandboxId::new("sbx_pool").expect("sandbox id");
        let shells = (0..4)
            .map(|index| {
                super::Shell::new(
                    backend.clone(),
                    sandbox_id.clone(),
                    crate::ids::ProcessTag::new(format!("agent-{index}")).expect("tag"),
                    Box::pin(stream::empty()),
                )
            })
            .collect::<Vec<_>>();
        let pool = ShellPool::new(shells).expect("pool");

        let slots = pool.slots();

        assert_eq!(slots.len(), 4);
        assert!(pool.slot(0).is_some());
        assert!(pool.slot(3).is_some());
        assert!(pool.slot(4).is_none());
    }

    struct RetainedShellBackend {
        fallback: FakeBackend,
        sent: Mutex<Vec<Vec<u8>>>,
    }

    impl RetainedShellBackend {
        fn new() -> Self {
            Self {
                fallback: FakeBackend::new(),
                sent: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl SandboxBackend for RetainedShellBackend {
        async fn capabilities(&self) -> Result<Capabilities> {
            self.fallback.capabilities().await
        }

        async fn preflight(&self) -> Result<RuntimePreflight> {
            self.fallback.preflight().await
        }

        async fn info(&self) -> Result<BackendInfo> {
            Ok(BackendInfo::new(BackendName::new("retained-test")?))
        }

        fn templates(&self) -> &dyn TemplateControl {
            self.fallback.templates()
        }

        fn sandboxes(&self) -> &dyn SandboxControl {
            self.fallback.sandboxes()
        }

        fn snapshots(&self) -> &dyn SnapshotControl {
            self.fallback.snapshots()
        }

        fn processes(&self) -> Option<&dyn ProcessControl> {
            Some(self)
        }

        fn filesystems(&self) -> Option<&dyn FilesystemControl> {
            self.fallback.filesystems()
        }

        fn ports(&self) -> Option<&dyn PortControl> {
            self.fallback.ports()
        }

        fn pause(&self) -> Option<&dyn PauseControl> {
            self.fallback.pause()
        }

        fn warm_pool(&self) -> Option<&dyn WarmPoolControl> {
            self.fallback.warm_pool()
        }

        fn events(&self) -> Option<&dyn EventControl> {
            self.fallback.events()
        }

        fn logs(&self) -> Option<&dyn LogControl> {
            self.fallback.logs()
        }

        fn metrics(&self) -> Option<&dyn MetricControl> {
            self.fallback.metrics()
        }
    }

    #[async_trait]
    impl ProcessControl for RetainedShellBackend {
        async fn run_process(
            &self,
            sandbox: &SandboxId,
            command: Command,
        ) -> Result<super::CommandOutput> {
            self.fallback.run_process(sandbox, command).await
        }

        async fn start_process(
            &self,
            sandbox: &SandboxId,
            command: Command,
        ) -> Result<ProcessInfo> {
            self.fallback.start_process(sandbox, command).await
        }

        async fn start_process_stream(
            &self,
            _sandbox: &SandboxId,
            _command: Command,
        ) -> Result<ProcessEventStream> {
            Ok(Box::pin(stream::iter(vec![
                Ok(ProcessEvent::Started(ProcessInfo {
                    id: crate::ids::ProcessId::new("shell_1").expect("process id"),
                    tag: None,
                    status: ProcessStatus::Running,
                })),
                Ok(ProcessEvent::Stdout(Bytes::from_static(b"\x1eFK_STDOUT:"))),
                Ok(ProcessEvent::Stdout(Bytes::from_static(
                    b"00000000000000000000000000000000:aGk=\n",
                ))),
                Ok(ProcessEvent::Stdout(Bytes::from_static(
                    b"\x1eFK_STDERR:00000000000000000000000000000000:ZXJy\n",
                ))),
                Ok(ProcessEvent::Stdout(Bytes::from_static(
                    b"\x1eFK_END:00000000000000000000000000000000:0\n",
                ))),
            ])))
        }

        async fn list_processes(&self, sandbox: &SandboxId) -> Result<Vec<ProcessInfo>> {
            self.fallback.list_processes(sandbox).await
        }

        async fn connect_process(
            &self,
            sandbox: &SandboxId,
            selector: ProcessSelector,
        ) -> Result<ProcessInfo> {
            self.fallback.connect_process(sandbox, selector).await
        }

        async fn signal_process(
            &self,
            sandbox: &SandboxId,
            selector: ProcessSelector,
            signal: Signal,
        ) -> Result<()> {
            self.fallback
                .signal_process(sandbox, selector, signal)
                .await
        }

        async fn send_process_input(
            &self,
            _sandbox: &SandboxId,
            _selector: ProcessSelector,
            input: ProcessInput,
        ) -> Result<()> {
            if let ProcessInput::Bytes(bytes) = input {
                self.sent.lock().expect("sent lock").push(bytes.to_vec());
            }
            Ok(())
        }

        async fn close_process_stdin(
            &self,
            sandbox: &SandboxId,
            selector: ProcessSelector,
        ) -> Result<()> {
            self.fallback.close_process_stdin(sandbox, selector).await
        }

        async fn resize_process_pty(
            &self,
            sandbox: &SandboxId,
            selector: ProcessSelector,
            size: PtySize,
        ) -> Result<()> {
            self.fallback
                .resize_process_pty(sandbox, selector, size)
                .await
        }

        async fn wait_process(
            &self,
            sandbox: &SandboxId,
            selector: ProcessSelector,
        ) -> Result<super::CommandOutput> {
            self.fallback.wait_process(sandbox, selector).await
        }
    }
}
