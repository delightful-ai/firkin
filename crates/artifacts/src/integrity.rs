//! integrity — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::manifest::SnapshotArtifactManifest;
#[allow(unused_imports)]
use sha2::Digest as ShaDigest;
#[allow(unused_imports)]
use sha2::Sha256;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Snapshot artifact integrity error.
#[derive(Debug, ThisError)]
pub enum SnapshotArtifactIntegrityError {
    /// Filesystem operation failed.
    #[error("snapshot artifact integrity filesystem operation failed while {operation}: {source}")]
    Io {
        /// Operation being attempted.
        operation: &'static str,
        /// Source error.
        #[source]
        source: io::Error,
    },
    /// Artifact byte length changed.
    #[error("snapshot artifact size mismatch: expected {expected}, actual {actual}")]
    SizeMismatch {
        /// Expected byte length.
        expected: u64,
        /// Actual byte length.
        actual: u64,
    },
    /// Artifact SHA-256 digest changed.
    #[error("snapshot artifact sha256 mismatch")]
    Sha256Mismatch {
        /// Expected SHA-256 hex digest.
        expected: String,
        /// Actual SHA-256 hex digest.
        actual: String,
    },
    /// Integrity sidecar JSON encoding or decoding failed.
    #[error("snapshot artifact integrity JSON operation failed at {path}: {source}")]
    Json {
        /// Integrity sidecar path involved in the operation.
        path: PathBuf,
        /// Source error.
        #[source]
        source: serde_json::Error,
    },
}
/// Snapshot artifact integrity record.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SnapshotArtifactIntegrity {
    size_bytes: u64,
    sha256_hex: String,
}
impl SnapshotArtifactIntegrity {
    /// Construct an integrity record from persisted metadata.
    #[must_use]
    pub fn new(size_bytes: u64, sha256_hex: impl Into<String>) -> Self {
        Self {
            size_bytes,
            sha256_hex: sha256_hex.into(),
        }
    }
    /// Compute integrity for a snapshot artifact manifest.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotArtifactIntegrityError`] when the artifact cannot be
    /// read.
    pub fn from_file(
        manifest: &SnapshotArtifactManifest,
    ) -> Result<Self, SnapshotArtifactIntegrityError> {
        let bytes =
            fs::read(manifest.path()).map_err(|source| SnapshotArtifactIntegrityError::Io {
                operation: "read snapshot artifact",
                source,
            })?;
        Ok(Self {
            size_bytes: bytes.len() as u64,
            sha256_hex: sha256_hex(&bytes),
        })
    }
    /// Return the deterministic JSON sidecar path for an artifact path.
    #[must_use]
    pub fn sidecar_path_for_artifact(path: impl AsRef<Path>) -> PathBuf {
        path.as_ref().with_extension("integrity.json")
    }
    /// Persist this integrity record as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotArtifactIntegrityError`] when JSON encoding or the
    /// filesystem write fails.
    pub fn write_json(&self, path: impl AsRef<Path>) -> Result<(), SnapshotArtifactIntegrityError> {
        let path = path.as_ref();
        let bytes = serde_json::to_vec_pretty(self).map_err(|source| {
            SnapshotArtifactIntegrityError::Json {
                path: path.to_path_buf(),
                source,
            }
        })?;
        fs::write(path, bytes).map_err(|source| SnapshotArtifactIntegrityError::Io {
            operation: "write snapshot artifact integrity JSON",
            source,
        })
    }
    /// Read an integrity JSON sidecar.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotArtifactIntegrityError`] when reading or decoding the
    /// sidecar fails.
    pub fn read_json(path: impl AsRef<Path>) -> Result<Self, SnapshotArtifactIntegrityError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| SnapshotArtifactIntegrityError::Io {
            operation: "read snapshot artifact integrity JSON",
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| SnapshotArtifactIntegrityError::Json {
            path: path.to_path_buf(),
            source,
        })
    }
    /// Verify a snapshot artifact still matches this integrity record.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotArtifactIntegrityError`] when the artifact cannot be
    /// read or its size/digest does not match.
    pub fn verify(
        &self,
        manifest: &SnapshotArtifactManifest,
    ) -> Result<(), SnapshotArtifactIntegrityError> {
        let current = Self::from_file(manifest)?;
        if current.size_bytes != self.size_bytes {
            return Err(SnapshotArtifactIntegrityError::SizeMismatch {
                expected: self.size_bytes,
                actual: current.size_bytes,
            });
        }
        if current.sha256_hex != self.sha256_hex {
            return Err(SnapshotArtifactIntegrityError::Sha256Mismatch {
                expected: self.sha256_hex.clone(),
                actual: current.sha256_hex,
            });
        }
        Ok(())
    }
    /// Return artifact byte length.
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
    /// Return SHA-256 hex digest.
    #[must_use]
    pub fn sha256_hex(&self) -> &str {
        &self.sha256_hex
    }
}
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("write to string");
    }
    output
}
