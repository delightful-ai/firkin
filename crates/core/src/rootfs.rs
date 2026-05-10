//! rootfs — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::builder::OnVm;
#[allow(unused_imports)]
use crate::ids::GuestPath;
#[allow(unused_imports)]
use firkin_oci::ImageBundle;
#[allow(unused_imports)]
use firkin_types::BlockDeviceId;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
/// Rootfs variants accepted by an implicit single-use VM builder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rootfs {
    /// Existing EXT4 image path.
    Ext4Image(PathBuf),
    /// Pulled OCI image bundle to assemble into EXT4 at spawn time.
    OciBundle(Box<ImageBundle>),
    /// Existing raw block image path.
    RawBlock(PathBuf),
}
impl Rootfs {
    /// Construct an EXT4 image rootfs.
    #[must_use]
    pub fn ext4_image(path: impl Into<PathBuf>) -> Self {
        Self::Ext4Image(path.into())
    }
    /// Construct an OCI image bundle rootfs.
    #[must_use]
    pub fn oci_bundle(bundle: ImageBundle) -> Self {
        Self::OciBundle(Box::new(bundle))
    }
    /// Construct a raw block image rootfs.
    #[must_use]
    pub fn raw_block(path: impl Into<PathBuf>) -> Self {
        Self::RawBlock(path.into())
    }
    /// Bridge from a predeclared VM block device to the running-VM rootfs type.
    #[must_use]
    pub const fn block_device(id: BlockDeviceId) -> VmRootfs {
        VmRootfs::BlockDevice(id)
    }
}
impl From<PathBuf> for Rootfs {
    fn from(value: PathBuf) -> Self {
        Self::Ext4Image(value)
    }
}
impl From<&Path> for Rootfs {
    fn from(value: &Path) -> Self {
        Self::Ext4Image(value.to_path_buf())
    }
}
/// Rootfs variants accepted by builders attached to an already-running VM.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum VmRootfs {
    /// A predeclared VM block device.
    BlockDevice(BlockDeviceId),
    /// A rootfs directory that already exists in the guest.
    GuestPath(GuestPath),
}
impl VmRootfs {
    /// Construct a guest-path rootfs.
    #[must_use]
    pub fn guest_path(path: GuestPath) -> Self {
        Self::GuestPath(path)
    }
    /// Return the predeclared block device handle, if this rootfs is one.
    #[must_use]
    pub const fn as_block_device(&self) -> Option<BlockDeviceId> {
        match self {
            Self::BlockDevice(id) => Some(*id),
            Self::GuestPath(_) => None,
        }
    }
    /// Return the guest rootfs path, if this rootfs is one.
    #[must_use]
    pub fn as_guest_path(&self) -> Option<&GuestPath> {
        match self {
            Self::BlockDevice(_) => None,
            Self::GuestPath(path) => Some(path),
        }
    }
}
impl From<BlockDeviceId> for VmRootfs {
    fn from(value: BlockDeviceId) -> Self {
        Self::BlockDevice(value)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct StagedRootfs {
    pub(crate) path: PathBuf,
}
#[allow(dead_code)]
impl StagedRootfs {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
#[derive(Debug)]
pub(crate) enum CopyInPayload {
    File { path: PathBuf, mode: u32 },
    DirectoryArchive { archive: tempfile::NamedTempFile },
}
impl CopyInPayload {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::File { path, .. } => path,
            Self::DirectoryArchive { archive } => archive.path(),
        }
    }
    pub(crate) const fn mode(&self) -> u32 {
        match self {
            Self::File { mode, .. } => *mode,
            Self::DirectoryArchive { .. } => 0,
        }
    }
    pub(crate) const fn is_archive(&self) -> bool {
        matches!(self, Self::DirectoryArchive { .. })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RootfsChoice {
    Implicit(Box<Rootfs>),
    OnVm(VmRootfs),
}
