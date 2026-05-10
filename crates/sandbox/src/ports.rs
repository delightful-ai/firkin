use std::sync::Arc;

use crate::backend::BoxBackend;
use crate::capability::CapabilityName;
use crate::error::{InvalidSpec, InvalidSpecReason, Result};
use crate::ids::SandboxId;
use crate::sandbox::unsupported;

#[derive(Clone)]
pub struct PortClient {
    backend: BoxBackend,
    sandbox_id: SandboxId,
}

impl PortClient {
    pub(crate) fn new(backend: BoxBackend, sandbox_id: SandboxId) -> Self {
        Self {
            backend,
            sandbox_id,
        }
    }

    pub async fn list(&self) -> Result<Vec<PortBinding>> {
        let Some(control) = self.backend.ports() else {
            return Err(unsupported("list ports", CapabilityName::PortsConnect));
        };
        control.list_ports(&self.sandbox_id).await
    }

    pub async fn connect(&self, port: GuestPort) -> Result<PortTarget> {
        let Some(control) = self.backend.ports() else {
            return Err(unsupported("connect port", CapabilityName::PortsConnect));
        };
        control.connect_port(&self.sandbox_id, port).await
    }

    pub async fn expose(&self, port: GuestPort, spec: PortExposure) -> Result<PortBinding> {
        let Some(control) = self.backend.ports() else {
            return Err(unsupported("expose port", CapabilityName::PortsExpose));
        };
        control.expose_port(&self.sandbox_id, port, spec).await
    }

    pub async fn unexpose(&self, binding: PortBinding) -> Result<()> {
        let Some(control) = self.backend.ports() else {
            return Err(unsupported("unexpose port", CapabilityName::PortsExpose));
        };
        control.unexpose_port(&self.sandbox_id, binding).await
    }

    pub async fn domain_proxy(&self, spec: DomainProxySpec) -> Result<DomainProxy> {
        let Some(control) = self.backend.ports() else {
            return Err(unsupported(
                "domain proxy",
                CapabilityName::PortsDomainProxy,
            ));
        };
        control.domain_proxy(&self.sandbox_id, spec).await
    }
}

impl From<(Arc<dyn crate::backend::SandboxBackend>, SandboxId)> for PortClient {
    fn from((backend, sandbox_id): (Arc<dyn crate::backend::SandboxBackend>, SandboxId)) -> Self {
        Self::new(backend, sandbox_id)
    }
}

pub type Port = GuestPort;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GuestPort(u16);

impl GuestPort {
    pub fn new(port: u16) -> Result<Self> {
        validate_port(port).map(Self)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostPort(u16);

impl HostPort {
    pub fn new(port: u16) -> Result<Self> {
        validate_port(port).map(Self)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

fn validate_port(port: u16) -> Result<u16> {
    if port == 0 {
        return Err(InvalidSpec::new("validate port", InvalidSpecReason::InvalidPort(port)).into());
    }
    Ok(port)
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortProtocol {
    Tcp,
    Udp,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortBinding {
    pub guest: GuestPort,
    pub host: Option<HostPort>,
    pub protocol: PortProtocol,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortTarget {
    Tcp { host: String, port: HostPort },
    Unix { path: String },
    Vsock { cid: u32, port: u32 },
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortExposure {
    pub protocol: PortProtocol,
    pub host_port: Option<HostPort>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainProxy {
    pub domain: String,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainProxySpec {
    pub domain: String,
}
