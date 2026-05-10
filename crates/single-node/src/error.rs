/// Result type for single-node runtime operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Runtime-oriented single-node operation errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The request cannot be handled by the runtime.
    #[error("invalid single-node runtime request: {0}")]
    InvalidRequest(String),
    /// The requested runtime capability is not implemented.
    #[error("unsupported single-node runtime capability: {0}")]
    UnsupportedCapability(String),
    /// Capacity admission rejected the operation.
    #[error("single-node runtime capacity rejected operation: {0}")]
    CapacityRejected(String),
    /// The host is below the required disk-space floor.
    #[error("single-node runtime disk pressure: {0}")]
    DiskPressure(String),
    /// The requested sandbox does not exist.
    #[error("single-node sandbox not found: {0}")]
    SandboxNotFound(String),
    /// The requested snapshot does not exist.
    #[error("single-node snapshot not found: {0}")]
    SnapshotNotFound(String),
    /// The requested runtime resource already exists.
    #[error("single-node runtime conflict: {0}")]
    Conflict(String),
    /// Template snapshot construction failed.
    #[error("single-node template build failed: {0}")]
    TemplateBuildFailed(String),
    /// Runtime launch failed.
    #[error("single-node runtime launch failed: {0}")]
    RuntimeLaunchFailed(String),
    /// Runtime command execution failed.
    #[error("single-node runtime command failed: {0}")]
    RuntimeCommandFailed(String),
    /// Durable runtime state persistence failed.
    #[error("single-node runtime state persistence failed: {0}")]
    StatePersistenceFailed(String),
    /// Runtime protocol or proxy operation failed.
    #[error("single-node runtime protocol failure: {0}")]
    ProtocolFailure(String),
}
