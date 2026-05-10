//! store — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::GuestPath;
#[allow(unused_imports)]
use crate::pod::mount_pod_store_with_client;
#[allow(unused_imports)]
use crate::pod::spec::GuestFilesystem;
#[allow(unused_imports)]
use crate::{Result, connect_vminitd};
#[allow(unused_imports)]
use firkin_types::BlockDeviceId;
#[allow(unused_imports)]
use firkin_vmm::{Running, VirtualMachine};
#[cfg(test)]
#[allow(unused_imports)]
use std::io;
/// Predeclared block device mounted as a Firkin pod store inside the guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodStoreSpec {
    block_device: BlockDeviceId,
    guest_mount: GuestPath,
    filesystem: GuestFilesystem,
}
impl PodStoreSpec {
    /// Construct an ext4 pod store mounted at `/run/firkin/pod-store`.
    ///
    /// The block device must be predeclared on the VM configuration.
    ///
    /// # Panics
    ///
    /// Panics only if the built-in pod-store guest path is invalid.
    #[must_use]
    pub fn ext4(block_device: BlockDeviceId) -> Self {
        Self {
            block_device,
            guest_mount: GuestPath::new("/run/firkin/pod-store")
                .expect("default pod-store path is valid"),
            filesystem: GuestFilesystem::Ext4,
        }
    }
    /// Return the backing block device.
    #[must_use]
    pub const fn block_device(&self) -> BlockDeviceId {
        self.block_device
    }
    /// Return the guest mount point.
    #[must_use]
    pub const fn guest_mount(&self) -> &GuestPath {
        &self.guest_mount
    }
    /// Return the guest filesystem.
    #[must_use]
    pub const fn filesystem(&self) -> GuestFilesystem {
        self.filesystem
    }
}
/// Mounted pod-store handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountedPodStore {
    pub(crate) spec: PodStoreSpec,
}
impl MountedPodStore {
    /// Return the mounted pod-store specification.
    #[must_use]
    pub const fn spec(&self) -> &PodStoreSpec {
        &self.spec
    }
    /// Return the mounted guest path.
    #[must_use]
    pub const fn guest_mount(&self) -> &GuestPath {
        self.spec.guest_mount()
    }
}
/// Mount a predeclared pod-store block device inside a running VM.
///
/// This is the substrate used by elastic pods before the first-class `Pod`
/// aggregate exists. The block device must have been declared on the VM config
/// before boot.
///
/// # Errors
///
/// Returns an error if vminitd cannot be reached, the block-device slot cannot
/// be mapped to the current virtio-blk guest path, or the guest rejects the
/// mount request.
pub async fn mount_pod_store(
    vm: &VirtualMachine<Running>,
    spec: &PodStoreSpec,
) -> Result<MountedPodStore> {
    let mut client = connect_vminitd(vm).await?;
    mount_pod_store_with_client(&mut client, spec).await
}
