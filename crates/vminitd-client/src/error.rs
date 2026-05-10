//! error — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_types::VsockPort;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Crate-local result type.
pub type Result<T> = std::result::Result<T, VminitdError>;
/// vminitd client errors.
#[derive(Debug, ThisError)]
pub enum VminitdError {
    /// A `SandboxContext` RPC returned a tonic status.
    #[error("{family:?} RPC failed: {source}")]
    Rpc {
        /// Family of the RPC that failed.
        family: RpcFamily,
        /// Original tonic status.
        source: Box<tonic::Status>,
    },
    /// Failed to encode an OCI runtime spec as JSON for vminitd.
    #[error("failed to encode OCI runtime spec: {source}")]
    EncodeSpec {
        /// Original serde error.
        source: serde_json::Error,
    },
    /// Failed to connect to vminitd over vsock.
    #[error("failed to connect to vminitd over vsock port {port:?}: {source}")]
    Connect {
        /// vminitd vsock port.
        port: VsockPort,
        /// Original tonic transport error.
        source: tonic::transport::Error,
    },
    /// A `Copy` RPC completed with a guest-side transfer error.
    #[error("Copy RPC failed: {reason}")]
    Copy {
        /// Guest-side error detail.
        reason: String,
    },
    /// A `Copy` RPC returned an unknown response status.
    #[error("Copy RPC returned unknown response status {status}")]
    UnknownCopyStatus {
        /// Raw protobuf status value.
        status: i32,
    },
}
/// Coarse RPC family for actionable error reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcFamily {
    /// `Mkdir`.
    Mkdir,
    /// `Mount`.
    Mount,
    /// `SetupEmulator`.
    SetupEmulator,
    /// `IpLinkSet`.
    IpLinkSet,
    /// `IpAddrAdd`.
    IpAddrAdd,
    /// `IpRouteAddDefault`.
    IpRouteAddDefault,
    /// `ConfigureDns`.
    ConfigureDns,
    /// `Copy`.
    Copy,
    /// `ProxyVsock`.
    ProxyVsock,
    /// `StopVsockProxy`.
    StopVsockProxy,
    /// `RemovePath`.
    RemovePath,
    /// `Fstrim`.
    Fstrim,
    /// `ApplyOciLayer`.
    ApplyOciLayer,
    /// `FilesystemUsage`.
    FilesystemUsage,
}
