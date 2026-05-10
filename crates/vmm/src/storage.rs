//! storage — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::disk_image::DiskImageFormat;
#[allow(unused_imports)]
pub use firkin_types::{BlockDeviceId, Size, VirtiofsTag, VmId, VsockPort};
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
/// A declared virtiofs share.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtiofsShare {
    pub(crate) tag: VirtiofsTag,
    pub(crate) host_path: PathBuf,
}
impl VirtiofsShare {
    /// Return the guest-visible tag.
    #[must_use]
    pub fn tag(&self) -> &VirtiofsTag {
        &self.tag
    }
    /// Return the host path.
    #[must_use]
    pub fn host_path(&self) -> &Path {
        &self.host_path
    }
}
/// A declared VM block device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockDevice {
    pub(crate) id: BlockDeviceId,
    pub(crate) path: PathBuf,
    pub(crate) disk_image_format: DiskImageFormat,
    pub(crate) read_only: bool,
}
impl BlockDevice {
    /// Return the typed device handle.
    #[must_use]
    pub const fn id(&self) -> BlockDeviceId {
        self.id
    }
    /// Return the host backing path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Return the host disk image format.
    #[must_use]
    pub const fn disk_image_format(&self) -> DiskImageFormat {
        self.disk_image_format
    }
    /// Return whether this block device is attached read-only.
    #[must_use]
    pub const fn read_only(&self) -> bool {
        self.read_only
    }
}
