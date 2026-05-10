//! continuation — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::manifest::SnapshotArtifactManifest;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
/// Reason a continuation snapshot is captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContinuationSnapshotReason {
    /// Session became idle.
    Idle,
    /// Session was stopped.
    Stopped,
    /// Session process exited.
    Exited,
}
/// Plan for capturing a follow-up continuation snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinuationSnapshotPlan {
    session_id: String,
    #[allow(missing_docs)]
    pub reason: ContinuationSnapshotReason,
    #[allow(missing_docs)]
    pub snapshot_output_path: PathBuf,
}
impl ContinuationSnapshotPlan {
    /// Construct a continuation snapshot plan.
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        reason: ContinuationSnapshotReason,
        snapshot_output_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            reason,
            snapshot_output_path: snapshot_output_path.into(),
        }
    }
    /// Return the source session id.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    /// Return the snapshot reason.
    #[must_use]
    pub const fn reason(&self) -> ContinuationSnapshotReason {
        self.reason
    }
    /// Return the snapshot output path.
    #[must_use]
    pub fn snapshot_output_path(&self) -> &Path {
        &self.snapshot_output_path
    }
    /// Return the continuation snapshot manifest this plan should produce.
    #[must_use]
    pub fn snapshot_manifest(&self) -> SnapshotArtifactManifest {
        SnapshotArtifactManifest::continuation(&self.session_id, self.snapshot_output_path.clone())
    }
}
