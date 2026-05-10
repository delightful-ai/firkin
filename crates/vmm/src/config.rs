//! config — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::disk_image::DiskImageFormat;
#[allow(unused_imports)]
use crate::error::{Result, invalid_config};
#[allow(unused_imports)]
use crate::kernel::{BootLog, KernelImage};
#[allow(unused_imports)]
use crate::network::{Network, default_network_macs, validate_mac_address, validate_vmnet_subnet};
#[allow(unused_imports)]
use crate::storage::{BlockDevice, VirtiofsShare};
#[cfg(target_vendor = "apple")]
#[allow(unused_imports)]
use crate::vz;
#[allow(unused_imports)]
pub use firkin_types::{BlockDeviceId, Size, VirtiofsTag, VmId, VsockPort};
#[allow(unused_imports)]
use std::collections::HashSet;
#[allow(unused_imports)]
use std::num::NonZeroU32;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
/// Validated VM configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmConfig {
    pub(crate) cpus: NonZeroU32,
    pub(crate) memory: Size,
    networks: Vec<Network>,
    virtiofs_shares: Vec<VirtiofsShare>,
    block_devices: Vec<BlockDevice>,
    rosetta: bool,
    nested_virtualization: bool,
    boot_log: BootLog,
    kernel: KernelImage,
    init_block: Option<PathBuf>,
    cmdline_extra: Vec<String>,
    machine_identifier: Vec<u8>,
    pub(crate) network_macs: Vec<String>,
}
impl VmConfig {
    /// Start a VM configuration builder.
    #[must_use]
    pub fn builder() -> VmConfigBuilder {
        VmConfigBuilder::default()
    }
    /// Return configured CPU count.
    #[must_use]
    pub const fn cpus(&self) -> NonZeroU32 {
        self.cpus
    }
    /// Return configured memory.
    #[must_use]
    pub const fn memory(&self) -> Size {
        self.memory
    }
    /// Return configured network attachments.
    #[must_use]
    pub fn networks(&self) -> &[Network] {
        &self.networks
    }
    /// Return configured virtiofs shares.
    #[must_use]
    pub fn virtiofs_shares(&self) -> &[VirtiofsShare] {
        &self.virtiofs_shares
    }
    /// Return configured block devices.
    #[must_use]
    pub fn block_devices(&self) -> &[BlockDevice] {
        &self.block_devices
    }
    /// Return whether Rosetta support is requested.
    #[must_use]
    pub const fn rosetta_enabled(&self) -> bool {
        self.rosetta
    }
    /// Return whether nested virtualization is requested.
    #[must_use]
    pub const fn nested_virtualization(&self) -> bool {
        self.nested_virtualization
    }
    /// Return boot-log configuration.
    #[must_use]
    pub const fn boot_log(&self) -> &BootLog {
        &self.boot_log
    }
    /// Return kernel image configuration.
    #[must_use]
    pub const fn kernel(&self) -> &KernelImage {
        &self.kernel
    }
    /// Return the vminitd init-block image path, if already resolved.
    #[must_use]
    pub fn init_block(&self) -> Option<&Path> {
        self.init_block.as_deref()
    }
    /// Return extra kernel command-line fragments.
    #[must_use]
    pub fn cmdline_extra(&self) -> &[String] {
        &self.cmdline_extra
    }
    /// Return the opaque Virtualization.framework machine identifier data.
    ///
    /// Callers that save snapshots should persist this value with the snapshot
    /// and rebuild future configs with [`VmConfigBuilder::machine_identifier`].
    #[must_use]
    pub fn machine_identifier(&self) -> &[u8] {
        &self.machine_identifier
    }
    /// Return guest network MAC addresses in network declaration order.
    ///
    /// Callers that save snapshots should persist this value with the snapshot
    /// and rebuild future configs with [`VmConfigBuilder::network_macs`].
    #[must_use]
    pub fn network_macs(&self) -> &[String] {
        &self.network_macs
    }
}
impl Default for VmConfig {
    fn default() -> Self {
        Self::builder()
            .build()
            .expect("default VM config should be valid")
    }
}
/// Builder for [`VmConfig`].
#[derive(Clone, Debug)]
pub struct VmConfigBuilder {
    pub(crate) cpus: NonZeroU32,
    pub(crate) memory: Size,
    networks: Vec<Network>,
    virtiofs_shares: Vec<VirtiofsShare>,
    block_devices: Vec<BlockDevice>,
    rosetta: bool,
    nested_virtualization: bool,
    boot_log: BootLog,
    kernel: KernelImage,
    init_block: Option<PathBuf>,
    cmdline_extra: Vec<String>,
    machine_identifier: Option<Vec<u8>>,
    pub(crate) network_macs: Option<Vec<String>>,
}
impl VmConfigBuilder {
    /// Set the CPU count.
    #[must_use]
    pub fn cpus(mut self, cpus: NonZeroU32) -> Self {
        self.cpus = cpus;
        self
    }
    /// Set the VM memory size.
    #[must_use]
    pub const fn memory(mut self, memory: Size) -> Self {
        self.memory = memory;
        self
    }
    /// Append a network attachment.
    #[must_use]
    pub fn network(mut self, network: Network) -> Self {
        self.networks.push(network);
        self
    }
    /// Replace network attachments.
    #[must_use]
    pub fn networks(mut self, networks: impl IntoIterator<Item = Network>) -> Self {
        self.networks = networks.into_iter().collect();
        self
    }
    /// Append a virtiofs share.
    #[must_use]
    pub fn virtiofs_share(mut self, tag: VirtiofsTag, host: impl Into<PathBuf>) -> Self {
        self.virtiofs_shares.push(VirtiofsShare {
            tag,
            host_path: host.into(),
        });
        self
    }
    /// Pre-declare a block device and return its typed handle.
    #[must_use]
    pub fn block_device(mut self, path: impl Into<PathBuf>) -> (Self, BlockDeviceId) {
        let id;
        (self, id) = self.push_block_device(path, DiskImageFormat::Raw, false);
        (self, id)
    }
    /// Pre-declare a writable local disk image and return its typed handle.
    #[must_use]
    pub fn local_disk_image(
        mut self,
        path: impl Into<PathBuf>,
        format: DiskImageFormat,
    ) -> (Self, BlockDeviceId) {
        let id;
        (self, id) = self.push_block_device(path, format, false);
        (self, id)
    }
    /// Pre-declare a read-only local disk image and return its typed handle.
    #[must_use]
    pub fn readonly_local_disk_image(
        mut self,
        path: impl Into<PathBuf>,
        format: DiskImageFormat,
    ) -> (Self, BlockDeviceId) {
        let id;
        (self, id) = self.push_block_device(path, format, true);
        (self, id)
    }
    /// Pre-declare a writable ASIF local disk image and return its typed handle.
    #[must_use]
    pub fn asif_disk_image(self, path: impl Into<PathBuf>) -> (Self, BlockDeviceId) {
        self.local_disk_image(path, DiskImageFormat::Asif)
    }
    /// Pre-declare a read-only ASIF local disk image and return its typed handle.
    #[must_use]
    pub fn readonly_asif_disk_image(self, path: impl Into<PathBuf>) -> (Self, BlockDeviceId) {
        self.readonly_local_disk_image(path, DiskImageFormat::Asif)
    }
    fn push_block_device(
        mut self,
        path: impl Into<PathBuf>,
        disk_image_format: DiskImageFormat,
        read_only: bool,
    ) -> (Self, BlockDeviceId) {
        let slot_value = u32::try_from(self.block_devices.len() + 1).unwrap_or(u32::MAX);
        let slot = NonZeroU32::new(slot_value).unwrap_or(NonZeroU32::MIN);
        let id = BlockDeviceId::from_slot(slot);
        self.block_devices.push(BlockDevice {
            id,
            path: path.into(),
            disk_image_format,
            read_only,
        });
        (self, id)
    }
    /// Enable or disable Rosetta support.
    #[must_use]
    pub const fn rosetta(mut self, enabled: bool) -> Self {
        self.rosetta = enabled;
        self
    }
    /// Enable or disable nested virtualization.
    #[must_use]
    pub const fn nested_virtualization(mut self, enabled: bool) -> Self {
        self.nested_virtualization = enabled;
        self
    }
    /// Set the boot-log sink.
    #[must_use]
    pub fn boot_log(mut self, boot_log: BootLog) -> Self {
        self.boot_log = boot_log;
        self
    }
    /// Set the kernel image.
    #[must_use]
    pub fn kernel(mut self, kernel: KernelImage) -> Self {
        self.kernel = kernel;
        self
    }
    /// Set the vminitd init-block image path.
    #[must_use]
    pub fn init_block(mut self, path: impl Into<PathBuf>) -> Self {
        self.init_block = Some(path.into());
        self
    }
    /// Append an extra kernel command-line fragment.
    #[must_use]
    pub fn cmdline_extra(mut self, value: impl Into<String>) -> Self {
        self.cmdline_extra.push(value.into());
        self
    }
    /// Set the opaque Virtualization.framework machine identifier data.
    ///
    /// Snapshot restore requires this value to match the machine identifier
    /// used by the VM that created the saved state.
    #[must_use]
    pub fn machine_identifier(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.machine_identifier = Some(data.into());
        self
    }
    /// Set guest network MAC addresses in network declaration order.
    ///
    /// Snapshot restore requires these values to match the network devices
    /// used by the VM that created the saved state.
    #[must_use]
    pub fn network_macs(mut self, macs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.network_macs = Some(macs.into_iter().map(Into::into).collect());
        self
    }
    /// Validate and build a VM configuration.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidConfig`] when the configuration violates a
    /// static constraint that can be checked before calling
    /// Virtualization.framework.
    pub fn build(self) -> Result<VmConfig> {
        if self.memory.as_bytes() < Size::mib(128).as_bytes() {
            return invalid_config("memory must be >= 128 MiB");
        }
        if self.networks.len() > 8 {
            return invalid_config("at most 8 network attachments are supported");
        }
        for network in &self.networks {
            if let Network::VmnetShared {
                subnet: Some(subnet),
            } = network
            {
                validate_vmnet_subnet(subnet)?;
            }
        }
        if self.block_devices.len() > 63 {
            return invalid_config("at most 63 block devices are supported");
        }
        let mut tags = HashSet::new();
        for share in &self.virtiofs_shares {
            if !tags.insert(share.tag.as_str().to_owned()) {
                return invalid_config(format!("virtiofs tag {} used twice", share.tag));
            }
        }
        for block_device in &self.block_devices {
            if std::fs::File::open(&block_device.path).is_err() {
                return invalid_config(format!(
                    "block_device: {} not accessible",
                    block_device.path.display()
                ));
            }
        }
        if let KernelImage::File(path) = &self.kernel
            && std::fs::File::open(path).is_err()
        {
            return invalid_config(format!("kernel: {} not accessible", path.display()));
        }
        if let Some(init_block) = &self.init_block
            && std::fs::File::open(init_block).is_err()
        {
            return invalid_config(format!(
                "init_block: {} not accessible",
                init_block.display()
            ));
        }
        let machine_identifier = match self.machine_identifier {
            Some(data) if data.is_empty() => {
                return invalid_config("machine identifier cannot be empty");
            }
            Some(data) => data,
            None => default_machine_identifier(),
        };
        let network_macs = match self.network_macs {
            Some(macs) => {
                if macs.len() != self.networks.len() {
                    return invalid_config(format!(
                        "network MAC count {} does not match network count {}",
                        macs.len(),
                        self.networks.len()
                    ));
                }
                for mac in &macs {
                    validate_mac_address(mac)?;
                }
                macs
            }
            None => default_network_macs(self.networks.len()),
        };
        Ok(VmConfig {
            cpus: self.cpus,
            memory: self.memory,
            networks: self.networks,
            virtiofs_shares: self.virtiofs_shares,
            block_devices: self.block_devices,
            rosetta: self.rosetta,
            nested_virtualization: self.nested_virtualization,
            boot_log: self.boot_log,
            kernel: self.kernel,
            init_block: self.init_block,
            cmdline_extra: self.cmdline_extra,
            machine_identifier,
            network_macs,
        })
    }
}
impl Default for VmConfigBuilder {
    fn default() -> Self {
        Self {
            cpus: NonZeroU32::new(4).expect("4 is nonzero"),
            memory: Size::gib(1),
            networks: vec![Network::Nat],
            virtiofs_shares: Vec::new(),
            block_devices: Vec::new(),
            rosetta: false,
            nested_virtualization: false,
            boot_log: BootLog::None,
            kernel: KernelImage::Bundled,
            init_block: None,
            cmdline_extra: Vec::new(),
            machine_identifier: None,
            network_macs: None,
        }
    }
}
#[cfg(target_vendor = "apple")]
fn default_machine_identifier() -> Vec<u8> {
    vz::new_machine_identifier_data()
}
#[cfg(not(target_vendor = "apple"))]
fn default_machine_identifier() -> Vec<u8> {
    b"firkin-non-apple-machine-identifier".to_vec()
}
