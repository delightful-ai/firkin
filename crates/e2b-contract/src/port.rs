//! port — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
/// Default code-interpreter service port used by E2B templates.
#[allow(private_interfaces)]
pub const DEFAULT_CODE_INTERPRETER_PORT: u16 = 49999;
/// Default MCP gateway port used by the local E2B Rust SDK.
#[allow(private_interfaces)]
pub const DEFAULT_MCP_PORT: u16 = 50005;
/// Runtime port-forward target.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum PortTarget {
    /// TCP host and port target.
    Tcp {
        /// Target host.
        host: String,
        /// Target port.
        port: u16,
    },
    /// Vsock cid and port target.
    Vsock {
        /// Guest cid.
        cid: u32,
        /// Guest port.
        port: u32,
    },
    /// Unix-domain socket target.
    UnixSocket {
        /// Socket path.
        path: String,
    },
}
/// Byte stream opened by a runtime adapter for proxy forwarding.
pub trait PortProxyIo: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T> PortProxyIo for T where T: AsyncRead + AsyncWrite + Send + Unpin {}
/// Boxed runtime-provided proxy stream.
#[allow(private_interfaces)]
pub type PortProxyStream = Box<dyn PortProxyIo>;
