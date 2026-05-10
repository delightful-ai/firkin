//! artifact gc — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_artifacts::SnapshotArtifactManifestError;
#[allow(unused_imports)]
use firkin_artifacts::{SnapshotArtifactManifest, is_snapshot_manifest_sidecar};
#[allow(unused_imports)]
use std::collections::BTreeSet;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use std::time::SystemTime;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Snapshot artifact GC planning error.
#[derive(Debug, ThisError)]
pub enum ArtifactGcError {
    /// Filesystem operation failed.
    #[error("artifact GC filesystem operation failed while {operation}: {source}")]
    Io {
        /// Operation being attempted.
        operation: &'static str,
        /// Source error.
        #[source]
        source: io::Error,
    },
    /// Manifest sidecar discovery failed.
    #[error("artifact GC manifest sidecar discovery failed: {0}")]
    Manifest(#[from] SnapshotArtifactManifestError),
}
/// Snapshot artifact GC plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactGcPlan {
    pub(crate) root: PathBuf,
    keep: BTreeSet<PathBuf>,
    delete: Vec<PathBuf>,
}
impl ArtifactGcPlan {
    /// Build a conservative GC plan for artifacts directly under `root`.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactGcError`] when the snapshot directory cannot be read.
    pub fn for_snapshot_dir(
        root: impl AsRef<Path>,
        manifests: impl IntoIterator<Item = SnapshotArtifactManifest>,
    ) -> Result<Self, ArtifactGcError> {
        Self::for_snapshot_dir_with_retention(root, manifests, Duration::ZERO, SystemTime::now())
    }
    /// Build a conservative GC plan for files or directories directly under
    /// `root`, retaining unreferenced artifacts newer than
    /// `min_unreferenced_age`.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactGcError`] when the snapshot directory or artifact
    /// metadata cannot be read.
    pub fn for_snapshot_dir_with_retention(
        root: impl AsRef<Path>,
        manifests: impl IntoIterator<Item = SnapshotArtifactManifest>,
        min_unreferenced_age: Duration,
        now: SystemTime,
    ) -> Result<Self, ArtifactGcError> {
        let root = root.as_ref().to_path_buf();
        let keep = manifests
            .into_iter()
            .map(|manifest| manifest.path().to_path_buf())
            .collect::<BTreeSet<_>>();
        let mut delete = Vec::new();
        for entry in fs::read_dir(&root).map_err(|source| ArtifactGcError::Io {
            operation: "read snapshot artifact directory",
            source,
        })? {
            let entry = entry.map_err(|source| ArtifactGcError::Io {
                operation: "read snapshot artifact entry",
                source,
            })?;
            let path = entry.path();
            let metadata = entry.metadata().map_err(|source| ArtifactGcError::Io {
                operation: "stat snapshot artifact",
                source,
            })?;
            if !keep.contains(&path)
                && !is_snapshot_manifest_sidecar(&path)
                && (metadata.is_file() || metadata.is_dir())
            {
                let modified = metadata.modified().map_err(|source| ArtifactGcError::Io {
                    operation: "stat snapshot artifact",
                    source,
                })?;
                if now.duration_since(modified).unwrap_or(Duration::ZERO) >= min_unreferenced_age {
                    delete.push(path);
                }
            }
        }
        delete.sort();
        Ok(Self { root, keep, delete })
    }
    /// Build a GC plan using manifests discovered from direct
    /// `*.manifest.json` sidecars under `manifest_root`.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactGcError`] when manifest sidecar discovery, snapshot
    /// directory scanning, or file metadata reads fail.
    pub fn for_snapshot_dir_with_manifest_sidecars(
        root: impl AsRef<Path>,
        manifest_root: impl AsRef<Path>,
        min_unreferenced_age: Duration,
        now: SystemTime,
    ) -> Result<Self, ArtifactGcError> {
        let manifests = SnapshotArtifactManifest::read_json_dir(manifest_root)?;
        Self::for_snapshot_dir_with_retention(root, manifests, min_unreferenced_age, now)
    }
    /// Return the artifact root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Return paths preserved by the plan.
    #[must_use]
    pub fn keep(&self) -> &BTreeSet<PathBuf> {
        &self.keep
    }
    /// Return paths deleted by the plan.
    #[must_use]
    pub fn delete(&self) -> &[PathBuf] {
        &self.delete
    }
    /// Return whether a path is preserved by the plan.
    #[must_use]
    pub fn keeps_path(&self, path: impl AsRef<Path>) -> bool {
        self.keep.contains(path.as_ref())
    }
    /// Return whether a path is deleted by the plan.
    #[must_use]
    pub fn deletes_path(&self, path: impl AsRef<Path>) -> bool {
        self.delete
            .iter()
            .any(|candidate| candidate == path.as_ref())
    }
    /// Execute the GC plan.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactGcError`] when any planned delete fails.
    pub fn execute(&self) -> Result<ArtifactGcReport, ArtifactGcError> {
        let mut deleted = Vec::new();
        for path in &self.delete {
            if path.is_dir() {
                fs::remove_dir_all(path).map_err(|source| ArtifactGcError::Io {
                    operation: "delete snapshot artifact directory",
                    source,
                })?;
            } else {
                fs::remove_file(path).map_err(|source| ArtifactGcError::Io {
                    operation: "delete snapshot artifact file",
                    source,
                })?;
            }
            deleted.push(path.clone());
        }
        Ok(ArtifactGcReport { deleted })
    }
}
/// Result of executing an artifact GC plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactGcReport {
    deleted: Vec<PathBuf>,
}
impl ArtifactGcReport {
    /// Return deleted paths.
    #[must_use]
    pub fn deleted(&self) -> &[PathBuf] {
        &self.deleted
    }
    /// Return deleted path count.
    #[must_use]
    pub const fn deleted_count(&self) -> usize {
        self.deleted.len()
    }
}
