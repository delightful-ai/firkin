use crate::capability::{CapabilityName, CapabilityReason};
use crate::data_plane::{GuestArch, ReservedPort};
use crate::filesystem::SandboxPath;
use crate::ids::{BackendName, ProcessId, SandboxId, SnapshotId, TemplateId};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    UnsupportedCapability(#[from] UnsupportedCapability),
    #[error(transparent)]
    InvalidSpec(#[from] InvalidSpec),
    #[error(transparent)]
    NotFound(#[from] NotFound),
    #[error(transparent)]
    AlreadyExists(#[from] AlreadyExists),
    #[error(transparent)]
    CapacityRejected(#[from] CapacityRejected),
    #[error(transparent)]
    DeadlineExceeded(#[from] DeadlineExceeded),
    #[error(transparent)]
    TemplatePrepareFailure(#[from] TemplatePrepareFailure),
    #[error(transparent)]
    ProcessFailure(#[from] ProcessFailure),
    #[error(transparent)]
    FilesystemFailure(#[from] FilesystemFailure),
    #[error(transparent)]
    SnapshotIntegrityFailure(#[from] SnapshotIntegrityFailure),
    #[error(transparent)]
    PortFailure(#[from] PortFailure),
    #[error(transparent)]
    BackendFailure(#[from] BackendFailure),
    #[error(transparent)]
    IoFailure(#[from] IoFailure),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryClass {
    Retryable,
    NotRetryable,
    Unknown,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedCapability {
    pub operation: &'static str,
    pub capability: CapabilityName,
    pub reason: CapabilityReason,
}

impl UnsupportedCapability {
    pub fn new(
        operation: &'static str,
        capability: CapabilityName,
        reason: CapabilityReason,
    ) -> Self {
        Self {
            operation,
            capability,
            reason,
        }
    }
}

impl std::fmt::Display for UnsupportedCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "sandbox operation `{}` requires unsupported capability `{}`",
            self.operation, self.capability
        )
    }
}

impl std::error::Error for UnsupportedCapability {}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid sandbox spec while {operation}: {reason}")]
pub struct InvalidSpec {
    pub operation: &'static str,
    pub reason: InvalidSpecReason,
}

impl InvalidSpec {
    pub const fn new(operation: &'static str, reason: InvalidSpecReason) -> Self {
        Self { operation, reason }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvalidSpecReason {
    MissingBackend,
    MissingTemplate,
    MissingSnapshot,
    MissingSandbox,
    InvalidId(String),
    InvalidPath(String),
    InvalidPort(u16),
    InvalidTimeout,
    InvalidResources(String),
    InvalidCommand(String),
    InvalidDataPlane(String),
}

impl std::fmt::Display for InvalidSpecReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBackend => f.write_str("missing backend"),
            Self::MissingTemplate => f.write_str("missing template"),
            Self::MissingSnapshot => f.write_str("missing snapshot"),
            Self::MissingSandbox => f.write_str("missing sandbox"),
            Self::InvalidId(value) => write!(f, "invalid id `{value}`"),
            Self::InvalidPath(value) => write!(f, "invalid path `{value}`"),
            Self::InvalidPort(value) => write!(f, "invalid port `{value}`"),
            Self::InvalidTimeout => f.write_str("invalid timeout"),
            Self::InvalidResources(value) => write!(f, "invalid resources: {value}"),
            Self::InvalidCommand(value) => write!(f, "invalid command: {value}"),
            Self::InvalidDataPlane(value) => write!(f, "invalid data plane: {value}"),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("{resource} not found while {operation}: {id}")]
pub struct NotFound {
    pub operation: &'static str,
    pub resource: ResourceKind,
    pub id: String,
}

impl NotFound {
    pub fn new(operation: &'static str, resource: ResourceKind, id: impl Into<String>) -> Self {
        Self {
            operation,
            resource,
            id: id.into(),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("{resource} already exists while {operation}: {id}")]
pub struct AlreadyExists {
    pub operation: &'static str,
    pub resource: ResourceKind,
    pub id: String,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Runtime,
    Template,
    Sandbox,
    Process,
    Snapshot,
    WarmPool,
    Port,
    File,
}

impl std::fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Runtime => "runtime",
            Self::Template => "template",
            Self::Sandbox => "sandbox",
            Self::Process => "process",
            Self::Snapshot => "snapshot",
            Self::WarmPool => "warm-pool entry",
            Self::Port => "port",
            Self::File => "file",
        };
        f.write_str(value)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("capacity rejected by {backend} while {operation}: {reason}")]
pub struct CapacityRejected {
    pub operation: &'static str,
    pub backend: BackendName,
    pub reason: String,
    pub retry: RetryClass,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("deadline exceeded while {operation}")]
pub struct DeadlineExceeded {
    pub operation: &'static str,
    pub sandbox_id: Option<SandboxId>,
    pub retry: RetryClass,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum TemplatePrepareFailure {
    #[error("envd is missing from template source")]
    EnvdMissing { reference: Option<String> },
    #[error("envd architecture mismatch: expected {expected:?}, found {found:?}")]
    EnvdWrongArch {
        expected: GuestArch,
        found: GuestArch,
    },
    #[error("envd version mismatch: expected {expected}, found {found}")]
    EnvdVersionMismatch { expected: String, found: String },
    #[error("envd integrity mismatch: expected {expected_sha256}, found {found_sha256}")]
    EnvdIntegrityMismatch {
        expected_sha256: String,
        found_sha256: String,
    },
    #[error("envd health failed on {port}: {path} with status {status:?}")]
    EnvdHealthFailed {
        port: ReservedPort,
        path: String,
        status: Option<u16>,
    },
    #[error("envd init failed: {reason}")]
    EnvdInitFailed { reason: String },
    #[error("entrypoint is unsupported: {reason}")]
    EntrypointUnsupported { reason: String },
    #[error("entrypoint could not be wrapped")]
    EntrypointNotWrapped,
    #[error("requested port conflicts with reserved port {port}")]
    PortConflict { port: ReservedPort },
    #[error("default user `{user}` is missing")]
    DefaultUserMissing { user: String },
    #[error("default workdir `{path}` is missing")]
    DefaultWorkdirMissing { path: SandboxPath },
    #[error("image rootfs is read-only")]
    ReadOnlyRootfs,
    #[error("writable layer is required")]
    WritableLayerRequired,
    #[error("unsupported image config: {reason}")]
    UnsupportedImageConfig { reason: String },
    #[error("registry unavailable for {reference}")]
    RegistryUnavailable { reference: String },
    #[error("snapshot after prepare failed: {reason}")]
    SnapshotAfterPrepareFailed { reason: String },
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("process failure while {operation}: {reason}")]
pub struct ProcessFailure {
    pub operation: &'static str,
    pub sandbox_id: Option<SandboxId>,
    pub process_id: Option<ProcessId>,
    pub reason: String,
    pub retry: RetryClass,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("filesystem failure while {operation}: {path:?}: {reason}")]
pub struct FilesystemFailure {
    pub operation: &'static str,
    pub sandbox_id: Option<SandboxId>,
    pub path: Option<SandboxPath>,
    pub reason: String,
    pub retry: RetryClass,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("snapshot integrity failure while {operation}: {snapshot_id:?}: {reason}")]
pub struct SnapshotIntegrityFailure {
    pub operation: &'static str,
    pub snapshot_id: Option<SnapshotId>,
    pub reason: String,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("port failure while {operation}: {reason}")]
pub struct PortFailure {
    pub operation: &'static str,
    pub sandbox_id: Option<SandboxId>,
    pub reason: String,
    pub retry: RetryClass,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("backend {backend} failed while {operation}: {reason}")]
pub struct BackendFailure {
    pub operation: &'static str,
    pub backend: BackendName,
    pub reason: String,
    pub retry: RetryClass,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("io failure while {operation}: {resource}: {reason}")]
pub struct IoFailure {
    pub operation: &'static str,
    pub resource: String,
    pub reason: String,
    pub retry: RetryClass,
}

impl From<crate::ids::InvalidId> for Error {
    fn from(error: crate::ids::InvalidId) -> Self {
        Self::InvalidSpec(InvalidSpec::new(
            "validate id",
            InvalidSpecReason::InvalidId(error.to_string()),
        ))
    }
}

impl From<crate::filesystem::InvalidSandboxPath> for Error {
    fn from(error: crate::filesystem::InvalidSandboxPath) -> Self {
        Self::InvalidSpec(InvalidSpec::new(
            "validate sandbox path",
            InvalidSpecReason::InvalidPath(error.to_string()),
        ))
    }
}

impl From<TemplateId> for NotFound {
    fn from(id: TemplateId) -> Self {
        Self::new("get template", ResourceKind::Template, id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::data_plane::ReservedPort;

    use super::{Error, TemplatePrepareFailure};

    #[test]
    fn template_prepare_failure_is_branchable() {
        let error = Error::from(TemplatePrepareFailure::PortConflict {
            port: ReservedPort::new(49_983).expect("reserved port"),
        });
        assert!(matches!(
            error,
            Error::TemplatePrepareFailure(TemplatePrepareFailure::PortConflict { .. })
        ));
    }
}
