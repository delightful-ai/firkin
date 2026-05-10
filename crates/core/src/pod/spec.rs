//! spec — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::GuestPath;
#[allow(unused_imports)]
use crate::Result;
#[allow(unused_imports)]
use crate::{IntoContainerId, Mount, Stdio};
#[allow(unused_imports)]
use firkin_oci::ImageBundle;
#[allow(unused_imports)]
use firkin_types::ContainerId;
#[allow(unused_imports)]
use firkin_types::{InvalidContainerId, Size};
#[allow(unused_imports)]
use std::collections::HashSet;
#[allow(unused_imports)]
use std::ffi::OsString;
#[cfg(test)]
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Guest filesystem used for a mounted Firkin pod store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuestFilesystem {
    /// Linux ext4.
    Ext4,
}
impl GuestFilesystem {
    pub(crate) fn mount_type(self) -> &'static str {
        match self {
            Self::Ext4 => "ext4",
        }
    }
}
/// Pod specification validation error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum PodValidationError {
    /// A pod declares the same initial container ID more than once.
    #[error("pod declares duplicate container `{0}`")]
    DuplicateContainerName(ContainerId),
    /// A pod declares the same `emptyDir` volume name more than once.
    #[error("pod declares duplicate emptyDir `{0}`")]
    DuplicateEmptyDirName(ContainerId),
    /// A container references an `emptyDir` that the pod did not declare.
    #[error("container `{container}` references unknown emptyDir `{volume_name}`")]
    UnknownEmptyDirMount {
        /// Container carrying the invalid mount.
        container: ContainerId,
        /// Missing pod `emptyDir` volume name.
        volume_name: ContainerId,
    },
}
/// Storage medium for a pod-local `emptyDir` volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmptyDirMedium {
    /// Directory inside the pod-store filesystem.
    Disk,
    /// Guest tmpfs mounted once for the pod.
    Memory,
}
/// Pod-owned `emptyDir` volume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmptyDirVolume {
    pub(crate) name: ContainerId,
    pub(crate) medium: EmptyDirMedium,
    pub(crate) size_limit: Option<Size>,
}
impl EmptyDirVolume {
    /// Construct a disk-backed `emptyDir` volume.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when `name` is not a path-safe volume
    /// name.
    pub fn disk(name: impl Into<String>) -> std::result::Result<Self, InvalidContainerId> {
        Ok(Self {
            name: ContainerId::new(name)?,
            medium: EmptyDirMedium::Disk,
            size_limit: None,
        })
    }
    /// Construct a memory-backed `emptyDir` volume.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when `name` is not a path-safe volume
    /// name.
    pub fn memory(
        name: impl Into<String>,
        size_limit: Option<Size>,
    ) -> std::result::Result<Self, InvalidContainerId> {
        Ok(Self {
            name: ContainerId::new(name)?,
            medium: EmptyDirMedium::Memory,
            size_limit,
        })
    }
    /// Return the volume name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }
    /// Return the volume medium.
    #[must_use]
    pub const fn medium(&self) -> EmptyDirMedium {
        self.medium
    }
    /// Return the optional size limit.
    #[must_use]
    pub const fn size_limit(&self) -> Option<Size> {
        self.size_limit
    }
}
/// Source used to materialize a pod container rootfs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PodRootfsSource {
    /// Already-materialized guest path.
    GuestPath(GuestPath),
    /// Pulled OCI image bundle.
    OciBundle(Box<ImageBundle>),
    /// Host ext4 image; not materialized in-place until a guest mount/copy
    /// helper lands.
    Ext4Image(PathBuf),
}
impl PodRootfsSource {
    /// Construct a guest-path rootfs source.
    #[must_use]
    pub const fn guest_path(path: GuestPath) -> Self {
        Self::GuestPath(path)
    }
    /// Construct an OCI bundle rootfs source.
    #[must_use]
    pub fn oci_bundle(bundle: ImageBundle) -> Self {
        Self::OciBundle(Box::new(bundle))
    }
    /// Construct an ext4 image rootfs source.
    #[must_use]
    pub fn ext4_image(path: impl Into<PathBuf>) -> Self {
        Self::Ext4Image(path.into())
    }
}
/// Mount of a pod-owned volume into one container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodVolumeMount {
    pub(crate) volume_name: ContainerId,
    pub(crate) container_path: PathBuf,
    pub(crate) read_only: bool,
}
impl PodVolumeMount {
    /// Construct a read-write pod volume mount.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when `volume_name` is not path-safe.
    pub fn read_write(
        volume_name: impl Into<String>,
        container_path: impl Into<PathBuf>,
    ) -> std::result::Result<Self, InvalidContainerId> {
        Ok(Self {
            volume_name: ContainerId::new(volume_name)?,
            container_path: container_path.into(),
            read_only: false,
        })
    }
    /// Construct a read-only pod volume mount.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when `volume_name` is not path-safe.
    pub fn read_only_mount(
        volume_name: impl Into<String>,
        container_path: impl Into<PathBuf>,
    ) -> std::result::Result<Self, InvalidContainerId> {
        Ok(Self {
            volume_name: ContainerId::new(volume_name)?,
            container_path: container_path.into(),
            read_only: true,
        })
    }
    /// Return the volume name.
    #[must_use]
    pub fn volume_name(&self) -> &str {
        self.volume_name.as_str()
    }
    /// Return the target path inside the container.
    #[must_use]
    pub fn container_path(&self) -> &Path {
        &self.container_path
    }
    /// Return whether the mount is read-only.
    #[must_use]
    pub const fn read_only(&self) -> bool {
        self.read_only
    }
}
/// Pre-start container specification owned by a [`PodBuilder`] or [`Pod`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodContainerSpec {
    pub(crate) id: ContainerId,
    pub(crate) rootfs: PodRootfsSource,
    pub(crate) command: Vec<OsString>,
    pub(crate) env: Vec<(OsString, OsString)>,
    pub(crate) mounts: Vec<Mount>,
    pub(crate) empty_dir_mounts: Vec<PodVolumeMount>,
    pub(crate) stdin: Stdio,
    pub(crate) stdout: Stdio,
    pub(crate) stderr: Stdio,
}
impl PodContainerSpec {
    /// Construct a pod container spec.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when `id` is not path-safe.
    pub fn new(
        id: impl IntoContainerId,
        rootfs: PodRootfsSource,
    ) -> std::result::Result<Self, InvalidContainerId> {
        Ok(Self {
            id: id.into_container_id()?,
            rootfs,
            command: Vec::new(),
            env: Vec::new(),
            mounts: Vec::new(),
            empty_dir_mounts: Vec::new(),
            stdin: Stdio::Null,
            stdout: Stdio::Null,
            stderr: Stdio::Null,
        })
    }
    /// Return the container ID.
    #[must_use]
    pub const fn id(&self) -> &ContainerId {
        &self.id
    }
    /// Return the rootfs source.
    #[must_use]
    pub const fn rootfs(&self) -> &PodRootfsSource {
        &self.rootfs
    }
    /// Return command arguments.
    #[must_use]
    pub fn command_args(&self) -> &[OsString] {
        &self.command
    }
    /// Return configured environment variables as key/value pairs.
    #[must_use]
    pub fn env_vars(&self) -> &[(OsString, OsString)] {
        &self.env
    }
    /// Return pod volume mounts.
    #[must_use]
    pub fn empty_dir_mounts(&self) -> &[PodVolumeMount] {
        &self.empty_dir_mounts
    }
    /// Set command arguments.
    #[must_use]
    pub fn command<I, A>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        self.command = args.into_iter().map(Into::into).collect();
        self
    }
    /// Append one environment variable.
    #[must_use]
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
    /// Append environment variables.
    #[must_use]
    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        self.env.extend(
            vars.into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        self
    }
    /// Append a raw OCI mount.
    #[must_use]
    pub fn mount(mut self, mount: Mount) -> Self {
        self.mounts.push(mount);
        self
    }
    /// Append a read-write `emptyDir` volume mount.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when `volume_name` is not path-safe.
    pub fn empty_dir_mount(
        mut self,
        volume_name: impl Into<String>,
        container_path: impl Into<PathBuf>,
    ) -> std::result::Result<Self, InvalidContainerId> {
        self.empty_dir_mounts
            .push(PodVolumeMount::read_write(volume_name, container_path)?);
        Ok(self)
    }
    /// Append a read-only `emptyDir` volume mount.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when `volume_name` is not path-safe.
    pub fn empty_dir_mount_read_only(
        mut self,
        volume_name: impl Into<String>,
        container_path: impl Into<PathBuf>,
    ) -> std::result::Result<Self, InvalidContainerId> {
        self.empty_dir_mounts.push(PodVolumeMount::read_only_mount(
            volume_name,
            container_path,
        )?);
        Ok(self)
    }
    /// Set stdin behavior.
    #[must_use]
    pub const fn stdin(mut self, stdin: Stdio) -> Self {
        self.stdin = stdin;
        self
    }
    /// Set stdout behavior.
    #[must_use]
    pub const fn stdout(mut self, stdout: Stdio) -> Self {
        self.stdout = stdout;
        self
    }
    /// Set stderr behavior.
    #[must_use]
    pub const fn stderr(mut self, stderr: Stdio) -> Self {
        self.stderr = stderr;
        self
    }
}
pub(crate) fn validate_pod_spec(
    empty_dirs: &[EmptyDirVolume],
    containers: &[PodContainerSpec],
) -> Result<()> {
    let mut seen_empty_dirs = HashSet::new();
    for volume in empty_dirs {
        if !seen_empty_dirs.insert(volume.name.clone()) {
            return Err(PodValidationError::DuplicateEmptyDirName(volume.name.clone()).into());
        }
    }
    let mut seen_containers = HashSet::new();
    for container in containers {
        if !seen_containers.insert(container.id.clone()) {
            return Err(PodValidationError::DuplicateContainerName(container.id.clone()).into());
        }
        validate_container_empty_dir_mounts(empty_dirs, container)?;
    }
    Ok(())
}
pub(crate) fn validate_container_empty_dir_mounts(
    empty_dirs: &[EmptyDirVolume],
    container: &PodContainerSpec,
) -> Result<()> {
    for mount in &container.empty_dir_mounts {
        if !empty_dirs
            .iter()
            .any(|volume| volume.name == mount.volume_name)
        {
            return Err(PodValidationError::UnknownEmptyDirMount {
                container: container.id.clone(),
                volume_name: mount.volume_name.clone(),
            }
            .into());
        }
    }
    Ok(())
}
