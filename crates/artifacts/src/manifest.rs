//! manifest — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::pod_membership::RecordedPodMembership;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Durable snapshot artifact category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotArtifactKind {
    /// Base template snapshot used as a normal session-create source.
    BaseTemplate,
    /// Continuation snapshot used to resume a prior session.
    Continuation,
}
/// Manifest for a durable snapshot artifact.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[allow(private_interfaces)]
pub struct SnapshotArtifactManifest {
    #[allow(missing_docs)]
    pub kind: SnapshotArtifactKind,
    logical_id: String,
    pub(crate) path: PathBuf,
    created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pod_membership: Option<RecordedPodMembership>,
}
impl SnapshotArtifactManifest {
    /// Construct a base template snapshot manifest.
    #[must_use]
    pub fn base(logical_id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(SnapshotArtifactKind::BaseTemplate, logical_id, path)
    }
    /// Construct a continuation snapshot manifest.
    #[must_use]
    pub fn continuation(logical_id: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self::new(SnapshotArtifactKind::Continuation, logical_id, path)
    }
    /// Return the deterministic JSON sidecar path for an artifact path.
    #[must_use]
    pub fn sidecar_path_for_artifact(path: impl AsRef<Path>) -> PathBuf {
        path.as_ref().with_extension("manifest.json")
    }
    /// Persist this manifest as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotArtifactManifestError`] when JSON encoding or the
    /// filesystem write fails.
    pub fn write_json(&self, path: impl AsRef<Path>) -> Result<(), SnapshotArtifactManifestError> {
        let path = path.as_ref();
        let bytes = serde_json::to_vec_pretty(self).map_err(|source| {
            SnapshotArtifactManifestError::Json {
                path: path.to_path_buf(),
                source,
            }
        })?;
        fs::write(path, bytes).map_err(|source| SnapshotArtifactManifestError::Io {
            operation: "write manifest JSON",
            path: path.to_path_buf(),
            source,
        })
    }
    /// Read a manifest JSON file.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotArtifactManifestError`] when reading or decoding the
    /// manifest fails.
    pub fn read_json(path: impl AsRef<Path>) -> Result<Self, SnapshotArtifactManifestError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| SnapshotArtifactManifestError::Io {
            operation: "read manifest JSON",
            path: path.to_path_buf(),
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| SnapshotArtifactManifestError::Json {
            path: path.to_path_buf(),
            source,
        })
    }
    /// Read all direct `*.manifest.json` sidecars under a directory in sorted
    /// path order.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotArtifactManifestError`] when the directory cannot be
    /// read or a sidecar cannot be decoded.
    pub fn read_json_dir(
        root: impl AsRef<Path>,
    ) -> Result<Vec<Self>, SnapshotArtifactManifestError> {
        let root = root.as_ref();
        let mut sidecars = Vec::new();
        for entry in fs::read_dir(root).map_err(|source| SnapshotArtifactManifestError::Io {
            operation: "read manifest directory",
            path: root.to_path_buf(),
            source,
        })? {
            let entry = entry.map_err(|source| SnapshotArtifactManifestError::Io {
                operation: "read manifest directory entry",
                path: root.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            let metadata =
                entry
                    .metadata()
                    .map_err(|source| SnapshotArtifactManifestError::Io {
                        operation: "stat manifest sidecar",
                        path: path.clone(),
                        source,
                    })?;
            if metadata.is_file() && is_snapshot_manifest_sidecar(&path) {
                sidecars.push(path);
            }
        }
        sidecars.sort();
        sidecars.into_iter().map(Self::read_json).collect()
    }
    #[allow(missing_docs)]
    pub fn new(
        kind: SnapshotArtifactKind,
        logical_id: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            kind,
            logical_id: logical_id.into(),
            path: path.into(),
            created_at: "pending-runtime-timestamp".to_owned(),
            pod_membership: None,
        }
    }
    /// Attach pod membership metadata to this manifest.
    #[must_use]
    pub fn with_pod_membership(mut self, membership: RecordedPodMembership) -> Self {
        self.pod_membership = Some(membership);
        self
    }
    /// Return the snapshot artifact kind.
    #[must_use]
    pub const fn kind(&self) -> SnapshotArtifactKind {
        self.kind
    }
    /// Return the caller-visible logical id.
    #[must_use]
    pub fn logical_id(&self) -> &str {
        &self.logical_id
    }
    /// Return the snapshot artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Return the creation marker.
    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.created_at
    }
    /// Return recorded pod membership, if this snapshot belongs to a pod.
    #[must_use]
    pub const fn pod_membership(&self) -> Option<&RecordedPodMembership> {
        self.pod_membership.as_ref()
    }
}
/// Snapshot artifact manifest persistence error.
#[derive(Debug, ThisError)]
#[allow(private_interfaces)]
pub enum SnapshotArtifactManifestError {
    /// Filesystem operation failed.
    #[error(
        "snapshot artifact manifest filesystem operation failed while {operation} at {path}: {source}"
    )]
    Io {
        /// Operation being attempted.
        operation: &'static str,
        /// Manifest path involved in the operation.
        path: PathBuf,
        /// Source error.
        #[source]
        source: io::Error,
    },
    /// JSON encoding or decoding failed.
    #[error("snapshot artifact manifest JSON operation failed at {path}: {source}")]
    Json {
        /// Manifest path involved in the operation.
        path: PathBuf,
        /// Source error.
        #[source]
        source: serde_json::Error,
    },
}
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub fn is_snapshot_manifest_sidecar(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".manifest.json"))
}
