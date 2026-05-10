//! proxy — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::pb;
#[allow(unused_imports)]
use firkin_types::VsockPort;
#[allow(unused_imports)]
use std::path::Path;
/// Direction for a vminitd Unix-socket/vsock proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketProxyDirection {
    /// Guest Unix socket dials back to a host vsock listener.
    Into,
    /// Host dials a VM vsock port and vminitd connects to a guest Unix socket.
    OutOf,
}
impl SocketProxyDirection {
    pub(crate) fn proto(self) -> pb::proxy_vsock_request::Action {
        match self {
            Self::Into => pb::proxy_vsock_request::Action::Into,
            Self::OutOf => pb::proxy_vsock_request::Action::OutOf,
        }
    }
}
/// Typed builder for a vminitd `ProxyVsock` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketProxy {
    pub(crate) id: String,
    pub(crate) vsock_port: VsockPort,
    guest_path: String,
    permissions: Option<u32>,
    pub(crate) direction: SocketProxyDirection,
}
impl SocketProxy {
    /// Construct a host-socket-into-guest proxy request.
    #[must_use]
    pub fn into_guest(
        id: impl Into<String>,
        vsock_port: VsockPort,
        guest_path: impl AsRef<Path>,
    ) -> Self {
        Self {
            id: id.into(),
            vsock_port,
            guest_path: guest_path.as_ref().display().to_string(),
            permissions: None,
            direction: SocketProxyDirection::Into,
        }
    }
    /// Construct a guest-socket-out-to-host proxy request.
    #[must_use]
    pub fn out_of_guest(
        id: impl Into<String>,
        vsock_port: VsockPort,
        guest_path: impl AsRef<Path>,
    ) -> Self {
        Self {
            id: id.into(),
            vsock_port,
            guest_path: guest_path.as_ref().display().to_string(),
            permissions: None,
            direction: SocketProxyDirection::OutOf,
        }
    }
    /// Set the guest-side Unix socket permissions.
    #[must_use]
    pub const fn permissions(mut self, permissions: Option<u32>) -> Self {
        self.permissions = permissions;
        self
    }
    /// Convert into the generated protobuf request.
    #[must_use]
    pub fn into_request(self) -> pb::ProxyVsockRequest {
        pb::ProxyVsockRequest {
            id: self.id,
            vsock_port: self.vsock_port.get(),
            guest_path: self.guest_path,
            guest_socket_permissions: self.permissions,
            action: self.direction.proto() as i32,
        }
    }
}
/// Build a vminitd `StopVsockProxy` request.
#[must_use]
pub fn stop_socket_proxy_request(id: impl Into<String>) -> pb::StopVsockProxyRequest {
    pb::StopVsockProxyRequest { id: id.into() }
}
