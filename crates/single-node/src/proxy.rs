//! Domain proxy and port routing for single-node sessions.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use firkin_types::SandboxNetworkPolicy;
use tokio::sync::Mutex;
use {
    firkin_e2b_contract::{
        BackendError, FollowupSnapshot, PausedSandbox, PortTarget, RuntimeAdapter,
        RuntimeCapabilitySet, RuntimeSandbox, SnapshotRef, StartSandboxRequest,
    },
    firkin_e2b_wire::{SandboxLogs, SandboxMetric, TemplateBuildRequest},
};

/// Registry that maps sandbox ports to host-side runtime targets.
#[derive(Clone, Debug, Default)]
pub struct PortRegistry {
    targets: Arc<Mutex<HashMap<(String, u16), PortTarget>>>,
}

impl PortRegistry {
    /// Route a sandbox TCP port to a host socket address.
    pub async fn route_tcp(&self, sandbox_id: &str, port: u16, target: SocketAddr) {
        self.targets.lock().await.insert(
            (sandbox_id.to_owned(), port),
            PortTarget::Tcp {
                host: target.ip().to_string(),
                port: target.port(),
            },
        );
    }

    /// Remove all port routes owned by `sandbox_id`.
    pub async fn remove_sandbox(&self, sandbox_id: &str) {
        self.targets
            .lock()
            .await
            .retain(|(target_sandbox_id, _), _| target_sandbox_id != sandbox_id);
    }

    /// Return the host-side target for a sandbox port.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when no route exists.
    pub async fn target(&self, sandbox_id: &str, port: u16) -> Result<PortTarget, BackendError> {
        self.targets
            .lock()
            .await
            .get(&(sandbox_id.to_owned(), port))
            .cloned()
            .ok_or_else(|| {
                BackendError::NotFound(format!("sandbox {sandbox_id} port {port} not found"))
            })
    }
}

/// Runtime adapter used by the E2B domain proxy sidecar.
#[derive(Clone, Debug)]
pub struct DomainProxyAdapter {
    ports: PortRegistry,
}

impl DomainProxyAdapter {
    /// Construct a domain proxy adapter backed by a port registry.
    #[must_use]
    pub const fn new(ports: PortRegistry) -> Self {
        Self { ports }
    }
}

#[async_trait]
impl RuntimeAdapter for DomainProxyAdapter {
    async fn preflight(&self) -> Result<RuntimeCapabilitySet, BackendError> {
        Ok(RuntimeCapabilitySet {
            backend: "apple-vz".to_owned(),
            supported: vec!["domain-host-proxy".to_owned()],
            unsupported: vec![],
        })
    }

    async fn prepare_template(
        &self,
        _request: TemplateBuildRequest,
    ) -> Result<firkin_e2b_contract::PreparedTemplate, BackendError> {
        Err(proxy_not_wired("prepare_template"))
    }

    async fn start(&self, _request: StartSandboxRequest) -> Result<RuntimeSandbox, BackendError> {
        Err(proxy_not_wired("start"))
    }

    async fn start_followup(
        &self,
        _request: StartSandboxRequest,
        _snapshot: FollowupSnapshot,
    ) -> Result<RuntimeSandbox, BackendError> {
        Err(proxy_not_wired("start_followup"))
    }

    async fn stop(&self, _sandbox_id: &str) -> Result<(), BackendError> {
        Err(proxy_not_wired("stop"))
    }

    async fn pause(&self, sandbox_id: &str) -> Result<PausedSandbox, BackendError> {
        Err(proxy_not_wired_for_sandbox("pause", sandbox_id))
    }

    async fn resume(&self, _paused: PausedSandbox) -> Result<RuntimeSandbox, BackendError> {
        Err(proxy_not_wired("resume"))
    }

    async fn snapshot(
        &self,
        sandbox_id: &str,
        _name: Option<String>,
    ) -> Result<SnapshotRef, BackendError> {
        Err(proxy_not_wired_for_sandbox("snapshot", sandbox_id))
    }

    async fn metrics(&self, sandbox_id: &str) -> Result<Vec<SandboxMetric>, BackendError> {
        Err(proxy_not_wired_for_sandbox("metrics", sandbox_id))
    }

    async fn logs(&self, sandbox_id: &str) -> Result<SandboxLogs, BackendError> {
        Err(proxy_not_wired_for_sandbox("logs", sandbox_id))
    }

    async fn apply_network(
        &self,
        sandbox_id: &str,
        _policy: SandboxNetworkPolicy,
    ) -> Result<(), BackendError> {
        Err(proxy_not_wired_for_sandbox("apply_network", sandbox_id))
    }

    async fn port_target(&self, sandbox_id: &str, port: u16) -> Result<PortTarget, BackendError> {
        self.ports.target(sandbox_id, port).await
    }
}

fn proxy_not_wired(operation: &'static str) -> BackendError {
    BackendError::Runtime(format!(
        "single-node proxy adapter operation `{operation}` is not wired"
    ))
}

fn proxy_not_wired_for_sandbox(operation: &'static str, sandbox_id: &str) -> BackendError {
    BackendError::Runtime(format!(
        "single-node proxy adapter operation `{operation}` is not wired for sandbox `{sandbox_id}`"
    ))
}
