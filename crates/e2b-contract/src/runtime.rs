//! runtime — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::capability::RuntimeCapabilitySet;
#[allow(unused_imports)]
use crate::port::{PortProxyStream, PortTarget};
#[allow(unused_imports)]
use crate::template::{PreparedTemplate, PreparedTemplateArtifactIntegrity, RuntimeTemplateBuild};
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use firkin_e2b_wire::PodCreateRequest;
#[allow(unused_imports)]
use firkin_e2b_wire::SandboxCreateRequest;
#[allow(unused_imports)]
use firkin_e2b_wire::{
    PodContainerCreateRequest, SandboxLogs, SandboxMetric, TemplateBuildRequest,
};
#[allow(unused_imports)]
use firkin_e2b_wire::{PodContainerInfo, PodContainerOutput};
#[allow(unused_imports)]
use firkin_types::SandboxNetworkPolicy;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use tokio::net::{TcpStream, UnixStream};
/// Runtime-facing pod start request.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct StartPodRequest {
    /// Original product create request.
    pub create_request: PodCreateRequest,
    /// Prepared template sources keyed by template ID. Built-in templates have
    /// `None` and are resolved by the runtime adapter.
    pub prepared_templates: BTreeMap<String, Option<PreparedTemplate>>,
}
/// Runtime-owned pod configuration returned after start.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct PodRuntimeConfig {
    /// Runtime-assigned pod ID.
    pub pod_id: String,
    /// RFC3339 start timestamp.
    pub started_at: String,
    /// RFC3339 timeout/deadline timestamp.
    pub end_at: String,
}
/// Runtime-owned pod start result.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct RuntimePod {
    /// Runtime-visible pod configuration.
    pub config: PodRuntimeConfig,
    /// Containers started by the runtime.
    pub containers: Vec<PodContainerInfo>,
}
/// Sandbox start request passed to a runtime adapter.
#[derive(Clone, Debug, PartialEq)]
#[allow(private_interfaces)]
pub struct StartSandboxRequest {
    /// E2B create request.
    pub create_request: SandboxCreateRequest,
    /// Prepared template, if the backend has one.
    pub prepared_template: Option<PreparedTemplate>,
}
/// Runtime snapshot source used for follow-up sandbox creation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct FollowupSnapshot {
    /// Snapshot id.
    pub snapshot_id: String,
    /// Runtime-local snapshot path or URI.
    pub location: String,
    /// Expected integrity for the continuation snapshot artifact.
    pub artifact_integrity: Option<PreparedTemplateArtifactIntegrity>,
}
/// Runtime-owned sandbox start/resume result.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct RuntimeSandbox {
    /// SDK-visible runtime config.
    pub config: SandboxRuntimeConfig,
    /// Ports exposed through the local proxy.
    pub exposed_ports: Vec<u16>,
}
/// Runtime-owned paused sandbox reference.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct PausedSandbox {
    /// Sandbox id.
    pub sandbox_id: String,
    /// Runtime snapshot reference, when pause materialized one.
    pub snapshot_id: Option<String>,
}
/// Action taken for an expired sandbox.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum SandboxExpirationAction {
    /// Sandbox was paused because `autoPause` was enabled.
    Paused,
    /// Sandbox was stopped and removed because `autoPause` was disabled.
    Deleted,
}
/// Result of applying lifecycle expiration to one sandbox.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct SandboxExpiration {
    /// Sandbox id.
    pub sandbox_id: String,
    /// Action taken by the backend.
    pub action: SandboxExpirationAction,
}
/// Runtime-owned snapshot reference.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct SnapshotRef {
    /// Snapshot id.
    pub snapshot_id: String,
    /// Runtime-local snapshot path or URI.
    pub location: Option<String>,
    /// Expected integrity for the snapshot artifact.
    pub artifact_integrity: Option<PreparedTemplateArtifactIntegrity>,
}
/// Host runtime adapter contract.
#[async_trait]
#[allow(private_interfaces)]
pub trait RuntimeAdapter: Clone + Send + Sync + 'static {
    /// Run runtime-specific preflight.
    async fn preflight(&self) -> Result<RuntimeCapabilitySet, BackendError>;
    /// Prepare a template from an E2B build request.
    async fn prepare_template(
        &self,
        request: TemplateBuildRequest,
    ) -> Result<PreparedTemplate, BackendError>;
    /// Materialize a started template build.
    async fn build_template(
        &self,
        request: RuntimeTemplateBuild,
    ) -> Result<PreparedTemplate, BackendError> {
        Err(BackendError::Runtime(format!(
            "runtime adapter does not support template build materialization for `{}/{}`",
            request.template_id, request.build_id
        )))
    }
    /// Start a sandbox.
    async fn start(&self, request: StartSandboxRequest) -> Result<RuntimeSandbox, BackendError>;
    /// Start a follow-up sandbox from a continuation snapshot.
    async fn start_followup(
        &self,
        request: StartSandboxRequest,
        snapshot: FollowupSnapshot,
    ) -> Result<RuntimeSandbox, BackendError>;
    /// Start a product pod.
    async fn start_pod(&self, request: StartPodRequest) -> Result<RuntimePod, BackendError> {
        let pod_id = request
            .create_request
            .pod_id
            .as_deref()
            .unwrap_or("unassigned");
        Err(BackendError::Runtime(format!(
            "runtime adapter does not support product pods for `{pod_id}`"
        )))
    }
    /// Stop a product pod.
    async fn stop_pod(&self, pod_id: &str) -> Result<(), BackendError> {
        Err(BackendError::Runtime(format!(
            "runtime adapter does not support product pod stop for `{pod_id}`"
        )))
    }
    /// Add a container to a running product pod.
    async fn add_pod_container(
        &self,
        pod_id: &str,
        container: PodContainerCreateRequest,
    ) -> Result<PodContainerInfo, BackendError> {
        Err(BackendError::Runtime(format!(
            "runtime adapter does not support adding container `{}` to product pod `{pod_id}`",
            container.name
        )))
    }
    /// Remove a container from a running product pod.
    async fn remove_pod_container(
        &self,
        pod_id: &str,
        container_name: &str,
    ) -> Result<(), BackendError> {
        Err(BackendError::Runtime(format!(
            "runtime adapter does not support removing container `{container_name}` from product pod `{pod_id}`"
        )))
    }
    /// Wait for a product pod container and collect stdout/stderr.
    async fn wait_pod_container(
        &self,
        pod_id: &str,
        container_name: &str,
    ) -> Result<PodContainerOutput, BackendError> {
        Err(BackendError::Runtime(format!(
            "runtime adapter does not support waiting for container `{container_name}` in product pod `{pod_id}`"
        )))
    }
    /// Stop a sandbox.
    async fn stop(&self, sandbox_id: &str) -> Result<(), BackendError>;
    /// Pause a sandbox.
    async fn pause(&self, sandbox_id: &str) -> Result<PausedSandbox, BackendError>;
    /// Resume a paused sandbox.
    async fn resume(&self, paused: PausedSandbox) -> Result<RuntimeSandbox, BackendError>;
    /// Create a runtime snapshot.
    async fn snapshot(
        &self,
        sandbox_id: &str,
        name: Option<String>,
    ) -> Result<SnapshotRef, BackendError>;
    /// Delete a runtime snapshot artifact.
    async fn delete_snapshot(&self, snapshot_id: &str) -> Result<(), BackendError> {
        Err(BackendError::Runtime(format!(
            "runtime adapter does not support snapshot artifact deletion for `{snapshot_id}`"
        )))
    }
    /// Return runtime metrics.
    async fn metrics(&self, sandbox_id: &str) -> Result<Vec<SandboxMetric>, BackendError>;
    /// Return runtime logs.
    async fn logs(&self, sandbox_id: &str) -> Result<SandboxLogs, BackendError>;
    /// Apply a network policy.
    async fn apply_network(
        &self,
        sandbox_id: &str,
        policy: SandboxNetworkPolicy,
    ) -> Result<(), BackendError>;
    /// Return a proxy target for a sandbox port.
    async fn port_target(&self, sandbox_id: &str, port: u16) -> Result<PortTarget, BackendError>;
    /// Open a byte stream to a proxy target.
    async fn connect_port_target(
        &self,
        _sandbox_id: &str,
        target: PortTarget,
    ) -> Result<PortProxyStream, BackendError> {
        match target {
            PortTarget::Tcp { host, port } => {
                let stream = TcpStream::connect((host.as_str(), port))
                    .await
                    .map_err(|error| {
                        BackendError::Runtime(format!(
                            "failed to connect proxy target {host}:{port}: {error}"
                        ))
                    })?;
                stream.set_nodelay(true).map_err(|error| {
                    BackendError::Runtime(format!(
                        "failed to configure proxy target {host}:{port} TCP_NODELAY: {error}"
                    ))
                })?;
                Ok(Box::new(stream) as PortProxyStream)
            }
            PortTarget::UnixSocket { path } => UnixStream::connect(&path)
                .await
                .map(|stream| Box::new(stream) as PortProxyStream)
                .map_err(|error| {
                    BackendError::Runtime(format!(
                        "failed to connect proxy target unix socket {path}: {error}"
                    ))
                }),
            PortTarget::Vsock { cid, port } => Err(BackendError::Runtime(format!(
                "runtime adapter does not provide a vsock dialer for target {cid}:{port}"
            ))),
        }
    }
}
/// Runtime-provided facts needed when registering a newly started sandbox.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct SandboxRuntimeConfig {
    /// Sandbox id.
    pub sandbox_id: String,
    /// Local proxy domain returned to SDK callers.
    pub domain: String,
    /// envd version exposed to SDK callers.
    pub envd_version: String,
    /// Optional envd access token.
    pub envd_access_token: Option<String>,
    /// Optional traffic access token.
    pub traffic_access_token: Option<String>,
    /// RFC3339 start timestamp.
    pub started_at: String,
    /// RFC3339 timeout/deadline timestamp.
    pub end_at: String,
    /// vCPU count.
    pub cpu_count: u32,
    /// Memory size in MiB.
    pub memory_mb: u32,
}
/// E2B backend registry error.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[allow(private_interfaces)]
pub enum BackendError {
    /// Sandbox already exists.
    #[error("sandbox `{0}` already exists")]
    AlreadyExists(String),
    /// Sandbox or snapshot was not found.
    #[error("sandbox `{0}` was not found")]
    NotFound(String),
    /// Runtime adapter error.
    #[error("runtime adapter error: {0}")]
    Runtime(String),
}
