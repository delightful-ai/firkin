//! error — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::pod;
#[allow(unused_imports)]
use firkin_types::{InvalidContainerId, InvalidProcessId};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Crate-local result type.
pub type Result<T> = std::result::Result<T, Error>;
/// Core container-surface errors.
#[derive(Debug, PartialEq, Eq, ThisError)]
pub enum Error {
    /// The container ID failed validation.
    #[error(transparent)]
    InvalidContainerId(#[from] InvalidContainerId),
    /// The process ID failed validation.
    #[error(transparent)]
    InvalidProcessId(#[from] InvalidProcessId),
    /// Runtime orchestration is not implemented in this slice yet.
    #[error("container runtime orchestration is not implemented")]
    RuntimeNotImplemented,
    /// A command argument cannot be represented in the OCI runtime spec.
    #[error("command argument at index {index} is not valid UTF-8")]
    InvalidCommandArgument {
        /// Argument index.
        index: usize,
    },
    /// An environment variable cannot be represented in the OCI runtime spec.
    #[error("environment variable at index {index} is not valid UTF-8")]
    InvalidEnvironmentVariable {
        /// Environment variable index.
        index: usize,
    },
    /// The working directory cannot be represented in the OCI runtime spec.
    #[error("working directory is not valid UTF-8")]
    InvalidWorkingDirectory,
    /// A guest path is invalid for use as a Firkin-managed path.
    #[error("invalid guest path `{path}`: {reason}")]
    InvalidGuestPath {
        /// Rejected guest path.
        path: String,
        /// Specific validation reason.
        reason: &'static str,
    },
    /// A pod specification failed validation before runtime work began.
    #[error(transparent)]
    PodValidation(#[from] pod::PodValidationError),
    /// Rootfs staging failed before VM boot.
    #[error("rootfs assembly failed while {operation}: {reason}")]
    RootfsAssembly {
        /// Operation being attempted.
        operation: &'static str,
        /// Lower-level error detail.
        reason: String,
    },
    /// Runtime artifact preparation failed before VM boot.
    #[error("runtime artifact preparation failed while {operation}: {reason}")]
    RuntimeArtifact {
        /// Operation being attempted.
        operation: &'static str,
        /// Lower-level error detail.
        reason: String,
    },
    /// Runtime orchestration failed after VM boot.
    #[error("runtime operation failed while {operation}: {reason}")]
    RuntimeOperation {
        /// Operation being attempted.
        operation: &'static str,
        /// Lower-level error detail.
        reason: String,
    },
    /// The configured writable layer is not backed by a block filesystem.
    #[error("writable_layer must be a block device (got {kind})")]
    InvalidWritableLayer {
        /// Mount type that was rejected.
        kind: String,
    },
    /// The configured raw seccomp policy is not valid JSON.
    #[error("invalid seccomp policy: {reason}")]
    InvalidSeccomp {
        /// Lower-level error detail.
        reason: String,
    },
}
