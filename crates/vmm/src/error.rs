//! error — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
pub use firkin_types::{BlockDeviceId, Size, VirtiofsTag, VmId, VsockPort};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Crate-local result type.
pub type Result<T> = std::result::Result<T, Error>;
/// Errors produced by VM configuration and runtime operations.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum Error {
    /// The VM configuration is invalid before calling into Virtualization.framework.
    #[error("invalid VM configuration: {reason}")]
    InvalidConfig {
        /// Specific validation reason.
        reason: String,
    },
    /// Nested virtualization was requested on an unsupported host.
    #[error("nested virtualization is not supported on this host")]
    NestedVirtNotSupported,
    /// Tombstone for unmapped Virtualization.framework failures.
    #[error("unclassified Virtualization.framework error: {reason}")]
    UnclassifiedVz {
        /// Original framework error text.
        reason: String,
    },
    /// Virtualization.framework rejected VM startup.
    #[error("failed to start VM: {reason}")]
    Start {
        /// Original framework error text.
        reason: String,
    },
    /// Virtualization.framework rejected VM stop.
    #[error("failed to stop VM: {reason}")]
    Stop {
        /// Original framework error text.
        reason: String,
    },
    /// Virtualization.framework rejected VM pause.
    #[error("failed to pause VM: {reason}")]
    Pause {
        /// Original framework error text.
        reason: String,
    },
    /// Virtualization.framework rejected VM resume.
    #[error("failed to resume VM: {reason}")]
    Resume {
        /// Original framework error text.
        reason: String,
    },
    /// Virtualization.framework rejected VM snapshot or restore.
    #[cfg(feature = "snapshot")]
    #[error("failed to {operation} VM snapshot: {reason}")]
    Snapshot {
        /// Snapshot operation being attempted.
        operation: &'static str,
        /// Original framework error text.
        reason: String,
    },
    /// Virtualization.framework rejected a vsock dial.
    #[error("failed to dial vsock port {port:?}: {reason}")]
    Dial {
        /// Guest-side port.
        port: VsockPort,
        /// Dial failure text.
        reason: String,
    },
    /// Virtualization.framework rejected a vsock listener.
    #[error("failed to listen on vsock port {port:?}: {reason}")]
    Listen {
        /// Host-side port.
        port: VsockPort,
        /// Listen failure text.
        reason: String,
    },
    /// The public operation needs a live runtime backend, but this handle has none.
    #[error("Virtualization.framework runtime backend is not available for this VM handle")]
    RuntimeNotImplemented,
    /// The caller selected a port reserved for library or vminitd use.
    #[error("vsock port {port:?} is reserved: {reason}")]
    ReservedPort {
        /// Rejected port.
        port: VsockPort,
        /// Reservation reason.
        reason: &'static str,
    },
}
pub(crate) fn invalid_config<T>(reason: impl Into<String>) -> Result<T> {
    Err(Error::InvalidConfig {
        reason: reason.into(),
    })
}
