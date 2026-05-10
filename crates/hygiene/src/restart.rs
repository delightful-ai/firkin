//! restart — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
/// Resource kind discovered during restart reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartResourceKind {
    /// VM believed to be active before restart.
    ActiveVm,
    /// Durable snapshot artifact.
    SnapshotArtifact,
    /// Log stream or log artifact.
    LogStream,
    /// Runtime process left behind by a previous controller.
    StaleRuntimeProcess,
}
/// Persisted or discovered state record considered during restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestartStateRecord {
    pub(crate) id: String,
    #[allow(missing_docs)]
    pub kind: RestartResourceKind,
}
impl RestartStateRecord {
    /// Construct a restart state record.
    #[must_use]
    pub fn new(id: impl Into<String>, kind: RestartResourceKind) -> Self {
        Self {
            id: id.into(),
            kind,
        }
    }
    /// Return the record identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Return the restart resource kind.
    #[must_use]
    pub const fn kind(&self) -> RestartResourceKind {
        self.kind
    }
}
