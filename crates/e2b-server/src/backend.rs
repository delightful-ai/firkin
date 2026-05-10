//! backend — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::control_plane::ControlPlaneError;
#[allow(unused_imports)]
use crate::envd_http::{EnvdProcessHttpServer, HostEnvdAdapter};
#[allow(unused_imports)]
use crate::registry::{
    LocalPodRegistry, LocalSandboxRegistry, LocalTemplateRegistry, LocalVolumeRegistry,
};
#[allow(unused_imports)]
use crate::state::{LocalRuntimeState, LocalRuntimeStateStoreError};
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use bytes::Bytes;
#[allow(unused_imports)]
use firkin_e2b_contract::{
    BackendError, DEFAULT_CODE_INTERPRETER_PORT, DEFAULT_MCP_PORT, FollowupSnapshot, PausedSandbox,
    PodRuntimeConfig, PortTarget, PreparedTemplate, RuntimeAdapter, RuntimeCapabilitySet,
    RuntimePod, RuntimeSandbox, RuntimeTemplateBuild, SandboxRuntimeConfig, SnapshotRef,
    StartPodRequest, StartSandboxRequest,
};
#[allow(unused_imports)]
use firkin_e2b_contract::{PortProxyStream, SandboxExpiration, SandboxExpirationAction};
#[allow(unused_imports)]
use firkin_e2b_wire::ControlPlaneResponse;
#[allow(unused_imports)]
use firkin_e2b_wire::VolumeWriteOptions;
#[allow(unused_imports)]
use firkin_e2b_wire::{
    AssignTemplateTags, ConnectRequest, ConnectedSandbox, ControlPlaneMethod, ControlPlaneRequest,
    CreateSnapshotRequest, FollowupSandboxCreateRequest, PodCreateRequest, PodInfo, RefreshRequest,
    RemoveTemplateTags, SandboxCreateRequest, SnapshotInfo, TemplateBuildRequestInfo,
    TemplateBuildStart, TemplateBuildStatus, TemplateUpdateRequest, TimeoutRequest,
    VolumeCreateRequest, VolumeMetadataRequest, validate_pod_container_request,
};
#[allow(unused_imports)]
use firkin_e2b_wire::{
    LogLevel, PodContainerCreateRequest, PodContainerInfo, PodContainerOutput, SandboxLogEntry,
    SandboxLogs, SandboxMetric, SandboxState, TemplateBuildRequest, TemplateInstructionKind,
    VolumeMount,
};
#[allow(unused_imports)]
use firkin_envd::DEFAULT_ENVD_PORT;
#[allow(unused_imports)]
use firkin_types::Hostname;
#[allow(unused_imports)]
use firkin_types::SandboxNetworkPolicy;
#[allow(unused_imports)]
use http_body_util::Full;
#[allow(unused_imports)]
use hyper::body::Incoming;
#[allow(unused_imports)]
use hyper::header::CONTENT_TYPE;
#[allow(unused_imports)]
use hyper::service::service_fn;
#[allow(unused_imports)]
use hyper::{Request, Response, StatusCode};
#[allow(unused_imports)]
use hyper_util::rt::TokioIo;
#[allow(unused_imports)]
use serde::Deserialize;
#[allow(unused_imports)]
use serde::Serialize;
#[allow(unused_imports)]
use serde::de::DeserializeOwned;
#[allow(unused_imports)]
use serde_json::Value as JsonValue;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::convert::Infallible;
#[allow(unused_imports)]
use std::fs::OpenOptions;
use std::io::Write as _;
#[allow(unused_imports)]
use std::path::Component;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::process::Stdio;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use std::time::SystemTime;
#[allow(unused_imports)]
use std::time::UNIX_EPOCH;
#[allow(unused_imports)]
use time::Duration as TimeDuration;
#[allow(unused_imports)]
use time::OffsetDateTime;
#[allow(unused_imports)]
use time::format_description::well_known::Rfc3339;
#[allow(unused_imports)]
use tokio::net::TcpListener;
#[allow(unused_imports)]
use tokio::process::Child;
#[allow(unused_imports)]
use tokio::process::Command;
#[allow(unused_imports)]
use tokio::sync::Mutex;
#[allow(unused_imports)]
use tokio::task::JoinHandle;
pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;
const LOCAL_RUNTIME_STATE_SCHEMA: &str = "firkin.e2b.local-runtime-state";
const LOCAL_RUNTIME_STATE_VERSION: u32 = 1;
fn json_response<T>(status: u16, body: &T) -> Result<ControlPlaneResponse, ControlPlaneError>
where
    T: Serialize,
{
    ControlPlaneResponse::json(status, body).map_err(Into::into)
}
/// Host-backed local E2B runtime adapter.
///
/// Each sandbox gets a directory under `root` and a loopback
/// [`EnvdProcessHttpServer`] backed by [`HostEnvdAdapter`]. This is a local
/// development/runtime adapter for SDK compatibility; it does not provide VM
/// isolation, guest envd deployment, or Cube network policy enforcement.
#[derive(Clone, Debug)]
pub struct HostRuntimeAdapter {
    pub(crate) root: Arc<PathBuf>,
    #[allow(missing_docs)]
    pub domain: Hostname,
    #[allow(missing_docs)]
    pub state: Arc<Mutex<HostRuntimeState>>,
}
impl HostRuntimeAdapter {
    /// Construct a host-backed runtime adapter rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, domain: Hostname) -> Self {
        Self {
            root: Arc::new(root.into()),
            domain,
            state: Arc::new(Mutex::new(HostRuntimeState::default())),
        }
    }
    /// Return the adapter root.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.root.as_ref()
    }
    /// Return the configured SDK-visible domain.
    #[must_use]
    pub const fn domain(&self) -> &Hostname {
        &self.domain
    }
    fn timestamps(timeout_seconds: Option<u64>) -> Result<(String, String), BackendError> {
        let started_at = OffsetDateTime::now_utc();
        let timeout = i64::try_from(timeout_seconds.unwrap_or(300))
            .map_err(|_| BackendError::Runtime("sandbox timeout is too large".to_owned()))?;
        let end_at = started_at + TimeDuration::seconds(timeout);
        Ok((
            started_at
                .format(&Rfc3339)
                .expect("RFC3339 formatting current UTC time is infallible"),
            end_at
                .format(&Rfc3339)
                .expect("RFC3339 formatting current UTC time is infallible"),
        ))
    }
    /// Restore host envd listeners for sandboxes present in persisted
    /// control-plane state.
    ///
    /// This restores the host-runtime transport for previously registered
    /// sandboxes. It does not recreate running child processes.
    ///
    /// # Errors
    ///
    /// Returns filesystem or listener bind errors.
    pub async fn restore_from_state(&self, state: &LocalRuntimeState) -> Result<(), BackendError> {
        let mut next_sandbox = 0_u64;
        for record in state.sandboxes.sandboxes.values() {
            let sandbox = &record.info;
            if let Some(suffix) = sandbox.sandbox_id.strip_prefix("sbx_host_")
                && let Ok(id) = suffix.parse::<u64>()
            {
                next_sandbox = next_sandbox.max(id);
            }
            self.start_envd_for_sandbox(
                sandbox.sandbox_id.clone(),
                sandbox.state == SandboxState::Paused,
                &record.create_request.env_vars,
                &sandbox.volume_mounts,
                Vec::new(),
            )
            .await?;
        }
        self.state.lock().await.next_sandbox = next_sandbox;
        Ok(())
    }
    async fn start_envd_for_sandbox(
        &self,
        sandbox_id: String,
        paused: bool,
        env_vars: &BTreeMap<String, String>,
        volume_mounts: &[VolumeMount],
        start_children: Vec<Arc<Mutex<Child>>>,
    ) -> Result<u16, BackendError> {
        let sandbox_root = self.root.join(&sandbox_id);
        tokio::fs::create_dir_all(&sandbox_root)
            .await
            .map_err(|error| BackendError::Runtime(format!("create sandbox root: {error}")))?;
        for mount in volume_mounts {
            self.attach_host_volume(&sandbox_root, mount).await?;
        }
        let envd_adapter = HostEnvdAdapter::new_with_envs(&sandbox_root, env_vars.clone()).await?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| BackendError::Runtime(format!("bind host envd listener: {error}")))?;
        let envd_port = listener
            .local_addr()
            .map_err(|error| BackendError::Runtime(format!("read host envd listener: {error}")))?
            .port();
        let envd_server_adapter = envd_adapter.clone();
        let envd_task = tokio::spawn(async move {
            let _ = EnvdProcessHttpServer::new(envd_server_adapter)
                .with_access_token("host-envd-token")
                .serve(listener)
                .await;
        });
        let code_listener = TcpListener::bind("127.0.0.1:0").await.map_err(|error| {
            BackendError::Runtime(format!("bind host code-interpreter listener: {error}"))
        })?;
        let code_interpreter_port = code_listener
            .local_addr()
            .map_err(|error| {
                BackendError::Runtime(format!("read host code-interpreter listener: {error}"))
            })?
            .port();
        let code_sandbox_id = sandbox_id.clone();
        let code_interpreter_task = tokio::spawn(async move {
            let _ = serve_host_service_probe(
                code_listener,
                code_sandbox_id,
                "code-interpreter",
                &["/", "/health"],
            )
            .await;
        });
        let mcp_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| BackendError::Runtime(format!("bind host MCP listener: {error}")))?;
        let mcp_port = mcp_listener
            .local_addr()
            .map_err(|error| BackendError::Runtime(format!("read host MCP listener: {error}")))?
            .port();
        let mcp_sandbox_id = sandbox_id.clone();
        let mcp_task = tokio::spawn(async move {
            let _ =
                serve_host_service_probe(mcp_listener, mcp_sandbox_id, "mcp", &["/", "/mcp"]).await;
        });
        let previous = {
            let mut state = self.state.lock().await;
            state.sandboxes.insert(
                sandbox_id,
                HostRuntimeSandbox {
                    root: sandbox_root,
                    envd_adapter,
                    start_children,
                    envd_port,
                    envd_task,
                    code_interpreter_port,
                    code_interpreter_task,
                    mcp_port,
                    mcp_task,
                    paused,
                },
            )
        };
        if let Some(previous) = previous {
            stop_host_runtime_sandbox_services(&previous).await;
        }
        Ok(envd_port)
    }
    async fn attach_host_volume(
        &self,
        sandbox_root: &Path,
        mount: &VolumeMount,
    ) -> Result<(), BackendError> {
        let volume_root = self.host_volume_path(&mount.name)?;
        tokio::fs::create_dir_all(&volume_root)
            .await
            .map_err(|error| BackendError::Runtime(format!("create host volume: {error}")))?;
        let mount_path = rooted_child_path(sandbox_root, &mount.path, "volume mount path")?;
        if let Some(parent) = mount_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| BackendError::Runtime(format!("create mount parent: {error}")))?;
        }
        remove_existing_mount_path(&mount_path).await?;
        symlink_dir(&volume_root, &mount_path)
            .map_err(|error| BackendError::Runtime(format!("mount host volume: {error}")))
    }
    fn host_volume_path(&self, name: &str) -> Result<PathBuf, BackendError> {
        rooted_child_path(&self.root.join("volumes"), name, "volume name")
    }
    fn host_template_artifact_path(&self, template_id: &str, build_id: &str) -> PathBuf {
        self.root.join("templates").join(template_id).join(build_id)
    }
    fn host_snapshot_artifact_path(&self, snapshot_id: &str) -> Result<PathBuf, BackendError> {
        rooted_child_path(&self.root.join("snapshots"), snapshot_id, "snapshot id")
    }
    fn host_pod_path(&self, pod_id: &str) -> Result<PathBuf, BackendError> {
        rooted_child_path(&self.root.join("pods"), pod_id, "pod id")
    }
    fn host_pod_container_path(&self, pod_id: &str, name: &str) -> Result<PathBuf, BackendError> {
        rooted_child_path(
            &self.host_pod_path(pod_id)?.join("containers"),
            name,
            "pod container name",
        )
    }
    async fn create_host_pod_container(
        &self,
        pod_id: &str,
        request: &PodContainerCreateRequest,
    ) -> Result<PodContainerInfo, BackendError> {
        let container_root = self.host_pod_container_path(pod_id, &request.name)?;
        if container_root.exists() {
            return Err(BackendError::AlreadyExists(request.name.clone()));
        }
        tokio::fs::create_dir_all(&container_root)
            .await
            .map_err(|error| {
                BackendError::Runtime(format!("create host pod container: {error}"))
            })?;
        tokio::fs::write(
            container_root.join("template_id"),
            request.template_id.as_bytes(),
        )
        .await
        .map_err(|error| {
            BackendError::Runtime(format!("write host pod container metadata: {error}"))
        })?;
        Ok(PodContainerInfo::running(request))
    }
}
#[async_trait]
impl RuntimeAdapter for HostRuntimeAdapter {
    async fn preflight(&self) -> Result<RuntimeCapabilitySet, BackendError> {
        Ok(RuntimeCapabilitySet {
            backend: "host".to_owned(),
            supported: vec![
                "e2b-control-plane".to_owned(),
                "domain-host-proxy".to_owned(),
                "e2b-envd-compatible-api".to_owned(),
                "host-command-execution".to_owned(),
                "host-filesystem".to_owned(),
            ],
            unsupported: vec![
                (
                    "vm-isolation".to_owned(),
                    "host adapter executes on the host under a rooted directory".to_owned(),
                ),
                (
                    "e2b-network-policy".to_owned(),
                    "host adapter does not enforce per-sandbox network policy".to_owned(),
                ),
                (
                    "snapshot-backed-pause-resume-connect".to_owned(),
                    "pause/resume is control-plane state only for the host adapter".to_owned(),
                ),
            ],
        })
    }
    async fn prepare_template(
        &self,
        request: TemplateBuildRequest,
    ) -> Result<PreparedTemplate, BackendError> {
        let name = request.name.as_deref().unwrap_or("host-template");
        Ok(PreparedTemplate {
            template_id: format!("host-{name}"),
            build_id: "host-build".to_owned(),
            artifact: self.root.join("templates").join(name).display().to_string(),
            has_envd: true,
            artifact_integrity: None,
        })
    }
    async fn build_template(
        &self,
        request: RuntimeTemplateBuild,
    ) -> Result<PreparedTemplate, BackendError> {
        let artifact = self.host_template_artifact_path(&request.template_id, &request.build_id);
        remove_existing_mount_path(&artifact).await?;
        tokio::fs::create_dir_all(&artifact)
            .await
            .map_err(|error| BackendError::Runtime(format!("create host template: {error}")))?;
        let mut envs = BTreeMap::new();
        let mut workdir = artifact.clone();
        let mut user = None;
        for step in &request.start.steps {
            match step.kind {
                TemplateInstructionKind::Copy => {
                    let hash = step.files_hash.as_deref().ok_or_else(|| {
                        BackendError::Runtime("template COPY step is missing filesHash".to_owned())
                    })?;
                    let archive = request
                        .uploaded_files
                        .get(hash)
                        .ok_or_else(|| BackendError::NotFound(hash.to_owned()))?;
                    let source = step.args.first().ok_or_else(|| {
                        BackendError::Runtime("template COPY step is missing source".to_owned())
                    })?;
                    let destination = step.args.get(1).ok_or_else(|| {
                        BackendError::Runtime(
                            "template COPY step is missing destination".to_owned(),
                        )
                    })?;
                    apply_copy_archive_to_artifact(archive, source, destination, &artifact)?;
                }
                TemplateInstructionKind::Env => {
                    let key = step.args.first().ok_or_else(|| {
                        BackendError::Runtime("template ENV step is missing key".to_owned())
                    })?;
                    let value = step.args.get(1).ok_or_else(|| {
                        BackendError::Runtime("template ENV step is missing value".to_owned())
                    })?;
                    envs.insert(key.clone(), value.clone());
                }
                TemplateInstructionKind::Workdir => {
                    let path = step.args.first().ok_or_else(|| {
                        BackendError::Runtime("template WORKDIR step is missing path".to_owned())
                    })?;
                    workdir = rooted_child_path(&artifact, path, "template WORKDIR")?;
                    tokio::fs::create_dir_all(&workdir).await.map_err(|error| {
                        BackendError::Runtime(format!("create template WORKDIR: {error}"))
                    })?;
                }
                TemplateInstructionKind::Run => {
                    let command = step.args.first().ok_or_else(|| {
                        BackendError::Runtime("template RUN step is missing command".to_owned())
                    })?;
                    run_host_template_command(command, &workdir, &envs, user.as_deref()).await?;
                }
                TemplateInstructionKind::User => {
                    let value = step.args.first().ok_or_else(|| {
                        BackendError::Runtime("template USER step is missing user".to_owned())
                    })?;
                    user = Some(value.clone());
                }
            }
        }
        write_host_template_metadata(
            &artifact,
            &HostTemplateMetadata {
                envs: envs.clone(),
                start_cmd: request.start.start_cmd.clone(),
                ready_cmd: request.start.ready_cmd.clone(),
            },
        )
        .await?;
        Ok(PreparedTemplate {
            template_id: request.template_id,
            build_id: request.build_id,
            artifact: artifact.display().to_string(),
            has_envd: true,
            artifact_integrity: None,
        })
    }
    async fn start(&self, request: StartSandboxRequest) -> Result<RuntimeSandbox, BackendError> {
        tokio::fs::create_dir_all(self.root.as_ref())
            .await
            .map_err(|error| BackendError::Runtime(format!("create host runtime root: {error}")))?;
        let sandbox_id = {
            let mut state = self.state.lock().await;
            state.next_sandbox = state.next_sandbox.saturating_add(1).max(1);
            format!("sbx_host_{}", state.next_sandbox)
        };
        let mut base_envs = BTreeMap::new();
        let mut template_metadata = HostTemplateMetadata::default();
        if let Some(template) = request.prepared_template.as_ref()
            && !template.artifact.is_empty()
        {
            let artifact = Path::new(&template.artifact);
            template_metadata = read_host_template_metadata(artifact).await?;
            base_envs = template_metadata.envs.clone();
            copy_host_directory_contents(artifact, &self.root.join(&sandbox_id)).await?;
        }
        base_envs.extend(request.create_request.env_vars.clone());
        let sandbox_root = self.root.join(&sandbox_id);
        let mut start_children = Vec::new();
        if let Some(command) = template_metadata.start_cmd.as_deref() {
            start_children
                .push(spawn_host_template_command(command, &sandbox_root, &base_envs, None).await?);
        }
        if let Some(command) = template_metadata.ready_cmd.as_deref()
            && let Err(error) =
                run_host_template_command(command, &sandbox_root, &base_envs, None).await
        {
            kill_host_template_children(&start_children).await;
            return Err(error);
        }
        let start_children_for_cleanup = start_children.clone();
        if let Err(error) = self
            .start_envd_for_sandbox(
                sandbox_id.clone(),
                false,
                &base_envs,
                &request.create_request.volume_mounts,
                start_children,
            )
            .await
        {
            kill_host_template_children(&start_children_for_cleanup).await;
            return Err(error);
        }
        let (started_at, end_at) = Self::timestamps(request.create_request.timeout.or(Some(300)))?;
        Ok(RuntimeSandbox {
            config: SandboxRuntimeConfig {
                sandbox_id,
                domain: self.domain.to_string(),
                envd_version: "host".to_owned(),
                envd_access_token: Some("host-envd-token".to_owned()),
                traffic_access_token: Some("host-traffic-token".to_owned()),
                started_at,
                end_at,
                cpu_count: 1,
                memory_mb: 512,
            },
            exposed_ports: vec![
                DEFAULT_ENVD_PORT,
                DEFAULT_CODE_INTERPRETER_PORT,
                DEFAULT_MCP_PORT,
            ],
        })
    }
    async fn start_followup(
        &self,
        mut request: StartSandboxRequest,
        snapshot: FollowupSnapshot,
    ) -> Result<RuntimeSandbox, BackendError> {
        request.prepared_template = Some(PreparedTemplate {
            template_id: snapshot.snapshot_id.clone(),
            build_id: snapshot.snapshot_id,
            artifact: snapshot.location,
            has_envd: true,
            artifact_integrity: snapshot.artifact_integrity,
        });
        self.start(request).await
    }
    async fn start_pod(&self, request: StartPodRequest) -> Result<RuntimePod, BackendError> {
        tokio::fs::create_dir_all(self.root.join("pods"))
            .await
            .map_err(|error| BackendError::Runtime(format!("create host pod root: {error}")))?;
        let pod_id = if let Some(pod_id) = request.create_request.pod_id.clone() {
            pod_id
        } else {
            let mut state = self.state.lock().await;
            state.next_pod = state.next_pod.saturating_add(1).max(1);
            format!("pod_host_{}", state.next_pod)
        };
        let pod_root = self.host_pod_path(&pod_id)?;
        if pod_root.exists() {
            return Err(BackendError::AlreadyExists(pod_id));
        }
        tokio::fs::create_dir_all(pod_root.join("emptydir"))
            .await
            .map_err(|error| {
                BackendError::Runtime(format!("create host pod emptyDir root: {error}"))
            })?;
        for volume in &request.create_request.empty_dirs {
            let volume_path = rooted_child_path(
                &pod_root.join("emptydir"),
                &volume.name,
                "pod emptyDir name",
            )?;
            tokio::fs::create_dir_all(volume_path)
                .await
                .map_err(|error| {
                    BackendError::Runtime(format!("create host pod emptyDir: {error}"))
                })?;
        }
        tokio::fs::create_dir_all(pod_root.join("containers"))
            .await
            .map_err(|error| {
                BackendError::Runtime(format!("create host pod containers root: {error}"))
            })?;
        let mut containers = Vec::with_capacity(request.create_request.containers.len());
        let mut container_paths = BTreeMap::new();
        for container in &request.create_request.containers {
            let info = self.create_host_pod_container(&pod_id, container).await?;
            container_paths.insert(
                container.name.clone(),
                self.host_pod_container_path(&pod_id, &container.name)?,
            );
            containers.push(info);
        }
        let (started_at, end_at) = Self::timestamps(request.create_request.timeout.or(Some(300)))?;
        self.state.lock().await.pods.insert(
            pod_id.clone(),
            HostRuntimePod {
                root: pod_root,
                containers: container_paths,
            },
        );
        Ok(RuntimePod {
            config: PodRuntimeConfig {
                pod_id,
                started_at,
                end_at,
            },
            containers,
        })
    }
    async fn stop_pod(&self, pod_id: &str) -> Result<(), BackendError> {
        let pod = self.state.lock().await.pods.remove(pod_id);
        let Some(pod) = pod else {
            return Err(BackendError::NotFound(pod_id.to_owned()));
        };
        tokio::fs::remove_dir_all(&pod.root)
            .await
            .map_err(|error| BackendError::Runtime(format!("remove host pod root: {error}")))?;
        Ok(())
    }
    async fn add_pod_container(
        &self,
        pod_id: &str,
        container: PodContainerCreateRequest,
    ) -> Result<PodContainerInfo, BackendError> {
        {
            let state = self.state.lock().await;
            let pod = state
                .pods
                .get(pod_id)
                .ok_or_else(|| BackendError::NotFound(pod_id.to_owned()))?;
            if pod.containers.contains_key(&container.name) {
                return Err(BackendError::AlreadyExists(container.name));
            }
        }
        let info = self.create_host_pod_container(pod_id, &container).await?;
        let mut state = self.state.lock().await;
        let pod = state
            .pods
            .get_mut(pod_id)
            .ok_or_else(|| BackendError::NotFound(pod_id.to_owned()))?;
        pod.containers.insert(
            container.name.clone(),
            self.host_pod_container_path(pod_id, &container.name)?,
        );
        Ok(info)
    }
    async fn remove_pod_container(
        &self,
        pod_id: &str,
        container_name: &str,
    ) -> Result<(), BackendError> {
        let container_path = {
            let mut state = self.state.lock().await;
            let pod = state
                .pods
                .get_mut(pod_id)
                .ok_or_else(|| BackendError::NotFound(pod_id.to_owned()))?;
            pod.containers
                .remove(container_name)
                .ok_or_else(|| BackendError::NotFound(container_name.to_owned()))?
        };
        tokio::fs::remove_dir_all(container_path)
            .await
            .map_err(|error| BackendError::Runtime(format!("remove host pod container: {error}")))
    }
    async fn stop(&self, sandbox_id: &str) -> Result<(), BackendError> {
        let sandbox = self.state.lock().await.sandboxes.remove(sandbox_id);
        let Some(sandbox) = sandbox else {
            return Err(BackendError::NotFound(sandbox_id.to_owned()));
        };
        stop_host_runtime_sandbox_services(&sandbox).await;
        tokio::fs::remove_dir_all(&sandbox.root)
            .await
            .map_err(|error| BackendError::Runtime(format!("remove sandbox root: {error}")))?;
        Ok(())
    }
    async fn pause(&self, sandbox_id: &str) -> Result<PausedSandbox, BackendError> {
        let mut state = self.state.lock().await;
        let sandbox = state
            .sandboxes
            .get_mut(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        sandbox.paused = true;
        Ok(PausedSandbox {
            sandbox_id: sandbox_id.to_owned(),
            snapshot_id: None,
        })
    }
    async fn resume(&self, paused: PausedSandbox) -> Result<RuntimeSandbox, BackendError> {
        let mut state = self.state.lock().await;
        let sandbox = state
            .sandboxes
            .get_mut(&paused.sandbox_id)
            .ok_or_else(|| BackendError::NotFound(paused.sandbox_id.clone()))?;
        sandbox.paused = false;
        let (started_at, end_at) = Self::timestamps(Some(300))?;
        Ok(RuntimeSandbox {
            config: SandboxRuntimeConfig {
                sandbox_id: paused.sandbox_id,
                domain: self.domain.to_string(),
                envd_version: "host".to_owned(),
                envd_access_token: Some("host-envd-token".to_owned()),
                traffic_access_token: Some("host-traffic-token".to_owned()),
                started_at,
                end_at,
                cpu_count: 1,
                memory_mb: 512,
            },
            exposed_ports: vec![
                DEFAULT_ENVD_PORT,
                DEFAULT_CODE_INTERPRETER_PORT,
                DEFAULT_MCP_PORT,
            ],
        })
    }
    async fn snapshot(
        &self,
        sandbox_id: &str,
        name: Option<String>,
    ) -> Result<SnapshotRef, BackendError> {
        let snapshot_id = name.unwrap_or_else(|| format!("{sandbox_id}-host-snapshot"));
        let snapshot_root = self.host_snapshot_artifact_path(&snapshot_id)?;
        if snapshot_root.exists() {
            return Err(BackendError::AlreadyExists(snapshot_id));
        }
        let sandbox_root = {
            let state = self.state.lock().await;
            state
                .sandboxes
                .get(sandbox_id)
                .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?
                .root
                .clone()
        };
        copy_host_directory_contents(&sandbox_root, &snapshot_root).await?;
        Ok(SnapshotRef {
            snapshot_id,
            location: Some(snapshot_root.display().to_string()),
            artifact_integrity: None,
        })
    }
    async fn delete_snapshot(&self, snapshot_id: &str) -> Result<(), BackendError> {
        let snapshot_root = self.host_snapshot_artifact_path(snapshot_id)?;
        if !snapshot_root.exists() {
            return Ok(());
        }
        tokio::fs::remove_dir_all(&snapshot_root)
            .await
            .map_err(|error| BackendError::Runtime(format!("remove host snapshot: {error}")))
    }
    async fn metrics(&self, sandbox_id: &str) -> Result<Vec<SandboxMetric>, BackendError> {
        let state = self.state.lock().await;
        let sandbox = state
            .sandboxes
            .get(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        let timestamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("RFC3339 formatting current UTC time is infallible");
        Ok(vec![SandboxMetric {
            timestamp,
            cpu_used_pct: 0.0,
            cpu_count: 1,
            mem_used: 0,
            mem_total: 512 * 1024 * 1024,
            disk_used: 0,
            disk_total: fs_capacity_hint(&sandbox.root),
        }])
    }
    async fn logs(&self, sandbox_id: &str) -> Result<SandboxLogs, BackendError> {
        let envd_adapter = {
            let state = self.state.lock().await;
            state
                .sandboxes
                .get(sandbox_id)
                .map(|sandbox| sandbox.envd_adapter.clone())
                .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?
        };
        let mut logs = envd_adapter.captured_process_logs().await;
        if logs.is_empty() {
            let timestamp = OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .expect("RFC3339 formatting current UTC time is infallible");
            logs.push(SandboxLogEntry {
                timestamp,
                level: LogLevel::Info,
                message: "host envd adapter running".to_owned(),
                fields: BTreeMap::new(),
            });
        }
        Ok(SandboxLogs { logs })
    }
    async fn apply_network(
        &self,
        _sandbox_id: &str,
        _policy: SandboxNetworkPolicy,
    ) -> Result<(), BackendError> {
        Err(BackendError::Runtime(
            "host runtime adapter does not enforce E2B network policy".to_owned(),
        ))
    }
    async fn port_target(&self, sandbox_id: &str, port: u16) -> Result<PortTarget, BackendError> {
        let state = self.state.lock().await;
        let sandbox = state
            .sandboxes
            .get(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        if sandbox.paused {
            return Err(BackendError::Runtime(format!(
                "sandbox `{sandbox_id}` is paused"
            )));
        }
        let target_port = match port {
            DEFAULT_ENVD_PORT => sandbox.envd_port,
            DEFAULT_CODE_INTERPRETER_PORT => sandbox.code_interpreter_port,
            DEFAULT_MCP_PORT => sandbox.mcp_port,
            _ => port,
        };
        Ok(PortTarget::Tcp {
            host: "127.0.0.1".to_owned(),
            port: target_port,
        })
    }
}
#[derive(Debug, Default)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct HostRuntimeState {
    next_sandbox: u64,
    next_pod: u64,
    #[allow(missing_docs)]
    pub sandboxes: BTreeMap<String, HostRuntimeSandbox>,
    pub(crate) pods: BTreeMap<String, HostRuntimePod>,
}
#[derive(Debug)]
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub struct HostRuntimeSandbox {
    pub(crate) root: PathBuf,
    envd_adapter: HostEnvdAdapter,
    start_children: Vec<Arc<Mutex<Child>>>,
    envd_port: u16,
    envd_task: JoinHandle<()>,
    code_interpreter_port: u16,
    code_interpreter_task: JoinHandle<()>,
    mcp_port: u16,
    mcp_task: JoinHandle<()>,
    paused: bool,
}
#[derive(Debug)]
pub(crate) struct HostRuntimePod {
    pub(crate) root: PathBuf,
    #[allow(missing_docs)]
    pub containers: BTreeMap<String, PathBuf>,
}
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
struct HostTemplateMetadata {
    #[allow(missing_docs)]
    pub envs: BTreeMap<String, String>,
    #[allow(missing_docs)]
    pub start_cmd: Option<String>,
    #[allow(missing_docs)]
    pub ready_cmd: Option<String>,
}
const HOST_TEMPLATE_METADATA_FILE: &str = ".firkin-template.json";
async fn serve_host_service_probe(
    listener: TcpListener,
    sandbox_id: String,
    service_name: &'static str,
    ok_paths: &'static [&'static str],
) -> Result<(), BoxError> {
    loop {
        let (stream, _) = listener.accept().await?;
        let sandbox_id = sandbox_id.clone();
        tokio::spawn(async move {
            let service = service_fn(move |request: Request<Incoming>| {
                let sandbox_id = sandbox_id.clone();
                async move {
                    let path = request.uri().path();
                    let status = if ok_paths.contains(&path) {
                        StatusCode::OK
                    } else {
                        StatusCode::NOT_FOUND
                    };
                    let body = if status == StatusCode::OK {
                        format!(
                            "{{\"status\":\"ok\",\"service\":\"{service_name}\",\"sandboxID\":\"{sandbox_id}\"}}"
                        )
                    } else {
                        "{\"error\":\"not found\"}".to_owned()
                    };
                    Ok::<_, Infallible>(
                        Response::builder()
                            .status(status)
                            .header(CONTENT_TYPE, "application/json")
                            .body(Full::new(Bytes::from(body)))
                            .expect("static code-interpreter probe response is valid"),
                    )
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}
/// Local E2B backend state plus runtime adapter.
#[derive(Clone, Debug)]
pub struct LocalRuntimeBackend<A> {
    pub(crate) adapter: A,
    #[allow(missing_docs)]
    pub sandboxes: LocalSandboxRegistry,
    pub(crate) pods: LocalPodRegistry,
    #[allow(missing_docs)]
    pub templates: LocalTemplateRegistry,
    #[allow(missing_docs)]
    pub volumes: LocalVolumeRegistry,
}
impl<A> LocalRuntimeBackend<A>
where
    A: RuntimeAdapter,
{
    /// Handle control-plane create routes without holding the backend registry
    /// lock across runtime adapter lifecycle work.
    ///
    /// Returns `None` for routes that should use the ordinary serialized
    /// dispatcher.
    pub async fn handle_concurrent_control_plane_create(
        backend: Arc<Mutex<Self>>,
        request: ControlPlaneRequest,
    ) -> Option<Result<ControlPlaneResponse, ControlPlaneError>> {
        let (path, _) = split_path_query(&request.path);
        match (request.method, path) {
            (ControlPlaneMethod::Post, "/sandboxes") => {
                let body = match decode_request_body::<SandboxCreateRequest>(&request) {
                    Ok(body) => body,
                    Err(error) => return Some(Err(error)),
                };
                Some(Self::create_with_detached_runtime_start(backend, body).await)
            }
            (ControlPlaneMethod::Post, "/sandboxes/followups") => {
                let body = match decode_request_body::<FollowupSandboxCreateRequest>(&request) {
                    Ok(body) => body,
                    Err(error) => return Some(Err(error)),
                };
                Some(Self::create_followup_with_detached_runtime_start(backend, body).await)
            }
            _ => None,
        }
    }

    async fn create_with_detached_runtime_start(
        backend: Arc<Mutex<Self>>,
        request: SandboxCreateRequest,
    ) -> Result<ControlPlaneResponse, ControlPlaneError> {
        let (adapter, prepared_template) = {
            let backend = backend.lock().await;
            let prepared_template = backend
                .templates
                .latest_prepared_template(&request.template_id)
                .or_else(|| {
                    backend
                        .sandboxes
                        .snapshot_prepared_template(&request.template_id)
                });
            if prepared_template.is_none() && !is_builtin_host_template(&request.template_id) {
                return Err(BackendError::NotFound(request.template_id.clone()).into());
            }
            (backend.adapter.clone(), prepared_template)
        };

        let runtime = adapter
            .start(StartSandboxRequest {
                create_request: request.clone(),
                prepared_template,
            })
            .await?;
        let sandbox_id = runtime.config.sandbox_id.clone();
        if let Some(policy) = effective_network_policy_request(&request)
            && let Err(error) = adapter.apply_network(&sandbox_id, policy).await
        {
            let _ = adapter.stop(&sandbox_id).await;
            return Err(error.into());
        }

        let connected = {
            let mut backend = backend.lock().await;
            backend.sandboxes.create(request, runtime.config)
        };
        match connected {
            Ok(connected) => json_response(200, &connected),
            Err(error) => {
                let _ = adapter.stop(&sandbox_id).await;
                Err(error.into())
            }
        }
    }

    async fn create_followup_with_detached_runtime_start(
        backend: Arc<Mutex<Self>>,
        request: FollowupSandboxCreateRequest,
    ) -> Result<ControlPlaneResponse, ControlPlaneError> {
        let (adapter, snapshot, create_request, prepared_template) = {
            let backend = backend.lock().await;
            let snapshot = backend.sandboxes.followup_snapshot(&request.snapshot_id)?;
            let mut create_request = request.create_request;
            create_request.template_id = snapshot.snapshot_id.clone();
            let prepared_template = Some(PreparedTemplate {
                template_id: snapshot.snapshot_id.clone(),
                build_id: snapshot.snapshot_id.clone(),
                artifact: snapshot.location.clone(),
                has_envd: true,
                artifact_integrity: snapshot.artifact_integrity.clone(),
            });
            (
                backend.adapter.clone(),
                snapshot,
                create_request,
                prepared_template,
            )
        };

        let runtime = adapter
            .start_followup(
                StartSandboxRequest {
                    create_request: create_request.clone(),
                    prepared_template,
                },
                snapshot,
            )
            .await?;
        let sandbox_id = runtime.config.sandbox_id.clone();
        if let Some(policy) = effective_network_policy_request(&create_request)
            && let Err(error) = adapter.apply_network(&sandbox_id, policy).await
        {
            let _ = adapter.stop(&sandbox_id).await;
            return Err(error.into());
        }

        let connected = {
            let mut backend = backend.lock().await;
            backend.sandboxes.create(create_request, runtime.config)
        };
        match connected {
            Ok(connected) => json_response(200, &connected),
            Err(error) => {
                let _ = adapter.stop(&sandbox_id).await;
                Err(error.into())
            }
        }
    }

    /// Construct a local runtime backend.
    #[must_use]
    pub fn new(adapter: A, now: impl Into<String>) -> Self {
        Self {
            adapter,
            sandboxes: LocalSandboxRegistry::new(),
            pods: LocalPodRegistry::new(),
            templates: LocalTemplateRegistry::new(now),
            volumes: LocalVolumeRegistry::new(),
        }
    }
    /// Construct a local runtime backend from persisted control-plane state.
    #[must_use]
    pub fn from_state(adapter: A, state: LocalRuntimeState) -> Self {
        Self {
            adapter,
            sandboxes: state.sandboxes,
            pods: state.pods,
            templates: state.templates,
            volumes: state.volumes,
        }
    }
    /// Return the runtime adapter.
    #[must_use]
    pub const fn adapter(&self) -> &A {
        &self.adapter
    }
    /// Return the sandbox registry.
    #[must_use]
    pub const fn sandboxes(&self) -> &LocalSandboxRegistry {
        &self.sandboxes
    }
    /// Return the pod registry.
    #[must_use]
    pub const fn pods(&self) -> &LocalPodRegistry {
        &self.pods
    }
    /// Return the template registry.
    #[must_use]
    pub const fn templates(&self) -> &LocalTemplateRegistry {
        &self.templates
    }
    /// Return the volume registry.
    #[must_use]
    pub const fn volumes(&self) -> &LocalVolumeRegistry {
        &self.volumes
    }
    /// Export the SDK-visible control-plane registries.
    #[must_use]
    pub fn export_state(&self) -> LocalRuntimeState {
        LocalRuntimeState {
            sandboxes: self.sandboxes.clone(),
            pods: self.pods.clone(),
            templates: self.templates.clone(),
            volumes: self.volumes.clone(),
        }
    }
    /// Encode the SDK-visible control-plane state as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns JSON serialization errors.
    pub fn export_state_json(&self) -> Result<Vec<u8>, LocalRuntimeStateStoreError> {
        Ok(serde_json::to_vec_pretty(
            &VersionedLocalRuntimeState::new(self.export_state()),
        )?)
    }
    /// Construct a local runtime backend from JSON control-plane state.
    ///
    /// # Errors
    ///
    /// Returns JSON decode errors.
    pub fn from_state_json(adapter: A, bytes: &[u8]) -> Result<Self, LocalRuntimeStateStoreError> {
        let envelope = decode_versioned_local_runtime_state(bytes)?;
        let state = envelope.into_state()?;
        Ok(Self::from_state(adapter, state))
    }
    /// Save SDK-visible control-plane state to a JSON file.
    ///
    /// # Errors
    ///
    /// Returns filesystem or JSON serialization errors.
    pub fn save_state_json(
        &self,
        path: impl AsRef<Path>,
    ) -> Result<(), LocalRuntimeStateStoreError> {
        let bytes = self.export_state_json()?;
        atomic_save_state_json(path.as_ref(), &bytes)?;
        Ok(())
    }
    /// Load SDK-visible control-plane state from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns filesystem or JSON decode errors.
    pub fn load_state_json(
        adapter: A,
        path: impl AsRef<Path>,
    ) -> Result<Self, LocalRuntimeStateStoreError> {
        let bytes = std::fs::read(path)?;
        Self::from_state_json(adapter, &bytes)
    }
    /// Run runtime preflight.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    pub async fn preflight(&self) -> Result<RuntimeCapabilitySet, BackendError> {
        self.adapter.preflight().await
    }
    /// Request and prepare a template build.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    pub async fn request_template_build(
        &mut self,
        request: TemplateBuildRequest,
    ) -> Result<TemplateBuildRequestInfo, BackendError> {
        let requested = self.templates.request_build(request.clone());
        let prepared = self.adapter.prepare_template(request).await?;
        let prepared = PreparedTemplate {
            template_id: requested.template_id.clone(),
            build_id: requested.build_id.clone(),
            artifact: prepared.artifact,
            has_envd: prepared.has_envd,
            artifact_integrity: prepared.artifact_integrity,
        };
        self.templates.start_build(
            &requested.template_id,
            &requested.build_id,
            TemplateBuildStart::default(),
        )?;
        self.templates
            .set_prepared_template(&requested.template_id, prepared)?;
        self.templates.set_build_status(
            &requested.template_id,
            &requested.build_id,
            TemplateBuildStatus::Ready,
        )?;
        Ok(requested)
    }
    async fn start_template_build(
        &mut self,
        template_id: &str,
        build_id: &str,
        start: TemplateBuildStart,
    ) -> Result<(), ControlPlaneError> {
        let uploaded_files = self
            .templates
            .uploaded_files_for_build(template_id, &start)?;
        self.templates
            .start_build(template_id, build_id, start.clone())?;
        match self
            .adapter
            .build_template(RuntimeTemplateBuild {
                template_id: template_id.to_owned(),
                build_id: build_id.to_owned(),
                start,
                uploaded_files,
            })
            .await
        {
            Ok(prepared) => {
                self.templates
                    .set_prepared_template(template_id, prepared)?;
                self.templates.set_build_status(
                    template_id,
                    build_id,
                    TemplateBuildStatus::Ready,
                )?;
                Ok(())
            }
            Err(error) => {
                self.templates.set_build_status(
                    template_id,
                    build_id,
                    TemplateBuildStatus::Error,
                )?;
                Err(ControlPlaneError::Backend(error))
            }
        }
    }
    /// Create and register a sandbox through the runtime adapter.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors or registry errors.
    pub async fn create(
        &mut self,
        request: SandboxCreateRequest,
    ) -> Result<ConnectedSandbox, BackendError> {
        let prepared_template = self
            .templates
            .latest_prepared_template(&request.template_id)
            .or_else(|| {
                self.sandboxes
                    .snapshot_prepared_template(&request.template_id)
            });
        if prepared_template.is_none() && !is_builtin_host_template(&request.template_id) {
            return Err(BackendError::NotFound(request.template_id));
        }
        let runtime = self
            .adapter
            .start(StartSandboxRequest {
                create_request: request.clone(),
                prepared_template,
            })
            .await?;
        let sandbox_id = runtime.config.sandbox_id.clone();
        let connected = match self.sandboxes.create(request.clone(), runtime.config) {
            Ok(connected) => connected,
            Err(error) => {
                let _ = self.adapter.stop(&sandbox_id).await;
                return Err(error);
            }
        };
        let network_result = if let Some(policy) = effective_network_policy_request(&request) {
            self.adapter
                .apply_network(&connected.sandbox_id, policy)
                .await
        } else {
            Ok(())
        };
        if let Err(error) = network_result {
            self.sandboxes.delete(&connected.sandbox_id);
            let _ = self.adapter.stop(&connected.sandbox_id).await;
            return Err(error);
        }
        Ok(connected)
    }
    /// Create and register a follow-up sandbox through the runtime adapter.
    ///
    /// # Errors
    ///
    /// Returns missing snapshot errors, runtime adapter errors, or registry
    /// errors.
    pub async fn create_followup(
        &mut self,
        request: FollowupSandboxCreateRequest,
    ) -> Result<ConnectedSandbox, BackendError> {
        let snapshot = self.sandboxes.followup_snapshot(&request.snapshot_id)?;
        let mut create_request = request.create_request;
        create_request.template_id = snapshot.snapshot_id.clone();
        let prepared_template = Some(PreparedTemplate {
            template_id: snapshot.snapshot_id.clone(),
            build_id: snapshot.snapshot_id.clone(),
            artifact: snapshot.location.clone(),
            has_envd: true,
            artifact_integrity: snapshot.artifact_integrity.clone(),
        });
        let runtime = self
            .adapter
            .start_followup(
                StartSandboxRequest {
                    create_request: create_request.clone(),
                    prepared_template,
                },
                snapshot,
            )
            .await?;
        let sandbox_id = runtime.config.sandbox_id.clone();
        let connected = match self
            .sandboxes
            .create(create_request.clone(), runtime.config)
        {
            Ok(connected) => connected,
            Err(error) => {
                let _ = self.adapter.stop(&sandbox_id).await;
                return Err(error);
            }
        };
        let network_result = if let Some(policy) = effective_network_policy_request(&create_request)
        {
            self.adapter
                .apply_network(&connected.sandbox_id, policy)
                .await
        } else {
            Ok(())
        };
        if let Err(error) = network_result {
            self.sandboxes.delete(&connected.sandbox_id);
            let _ = self.adapter.stop(&connected.sandbox_id).await;
            return Err(error);
        }
        Ok(connected)
    }
    /// Create and register a product pod through the runtime adapter.
    ///
    /// # Errors
    ///
    /// Returns validation, template lookup, runtime adapter, or registry
    /// errors.
    pub async fn create_pod(&mut self, request: PodCreateRequest) -> Result<PodInfo, BackendError> {
        request.validate().map_err(BackendError::Runtime)?;
        if let Some(pod_id) = request.pod_id.as_deref()
            && self.pods.get(pod_id).is_ok()
        {
            return Err(BackendError::AlreadyExists(pod_id.to_owned()));
        }
        let prepared_templates = self.pod_prepared_templates(&request.containers)?;
        let runtime = self
            .adapter
            .start_pod(StartPodRequest {
                create_request: request.clone(),
                prepared_templates,
            })
            .await?;
        let pod_id = runtime.config.pod_id.clone();
        match self.pods.create(request, runtime) {
            Ok(info) => Ok(info),
            Err(error) => {
                let _ = self.adapter.stop_pod(&pod_id).await;
                Err(error)
            }
        }
    }
    /// Delete a product pod through the runtime adapter.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    pub async fn delete_pod(&mut self, pod_id: &str) -> Result<bool, BackendError> {
        self.adapter.stop_pod(pod_id).await?;
        Ok(self.pods.delete(pod_id))
    }
    /// Add a container to an existing product pod.
    ///
    /// # Errors
    ///
    /// Returns pod lookup, validation, template lookup, runtime adapter, or
    /// registry errors.
    pub async fn add_pod_container(
        &mut self,
        pod_id: &str,
        request: PodContainerCreateRequest,
    ) -> Result<PodContainerInfo, BackendError> {
        let pod = self.pods.get(pod_id)?.clone();
        validate_pod_container_request(&pod.empty_dirs, &request).map_err(BackendError::Runtime)?;
        if pod
            .containers
            .iter()
            .any(|container| container.name == request.name)
        {
            return Err(BackendError::AlreadyExists(request.name));
        }
        self.pod_prepared_templates(std::slice::from_ref(&request))?;
        let runtime_container = self
            .adapter
            .add_pod_container(pod_id, request.clone())
            .await?;
        match self.pods.add_container(pod_id, runtime_container.clone()) {
            Ok(container) => Ok(container),
            Err(error) => {
                let _ = self
                    .adapter
                    .remove_pod_container(pod_id, &runtime_container.name)
                    .await;
                Err(error)
            }
        }
    }
    /// Remove a container from an existing product pod.
    ///
    /// # Errors
    ///
    /// Returns pod/container lookup or runtime adapter errors.
    pub async fn delete_pod_container(
        &mut self,
        pod_id: &str,
        container_name: &str,
    ) -> Result<(), BackendError> {
        let pod = self.pods.get(pod_id)?;
        if !pod
            .containers
            .iter()
            .any(|container| container.name == container_name)
        {
            return Err(BackendError::NotFound(container_name.to_owned()));
        }
        self.adapter
            .remove_pod_container(pod_id, container_name)
            .await?;
        self.pods.delete_container(pod_id, container_name)
    }
    /// Wait for a product pod container and collect output.
    ///
    /// # Errors
    ///
    /// Returns pod/container lookup, runtime adapter, or registry errors.
    pub async fn wait_pod_container(
        &mut self,
        pod_id: &str,
        container_name: &str,
    ) -> Result<PodContainerOutput, BackendError> {
        let pod = self.pods.get(pod_id)?;
        if !pod
            .containers
            .iter()
            .any(|container| container.name == container_name)
        {
            return Err(BackendError::NotFound(container_name.to_owned()));
        }
        let output = self
            .adapter
            .wait_pod_container(pod_id, container_name)
            .await?;
        self.pods.delete_container(pod_id, container_name)?;
        Ok(output)
    }
    /// Delete a sandbox through the runtime adapter.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    pub async fn delete(&mut self, sandbox_id: &str) -> Result<bool, BackendError> {
        self.adapter.stop(sandbox_id).await?;
        Ok(self.sandboxes.delete(sandbox_id))
    }
    /// Pause a sandbox through the runtime adapter.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors or registry errors.
    pub async fn pause(&mut self, sandbox_id: &str) -> Result<bool, BackendError> {
        self.adapter.pause(sandbox_id).await?;
        self.sandboxes.pause(sandbox_id)
    }
    /// Apply timeout expiration policy to due running sandboxes.
    ///
    /// `now` must use the same sortable RFC3339 form as `SandboxInfo::end_at`.
    /// Expired sandboxes with `autoPause: true` are paused through the runtime
    /// adapter; all others are stopped and removed.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors or registry errors.
    pub async fn expire_due_sandboxes(
        &mut self,
        now: &str,
    ) -> Result<Vec<SandboxExpiration>, BackendError> {
        let due = self.sandboxes.due_running_sandboxes(now);
        let mut expired = Vec::with_capacity(due.len());
        for sandbox_id in due {
            if self.sandboxes.auto_pause_enabled(&sandbox_id)? {
                self.adapter.pause(&sandbox_id).await?;
                self.sandboxes.pause(&sandbox_id)?;
                expired.push(SandboxExpiration {
                    sandbox_id,
                    action: SandboxExpirationAction::Paused,
                });
            } else {
                self.adapter.stop(&sandbox_id).await?;
                self.sandboxes.delete(&sandbox_id);
                expired.push(SandboxExpiration {
                    sandbox_id,
                    action: SandboxExpirationAction::Deleted,
                });
            }
        }
        Ok(expired)
    }
    /// Connect or resume a sandbox through the runtime adapter.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors or registry errors.
    pub async fn connect(
        &mut self,
        sandbox_id: &str,
        request: ConnectRequest,
    ) -> Result<ConnectedSandbox, BackendError> {
        self.adapter
            .resume(PausedSandbox {
                sandbox_id: sandbox_id.to_owned(),
                snapshot_id: None,
            })
            .await?;
        self.sandboxes.connect(sandbox_id, request)
    }
    /// Create a runtime snapshot and register E2B snapshot metadata.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors or registry errors.
    pub async fn create_snapshot(
        &mut self,
        sandbox_id: &str,
        request: CreateSnapshotRequest,
    ) -> Result<SnapshotInfo, BackendError> {
        let snapshot = self
            .adapter
            .snapshot(sandbox_id, request.name.clone())
            .await?;
        self.sandboxes
            .create_snapshot(sandbox_id, request, snapshot)
    }
    /// Delete a runtime snapshot and its registry metadata.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors or [`BackendError::NotFound`] when the
    /// snapshot is absent.
    pub async fn delete_snapshot(&mut self, snapshot_id: &str) -> Result<bool, BackendError> {
        if self
            .sandboxes
            .snapshot_prepared_template(snapshot_id)
            .is_none()
        {
            return Err(BackendError::NotFound(snapshot_id.to_owned()));
        }
        self.adapter.delete_snapshot(snapshot_id).await?;
        Ok(self.sandboxes.delete_snapshot(snapshot_id))
    }
    fn pod_prepared_templates(
        &self,
        containers: &[PodContainerCreateRequest],
    ) -> Result<BTreeMap<String, Option<PreparedTemplate>>, BackendError> {
        let mut templates = BTreeMap::new();
        for container in containers {
            if templates.contains_key(&container.template_id) {
                continue;
            }
            let prepared = self
                .templates
                .latest_prepared_template(&container.template_id)
                .or_else(|| {
                    self.sandboxes
                        .snapshot_prepared_template(&container.template_id)
                });
            if prepared.is_none() && !is_builtin_host_template(&container.template_id) {
                return Err(BackendError::NotFound(container.template_id.clone()));
            }
            templates.insert(container.template_id.clone(), prepared);
        }
        Ok(templates)
    }
    /// Refresh metrics from the runtime adapter and store the latest sample.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors or registry errors.
    pub async fn refresh_metrics(
        &mut self,
        sandbox_id: &str,
    ) -> Result<Vec<SandboxMetric>, BackendError> {
        let metrics = self.adapter.metrics(sandbox_id).await?;
        if let Some(metric) = metrics.last().cloned() {
            self.sandboxes.set_metric(sandbox_id, metric)?;
        }
        Ok(metrics)
    }
    /// Refresh logs from the runtime adapter and store them.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors or registry errors.
    pub async fn refresh_logs(&mut self, sandbox_id: &str) -> Result<SandboxLogs, BackendError> {
        let logs = self.adapter.logs(sandbox_id).await?;
        for log in logs.logs.iter().cloned() {
            self.sandboxes.push_log(sandbox_id, log)?;
        }
        Ok(logs)
    }
    /// Return proxy target for a sandbox port.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    pub async fn port_target(
        &self,
        sandbox_id: &str,
        port: u16,
    ) -> Result<PortTarget, BackendError> {
        self.adapter.port_target(sandbox_id, port).await
    }
    /// Open a runtime-owned stream for a sandbox port target.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    pub async fn connect_port_target(
        &self,
        sandbox_id: &str,
        target: PortTarget,
    ) -> Result<PortProxyStream, BackendError> {
        self.adapter.connect_port_target(sandbox_id, target).await
    }
    /// Dispatch one SDK-shaped control-plane request.
    ///
    /// # Errors
    ///
    /// Returns route, JSON, registry, or runtime adapter errors.
    pub async fn handle_control_plane(
        &mut self,
        request: ControlPlaneRequest,
    ) -> Result<ControlPlaneResponse, ControlPlaneError> {
        let (path, query) = split_path_query(&request.path);
        let segments = path_segments(path)?;
        let segment_refs = segments.iter().map(String::as_str).collect::<Vec<_>>();
        let segment_refs = segment_refs.as_slice();
        if is_pod_route(segment_refs) {
            return self.handle_pod_control_plane(&request, segment_refs).await;
        }
        if is_sandbox_route(segment_refs) {
            return self
                .handle_sandbox_control_plane(&request, query, segment_refs)
                .await;
        }
        if is_template_route(segment_refs) {
            return self
                .handle_template_control_plane(&request, segment_refs)
                .await;
        }
        if is_volume_route(segment_refs) {
            return self.handle_volume_control_plane(&request, segment_refs);
        }
        if segment_refs.is_empty() {
            return Err(ControlPlaneError::NotFound("/".to_owned()));
        }
        Err(ControlPlaneError::NotFound(path.to_owned()))
    }
    async fn handle_pod_control_plane(
        &mut self,
        request: &ControlPlaneRequest,
        segments: &[&str],
    ) -> Result<ControlPlaneResponse, ControlPlaneError> {
        match (request.method, segments) {
            (ControlPlaneMethod::Post, ["pods"]) => {
                let body = decode_request_body::<PodCreateRequest>(request)?;
                body.validate().map_err(ControlPlaneError::BadRequest)?;
                json_response(200, &self.create_pod(body).await?)
            }
            (ControlPlaneMethod::Get, ["pods"]) => json_response(200, &self.pods.list()),
            (ControlPlaneMethod::Get, ["pods", pod_id]) => {
                json_response(200, self.pods.get(pod_id)?)
            }
            (ControlPlaneMethod::Delete, ["pods", pod_id]) => {
                if self.delete_pod(pod_id).await? {
                    Ok(ControlPlaneResponse::empty(204))
                } else {
                    Err(ControlPlaneError::NotFound((*pod_id).to_owned()))
                }
            }
            (ControlPlaneMethod::Post, ["pods", pod_id, "containers"]) => {
                let body = decode_request_body::<PodContainerCreateRequest>(request)?;
                json_response(200, &self.add_pod_container(pod_id, body).await?)
            }
            (ControlPlaneMethod::Post, ["pods", pod_id, "containers", container_name, "wait"]) => {
                json_response(200, &self.wait_pod_container(pod_id, container_name).await?)
            }
            (ControlPlaneMethod::Delete, ["pods", pod_id, "containers", container_name]) => {
                self.delete_pod_container(pod_id, container_name).await?;
                Ok(ControlPlaneResponse::empty(204))
            }
            _ => Err(ControlPlaneError::NotFound(request.path.clone())),
        }
    }
    async fn handle_sandbox_control_plane(
        &mut self,
        request: &ControlPlaneRequest,
        query: Option<&str>,
        segments: &[&str],
    ) -> Result<ControlPlaneResponse, ControlPlaneError> {
        match (request.method, segments) {
            (ControlPlaneMethod::Post, ["sandboxes"]) => {
                let body = decode_request_body::<SandboxCreateRequest>(request)?;
                json_response(200, &self.create(body).await?)
            }
            (ControlPlaneMethod::Post, ["sandboxes", "followups"]) => {
                let body = decode_request_body::<FollowupSandboxCreateRequest>(request)?;
                json_response(200, &self.create_followup(body).await?)
            }
            (ControlPlaneMethod::Get, ["v2", "sandboxes"]) => {
                json_response(200, &self.sandboxes.list())
            }
            (ControlPlaneMethod::Get, ["sandboxes", "metrics"]) => {
                let ids = query_value(query, "sandbox_ids")
                    .ok_or_else(|| ControlPlaneError::BadRequest("missing sandbox_ids".to_owned()))?
                    .split(',')
                    .filter(|id| !id.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                json_response(200, &self.sandboxes.metrics_many(&ids)?)
            }
            (ControlPlaneMethod::Get, ["snapshots"]) => {
                let sandbox_id = query_value(query, "sandboxID");
                json_response(200, &self.sandboxes.list_snapshots(sandbox_id))
            }
            (ControlPlaneMethod::Get, ["sandboxes", sandbox_id]) => {
                json_response(200, self.sandboxes.get(sandbox_id)?)
            }
            (ControlPlaneMethod::Delete, ["sandboxes", sandbox_id]) => {
                if self.delete(sandbox_id).await? {
                    Ok(ControlPlaneResponse::empty(204))
                } else {
                    Err(ControlPlaneError::NotFound((*sandbox_id).to_owned()))
                }
            }
            (ControlPlaneMethod::Post, ["sandboxes", sandbox_id, "connect" | "resume"]) => {
                let body = decode_request_body::<ConnectRequest>(request)?;
                json_response(200, &self.connect(sandbox_id, body).await?)
            }
            (ControlPlaneMethod::Post, ["sandboxes", sandbox_id, "pause"]) => {
                if self.pause(sandbox_id).await? {
                    Ok(ControlPlaneResponse::empty(204))
                } else {
                    Err(ControlPlaneError::Backend(BackendError::AlreadyExists(
                        (*sandbox_id).to_owned(),
                    )))
                }
            }
            (ControlPlaneMethod::Post, ["sandboxes", sandbox_id, "timeout"]) => {
                let body = decode_request_body::<TimeoutRequest>(request)?;
                let end_at = self.sandboxes.get(sandbox_id)?.end_at.clone();
                self.sandboxes.set_timeout(sandbox_id, body, end_at)?;
                Ok(ControlPlaneResponse::empty(204))
            }
            (ControlPlaneMethod::Post, ["sandboxes", sandbox_id, "refreshes"]) => {
                let body = decode_request_body::<RefreshRequest>(request)?;
                let end_at = self.sandboxes.get(sandbox_id)?.end_at.clone();
                self.sandboxes.refresh(sandbox_id, body, end_at)?;
                Ok(ControlPlaneResponse::empty(204))
            }
            (ControlPlaneMethod::Get, ["sandboxes", sandbox_id, "metrics"]) => {
                json_response(200, &self.refresh_metrics(sandbox_id).await?)
            }
            (ControlPlaneMethod::Get, ["v2", "sandboxes", sandbox_id, "logs"]) => {
                json_response(200, &self.refresh_logs(sandbox_id).await?)
            }
            (ControlPlaneMethod::Post, ["sandboxes", sandbox_id, "snapshots"]) => {
                let body = decode_request_body::<CreateSnapshotRequest>(request)?;
                json_response(200, &self.create_snapshot(sandbox_id, body).await?)
            }
            _ => Err(ControlPlaneError::NotFound(request.path.clone())),
        }
    }
    async fn handle_template_control_plane(
        &mut self,
        request: &ControlPlaneRequest,
        segments: &[&str],
    ) -> Result<ControlPlaneResponse, ControlPlaneError> {
        match (request.method, segments) {
            (ControlPlaneMethod::Delete, ["templates", "tags"]) => {
                let body = decode_request_body::<RemoveTemplateTags>(request)?;
                self.templates.remove_tags(body)?;
                Ok(ControlPlaneResponse::empty(204))
            }
            (ControlPlaneMethod::Delete, ["templates", id]) => {
                if self.templates.delete(id) {
                    Ok(ControlPlaneResponse::empty(204))
                } else if self.sandboxes.snapshot_prepared_template(id).is_some() {
                    self.adapter.delete_snapshot(id).await?;
                    self.sandboxes.delete_snapshot(id);
                    Ok(ControlPlaneResponse::empty(204))
                } else {
                    Err(ControlPlaneError::NotFound((*id).to_owned()))
                }
            }
            (ControlPlaneMethod::Post, ["v3", "templates"]) => {
                let body = decode_request_body::<TemplateBuildRequest>(request)?;
                json_response(200, &self.request_template_build(body).await?)
            }
            (ControlPlaneMethod::Get, ["templates"]) => json_response(200, &self.templates.list()),
            (ControlPlaneMethod::Get, ["templates", template_id]) => {
                json_response(200, &self.templates.get(template_id)?)
            }
            (ControlPlaneMethod::Patch, ["v2", "templates", template_id]) => {
                let body = decode_request_body::<TemplateUpdateRequest>(request)?;
                json_response(200, &self.templates.update(template_id, body)?)
            }
            (ControlPlaneMethod::Get, ["templates", template_id, "files", hash]) => json_response(
                200,
                &self
                    .templates
                    .file_upload(template_id, hash, request.origin.as_deref())?,
            ),
            (ControlPlaneMethod::Put, ["templates", template_id, "files", hash, "upload"]) => {
                let body = request.body.clone().unwrap_or_default();
                self.templates.upload_file(template_id, hash, body)?;
                Ok(ControlPlaneResponse::empty(204))
            }
            (ControlPlaneMethod::Post, ["v2", "templates", template_id, "builds", build_id]) => {
                let body = decode_request_body::<TemplateBuildStart>(request)?;
                self.start_template_build(template_id, build_id, body)
                    .await?;
                Ok(ControlPlaneResponse::empty(204))
            }
            (ControlPlaneMethod::Get, ["templates", template_id, "builds", build_id, "status"]) => {
                json_response(200, &self.templates.build_status(template_id, build_id)?)
            }
            (ControlPlaneMethod::Get, ["templates", template_id, "builds", build_id, "logs"]) => {
                json_response(200, &self.templates.build_logs(template_id, build_id)?)
            }
            (ControlPlaneMethod::Get, ["templates", "aliases", name]) => self
                .templates
                .alias(name)
                .map(|alias| json_response(200, &alias))
                .transpose()?
                .ok_or_else(|| ControlPlaneError::NotFound((*name).to_owned())),
            (ControlPlaneMethod::Post, ["templates", "tags"]) => {
                let body = decode_request_body::<AssignTemplateTags>(request)?;
                json_response(200, &self.templates.assign_tags(body)?)
            }
            (ControlPlaneMethod::Get, ["templates", template_id, "tags"]) => {
                json_response(200, &self.templates.tags(template_id)?)
            }
            _ => Err(ControlPlaneError::NotFound(request.path.clone())),
        }
    }
    fn handle_volume_control_plane(
        &mut self,
        request: &ControlPlaneRequest,
        segments: &[&str],
    ) -> Result<ControlPlaneResponse, ControlPlaneError> {
        let (_, query) = split_path_query(&request.path);
        match (request.method, segments) {
            (ControlPlaneMethod::Post, ["volumes"]) => {
                let body = decode_request_body::<VolumeCreateRequest>(request)?;
                json_response(200, &self.volumes.create(body)?)
            }
            (ControlPlaneMethod::Get, ["volumes"]) => json_response(200, &self.volumes.list()),
            (ControlPlaneMethod::Get, ["volumes", volume_id]) => {
                json_response(200, &self.volumes.get(volume_id)?)
            }
            (ControlPlaneMethod::Delete, ["volumes", volume_id]) => {
                if self.volumes.delete(volume_id) {
                    Ok(ControlPlaneResponse::empty(204))
                } else {
                    Err(ControlPlaneError::NotFound((*volume_id).to_owned()))
                }
            }
            (ControlPlaneMethod::Get, ["volumecontent", volume_id, "dir"]) => {
                let path = query_value_decoded(query, "path")?;
                json_response(200, &self.volumes.list_dir(volume_id, &path)?)
            }
            (ControlPlaneMethod::Post, ["volumecontent", volume_id, "dir"]) => {
                let path = query_value_decoded(query, "path")?;
                let opts = volume_write_options(query);
                json_response(200, &self.volumes.make_dir(volume_id, &path, opts)?)
            }
            (ControlPlaneMethod::Get, ["volumecontent", volume_id, "path"]) => {
                let path = query_value_decoded(query, "path")?;
                json_response(200, &self.volumes.path_info(volume_id, &path)?)
            }
            (ControlPlaneMethod::Patch, ["volumecontent", volume_id, "path"]) => {
                let path = query_value_decoded(query, "path")?;
                let body = decode_request_body::<VolumeMetadataRequest>(request)?;
                json_response(200, &self.volumes.update_metadata(volume_id, &path, body)?)
            }
            (ControlPlaneMethod::Delete, ["volumecontent", volume_id, "path"]) => {
                let path = query_value_decoded(query, "path")?;
                self.volumes.remove_path(volume_id, &path)?;
                Ok(ControlPlaneResponse::empty(204))
            }
            (ControlPlaneMethod::Get, ["volumecontent", volume_id, "file"]) => {
                let path = query_value_decoded(query, "path")?;
                let body = self.volumes.read_file(volume_id, &path)?;
                Ok(ControlPlaneResponse {
                    status: 200,
                    headers: BTreeMap::from([(
                        "content-type".to_owned(),
                        "application/octet-stream".to_owned(),
                    )]),
                    body,
                })
            }
            (ControlPlaneMethod::Put, ["volumecontent", volume_id, "file"]) => {
                let path = query_value_decoded(query, "path")?;
                let opts = volume_write_options(query);
                json_response(
                    200,
                    &self.volumes.write_file(
                        volume_id,
                        &path,
                        request.body.clone().unwrap_or_default(),
                        opts,
                    )?,
                )
            }
            _ => Err(ControlPlaneError::NotFound(request.path.clone())),
        }
    }
}
fn effective_network_policy_request(
    request: &SandboxCreateRequest,
) -> Option<SandboxNetworkPolicy> {
    let policy = request.network.clone().unwrap_or_else(|| {
        SandboxNetworkPolicy::new(request.allow_internet_access, [], [], None, None)
    });
    let policy = if request.allow_internet_access.is_some()
        && request.allow_internet_access != policy.allow_internet_access()
    {
        SandboxNetworkPolicy::new(
            request.allow_internet_access,
            policy.allow_out().iter().cloned(),
            policy.deny_out().iter().cloned(),
            policy.allow_public_traffic(),
            policy.mask_request_host().map(ToOwned::to_owned),
        )
    } else {
        policy
    };
    policy.requires_policy_engine().then_some(policy)
}
fn is_builtin_host_template(template_id: &str) -> bool {
    template_id == SandboxCreateRequest::default().template_id || template_id == "mcp-gateway"
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct VersionedLocalRuntimeState {
    schema: String,
    version: u32,
    #[allow(missing_docs)]
    pub state: LocalRuntimeState,
}
impl VersionedLocalRuntimeState {
    #[allow(missing_docs)]
    pub fn new(state: LocalRuntimeState) -> Self {
        Self {
            schema: LOCAL_RUNTIME_STATE_SCHEMA.to_owned(),
            version: LOCAL_RUNTIME_STATE_VERSION,
            state,
        }
    }
    fn into_state(self) -> Result<LocalRuntimeState, LocalRuntimeStateStoreError> {
        if self.schema != LOCAL_RUNTIME_STATE_SCHEMA {
            return Err(LocalRuntimeStateStoreError::UnsupportedSchema(self.schema));
        }
        if self.version != LOCAL_RUNTIME_STATE_VERSION {
            return Err(LocalRuntimeStateStoreError::UnsupportedVersion(
                self.version,
            ));
        }
        Ok(self.state)
    }
}
fn decode_versioned_local_runtime_state(
    bytes: &[u8],
) -> Result<VersionedLocalRuntimeState, LocalRuntimeStateStoreError> {
    let value: JsonValue = serde_json::from_slice(bytes)?;
    if value.get("schema").is_some()
        || value.get("version").is_some()
        || value.get("state").is_some()
    {
        Ok(serde_json::from_value(value)?)
    } else {
        Ok(VersionedLocalRuntimeState::new(serde_json::from_value(
            value,
        )?))
    }
}
struct StateFileLock {
    #[allow(missing_docs)]
    pub path: PathBuf,
}
impl StateFileLock {
    fn acquire(path: &Path) -> Result<Self, std::io::Error> {
        let lock_path = state_sidecar_path(path, "lock");
        for _ in 0..50 {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    writeln!(file, "pid={}", std::process::id())?;
                    file.sync_all()?;
                    return Ok(Self { path: lock_path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            format!("state lock {} is busy", lock_path.display()),
        ))
    }
}
impl Drop for StateFileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn atomic_save_state_json(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let _lock = StateFileLock::acquire(path)?;
    let tmp_path = state_sidecar_path(path, &format!("tmp-{}-{}", std::process::id(), now_nanos()));
    let write_result = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    write_result
}
fn state_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    path.with_file_name(format!("{file_name}.{suffix}"))
}
fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
}
fn split_path_query(path: &str) -> (&str, Option<&str>) {
    path.split_once('?')
        .map_or((path, None), |(path, query)| (path, Some(query)))
}
fn query_value<'a>(query: Option<&'a str>, key: &str) -> Option<&'a str> {
    query?.split('&').find_map(|pair| {
        let (candidate, value) = pair.split_once('=')?;
        (candidate == key).then_some(value)
    })
}
fn query_value_decoded(query: Option<&str>, key: &str) -> Result<String, ControlPlaneError> {
    let value = query_value(query, key)
        .ok_or_else(|| ControlPlaneError::BadRequest(format!("missing {key}")))?;
    decode_query_component(value)
}
fn query_u32(query: Option<&str>, key: &str) -> Option<u32> {
    query_value(query, key)?.parse().ok()
}
fn query_bool(query: Option<&str>, key: &str) -> Option<bool> {
    query_value(query, key).and_then(|value| match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}
fn volume_write_options(query: Option<&str>) -> VolumeWriteOptions {
    VolumeWriteOptions {
        uid: query_u32(query, "uid"),
        gid: query_u32(query, "gid"),
        mode: query_u32(query, "mode"),
        force: query_bool(query, "force"),
    }
}
fn decode_request_body<T>(request: &ControlPlaneRequest) -> Result<T, ControlPlaneError>
where
    T: DeserializeOwned,
{
    let body = request
        .body
        .as_deref()
        .ok_or_else(|| ControlPlaneError::BadRequest("missing JSON body".to_owned()))?;
    Ok(serde_json::from_slice(body)?)
}
fn is_sandbox_route(segments: &[&str]) -> bool {
    matches!(
        segments,
        ["sandboxes", ..] | ["snapshots"] | ["v2", "sandboxes", ..]
    )
}
fn is_pod_route(segments: &[&str]) -> bool {
    matches!(segments, ["pods", ..])
}
fn is_template_route(segments: &[&str]) -> bool {
    matches!(
        segments,
        ["templates", ..] | ["v2", "templates", ..] | ["v3", "templates"]
    )
}
fn is_volume_route(segments: &[&str]) -> bool {
    matches!(segments, ["volumes" | "volumecontent", ..])
}
fn path_segments(path: &str) -> Result<Vec<String>, ControlPlaneError> {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(decode_path_segment)
        .collect()
}
fn decode_path_segment(segment: &str) -> Result<String, ControlPlaneError> {
    let mut bytes = Vec::with_capacity(segment.len());
    let mut input = segment.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }
        let high = input.next().ok_or_else(|| {
            ControlPlaneError::BadRequest(format!("invalid percent escape in `{segment}`"))
        })?;
        let low = input.next().ok_or_else(|| {
            ControlPlaneError::BadRequest(format!("invalid percent escape in `{segment}`"))
        })?;
        let high = hex_value(high).ok_or_else(|| {
            ControlPlaneError::BadRequest(format!("invalid percent escape in `{segment}`"))
        })?;
        let low = hex_value(low).ok_or_else(|| {
            ControlPlaneError::BadRequest(format!("invalid percent escape in `{segment}`"))
        })?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes)
        .map_err(|_| ControlPlaneError::BadRequest(format!("invalid UTF-8 in `{segment}`")))
}
fn decode_query_component(component: &str) -> Result<String, ControlPlaneError> {
    decode_path_segment(&component.replace('+', " "))
}
const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
fn apply_copy_archive_to_artifact(
    archive: &[u8],
    source: &str,
    destination: &str,
    artifact_root: &Path,
) -> Result<(), BackendError> {
    let unpack_root = artifact_root.join(".copy-unpack");
    if unpack_root.exists() {
        std::fs::remove_dir_all(&unpack_root)
            .map_err(|error| BackendError::Runtime(format!("clear COPY unpack dir: {error}")))?;
    }
    std::fs::create_dir_all(&unpack_root)
        .map_err(|error| BackendError::Runtime(format!("create COPY unpack dir: {error}")))?;
    tar::Archive::new(std::io::Cursor::new(archive))
        .unpack(&unpack_root)
        .map_err(|error| BackendError::Runtime(format!("unpack COPY archive: {error}")))?;
    let source_path = rooted_child_path(&unpack_root, source, "COPY source")?;
    let destination_path = rooted_child_path(artifact_root, destination, "COPY destination")?;
    copy_host_path(&source_path, &destination_path)?;
    std::fs::remove_dir_all(&unpack_root)
        .map_err(|error| BackendError::Runtime(format!("remove COPY unpack dir: {error}")))?;
    Ok(())
}
fn copy_host_path(source: &Path, destination: &Path) -> Result<(), BackendError> {
    let metadata = std::fs::symlink_metadata(source)
        .map_err(|error| BackendError::Runtime(format!("stat copy source: {error}")))?;
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(source)
            .map_err(|error| BackendError::Runtime(format!("read symlink copy source: {error}")))?;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                BackendError::Runtime(format!("create symlink destination parent: {error}"))
            })?;
        }
        if destination.exists() {
            std::fs::remove_file(destination).map_err(|error| {
                BackendError::Runtime(format!("remove existing symlink destination: {error}"))
            })?;
        }
        symlink_path(&target, destination)
            .map_err(|error| BackendError::Runtime(format!("copy symlink: {error}")))?;
    } else if metadata.is_dir() {
        std::fs::create_dir_all(destination)
            .map_err(|error| BackendError::Runtime(format!("create copy destination: {error}")))?;
        for entry in std::fs::read_dir(source)
            .map_err(|error| BackendError::Runtime(format!("read copy source dir: {error}")))?
        {
            let entry = entry
                .map_err(|error| BackendError::Runtime(format!("read copy entry: {error}")))?;
            copy_host_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                BackendError::Runtime(format!("create copy destination parent: {error}"))
            })?;
        }
        std::fs::copy(source, destination)
            .map_err(|error| BackendError::Runtime(format!("copy file: {error}")))?;
    }
    Ok(())
}
async fn copy_host_directory_contents(
    source: &Path,
    destination: &Path,
) -> Result<(), BackendError> {
    tokio::fs::create_dir_all(destination)
        .await
        .map_err(|error| BackendError::Runtime(format!("create sandbox template root: {error}")))?;
    copy_host_path(source, destination)
}
async fn write_host_template_metadata(
    artifact: &Path,
    metadata: &HostTemplateMetadata,
) -> Result<(), BackendError> {
    let bytes = serde_json::to_vec(metadata).map_err(|error| {
        BackendError::Runtime(format!("encode host template metadata: {error}"))
    })?;
    tokio::fs::write(artifact.join(HOST_TEMPLATE_METADATA_FILE), bytes)
        .await
        .map_err(|error| BackendError::Runtime(format!("write host template metadata: {error}")))
}
async fn read_host_template_metadata(
    artifact: &Path,
) -> Result<HostTemplateMetadata, BackendError> {
    let path = artifact.join(HOST_TEMPLATE_METADATA_FILE);
    match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            BackendError::Runtime(format!("decode host template metadata: {error}"))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(HostTemplateMetadata::default())
        }
        Err(error) => Err(BackendError::Runtime(format!(
            "read host template metadata: {error}"
        ))),
    }
}
async fn run_host_template_command(
    command: &str,
    workdir: &Path,
    envs: &BTreeMap<String, String>,
    user: Option<&str>,
) -> Result<(), BackendError> {
    tokio::fs::create_dir_all(workdir)
        .await
        .map_err(|error| BackendError::Runtime(format!("create template command cwd: {error}")))?;
    let mut process = Command::new("/bin/bash");
    process.args(["-l", "-c", command]).current_dir(workdir);
    for (key, value) in envs {
        process.env(key, value);
    }
    if let Some(user) = user {
        process.env("USER", user).env("LOGNAME", user);
    }
    let output = process
        .output()
        .await
        .map_err(|error| BackendError::Runtime(format!("run template command: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    let exit_code = output.status.code().unwrap_or(128);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(BackendError::Runtime(format!(
        "template RUN exited {exit_code}: {stderr}"
    )))
}
async fn spawn_host_template_command(
    command: &str,
    workdir: &Path,
    envs: &BTreeMap<String, String>,
    user: Option<&str>,
) -> Result<Arc<Mutex<Child>>, BackendError> {
    tokio::fs::create_dir_all(workdir)
        .await
        .map_err(|error| BackendError::Runtime(format!("create template command cwd: {error}")))?;
    let mut process = Command::new("/bin/bash");
    process
        .args(["-l", "-c", command])
        .current_dir(workdir)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in envs {
        process.env(key, value);
    }
    if let Some(user) = user {
        process.env("USER", user).env("LOGNAME", user);
    }
    let child = process
        .spawn()
        .map_err(|error| BackendError::Runtime(format!("spawn template start command: {error}")))?;
    Ok(Arc::new(Mutex::new(child)))
}
async fn kill_host_template_children(children: &[Arc<Mutex<Child>>]) {
    for child in children {
        let mut child = child.lock().await;
        let _ = child.start_kill();
    }
}
async fn stop_host_runtime_sandbox_services(sandbox: &HostRuntimeSandbox) {
    sandbox.envd_task.abort();
    sandbox.code_interpreter_task.abort();
    sandbox.mcp_task.abort();
    kill_host_template_children(&sandbox.start_children).await;
}
fn rooted_child_path(root: &Path, path: &str, label: &str) -> Result<PathBuf, BackendError> {
    let mut child = root.to_path_buf();
    let mut saw_component = false;
    for component in Path::new(path).components() {
        match component {
            Component::Prefix(_) | Component::ParentDir => {
                return Err(BackendError::Runtime(format!(
                    "{label} `{path}` escapes its root"
                )));
            }
            Component::RootDir | Component::CurDir => {}
            Component::Normal(part) => {
                saw_component = true;
                child.push(part);
            }
        }
    }
    if saw_component {
        Ok(child)
    } else {
        Err(BackendError::Runtime(format!(
            "{label} `{path}` must not target the root"
        )))
    }
}
async fn remove_existing_mount_path(path: &Path) -> Result<(), BackendError> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(BackendError::Runtime(format!(
                "stat existing mount path: {error}"
            )));
        }
    };
    let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    };
    result.map_err(|error| BackendError::Runtime(format!("clear existing mount path: {error}")))
}
#[cfg(unix)]
fn symlink_dir(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}
#[cfg(windows)]
fn symlink_dir(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, destination)
}
#[cfg(unix)]
fn symlink_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}
#[cfg(windows)]
fn symlink_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, destination)
    } else {
        std::os::windows::fs::symlink_file(source, destination)
    }
}
fn fs_capacity_hint(_root: &Path) -> u64 {
    0
}
