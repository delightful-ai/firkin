//! connect — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Result, VminitdError};
use crate::pb::sandbox_context_client::SandboxContextClient;
#[allow(unused_imports)]
use firkin_types::VsockPort;
#[allow(unused_imports)]
use firkin_vsock::{VsockConnector, VsockStream};
#[allow(unused_imports)]
use std::future::Future;
#[allow(unused_imports)]
use tonic::transport::{Channel, Endpoint};
/// vminitd's fixed gRPC vsock port.
pub const VMINITD_PORT: VsockPort = VsockPort::new(1024);
/// Connect a tonic `SandboxContext` client through a VM-provided vsock dialer.
///
/// # Errors
///
/// Returns [`VminitdError::Connect`] when the dialer or tonic transport cannot
/// establish the channel.
pub async fn connect_with_dialer<D, F>(dialer: D) -> Result<SandboxContextClient<Channel>>
where
    D: Fn(VsockPort) -> F + Clone + Send + Sync + 'static,
    F: Future<Output = firkin_vsock::Result<VsockStream>> + Send + 'static,
{
    let channel = Endpoint::from_static("http://vminitd.vsock")
        .connect_with_connector(VsockConnector::new(VMINITD_PORT, dialer))
        .await
        .map_err(|source| VminitdError::Connect {
            port: VMINITD_PORT,
            source,
        })?;
    Ok(SandboxContextClient::new(channel))
}
