//! pod membership — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
/// Durable pod membership recorded with a snapshot artifact.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct RecordedPodMembership {
    pod_id: String,
    containers: Vec<RecordedPodContainer>,
}
impl RecordedPodMembership {
    /// Construct recorded pod membership.
    pub fn new(
        pod_id: impl Into<String>,
        containers: impl IntoIterator<Item = RecordedPodContainer>,
    ) -> Self {
        Self {
            pod_id: pod_id.into(),
            containers: containers.into_iter().collect(),
        }
    }
    /// Return the recorded pod ID.
    #[must_use]
    pub fn pod_id(&self) -> &str {
        &self.pod_id
    }
    /// Return recorded pod containers.
    #[must_use]
    pub fn containers(&self) -> &[RecordedPodContainer] {
        &self.containers
    }
}
/// Durable container membership recorded with a pod snapshot artifact.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct RecordedPodContainer {
    #[allow(missing_docs)]
    pub name: String,
    rootfs_logical_id: String,
    rootfs_sha256_hex: String,
    rootfs_size_bytes: u64,
    volume_mounts: Vec<RecordedPodVolumeMount>,
}
impl RecordedPodContainer {
    /// Construct recorded pod container membership.
    pub fn new(
        name: impl Into<String>,
        rootfs_logical_id: impl Into<String>,
        rootfs_sha256_hex: impl Into<String>,
        rootfs_size_bytes: u64,
        volume_mounts: impl IntoIterator<Item = RecordedPodVolumeMount>,
    ) -> Self {
        Self {
            name: name.into(),
            rootfs_logical_id: rootfs_logical_id.into(),
            rootfs_sha256_hex: rootfs_sha256_hex.into(),
            rootfs_size_bytes,
            volume_mounts: volume_mounts.into_iter().collect(),
        }
    }
    /// Return the container name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Return the rootfs logical ID.
    #[must_use]
    pub fn rootfs_logical_id(&self) -> &str {
        &self.rootfs_logical_id
    }
    /// Return the rootfs SHA-256 digest.
    #[must_use]
    pub fn rootfs_sha256_hex(&self) -> &str {
        &self.rootfs_sha256_hex
    }
    /// Return the rootfs byte size.
    #[must_use]
    pub const fn rootfs_size_bytes(&self) -> u64 {
        self.rootfs_size_bytes
    }
    /// Return volume mounts recorded for this container.
    #[must_use]
    pub fn volume_mounts(&self) -> &[RecordedPodVolumeMount] {
        &self.volume_mounts
    }
}
/// Durable pod volume mount recorded with a pod snapshot artifact.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct RecordedPodVolumeMount {
    #[allow(missing_docs)]
    pub name: String,
    mount_path: PathBuf,
    read_only: bool,
}
impl RecordedPodVolumeMount {
    /// Construct a recorded pod volume mount.
    pub fn new(name: impl Into<String>, mount_path: impl Into<PathBuf>, read_only: bool) -> Self {
        Self {
            name: name.into(),
            mount_path: mount_path.into(),
            read_only,
        }
    }
    /// Return the volume name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Return the container mount path.
    #[must_use]
    pub fn mount_path(&self) -> &Path {
        &self.mount_path
    }
    /// Return whether the mount was read-only.
    #[must_use]
    pub const fn read_only(&self) -> bool {
        self.read_only
    }
}
