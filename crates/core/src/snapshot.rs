//! snapshot — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::io::Streams;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::runtime::Container;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use std::time::Duration;
/// State needed to restore a saved implicit-VM container snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerSnapshotState {
    pub(crate) staging_dir: PathBuf,
    pub(crate) machine_identifier: Vec<u8>,
    pub(crate) network_macs: Vec<String>,
}
impl ContainerSnapshotState {
    /// Construct snapshot restore state.
    #[must_use]
    pub fn new(
        staging_dir: impl Into<PathBuf>,
        machine_identifier: Vec<u8>,
        network_macs: Vec<String>,
    ) -> Self {
        Self {
            staging_dir: staging_dir.into(),
            machine_identifier,
            network_macs,
        }
    }
    /// Return the persistent staging directory that backs the VM block devices.
    #[must_use]
    pub fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }
    /// Return the opaque Virtualization.framework machine identifier.
    #[must_use]
    pub fn machine_identifier(&self) -> &[u8] {
        &self.machine_identifier
    }
    /// Return guest network MAC addresses in network declaration order.
    #[must_use]
    pub fn network_macs(&self) -> &[String] {
        &self.network_macs
    }
}
/// Phase timings captured during VM snapshot restore.
#[cfg(feature = "snapshot")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContainerRestoreTimings {
    pub(crate) staging: Duration,
    vm_restore: Duration,
    vminitd_connect: Duration,
    pub(crate) rootfs_stage: Option<RestoredRootfsStage>,
}
#[cfg(feature = "snapshot")]
impl ContainerRestoreTimings {
    /// Construct restore phase timings.
    #[must_use]
    pub const fn new(
        staging: Duration,
        vm_restore: Duration,
        vminitd_connect: Duration,
        rootfs_stage: Option<RestoredRootfsStage>,
    ) -> Self {
        Self {
            staging,
            vm_restore,
            vminitd_connect,
            rootfs_stage,
        }
    }
    /// Return time spent preparing restored VM storage and configuration.
    #[must_use]
    pub const fn staging(self) -> Duration {
        self.staging
    }
    /// Return time spent in Virtualization.framework restore.
    #[must_use]
    pub const fn vm_restore(self) -> Duration {
        self.vm_restore
    }
    /// Return time spent waiting for vminitd to accept connections.
    #[must_use]
    pub const fn vminitd_connect(self) -> Duration {
        self.vminitd_connect
    }
    /// Return rootfs staging diagnostics for restored rootfs paths.
    #[must_use]
    pub const fn rootfs_stage(self) -> Option<RestoredRootfsStage> {
        self.rootfs_stage
    }
}
/// How a restored rootfs was staged.
#[cfg(feature = "snapshot")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RestoredRootfsStageMethod {
    /// Staged with a copy-on-write clone.
    Clone,
    /// Staged with a byte-for-byte copy.
    #[default]
    Copy,
    /// Rebuilt from an OCI bundle.
    Rebuild,
}
#[cfg(feature = "snapshot")]
impl RestoredRootfsStageMethod {
    /// Return the stable evidence-artifact spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clone => "clone",
            Self::Copy => "copy",
            Self::Rebuild => "rebuild",
        }
    }
}
/// Rootfs staging diagnostics captured during snapshot restore.
#[cfg(feature = "snapshot")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RestoredRootfsStage {
    method: RestoredRootfsStageMethod,
    source_bytes: u64,
    elapsed: Duration,
}
#[cfg(feature = "snapshot")]
impl RestoredRootfsStage {
    /// Construct a rootfs staging report.
    #[must_use]
    pub const fn new(
        method: RestoredRootfsStageMethod,
        source_bytes: u64,
        elapsed: Duration,
    ) -> Self {
        Self {
            method,
            source_bytes,
            elapsed,
        }
    }
    /// Return the rootfs staging method.
    #[must_use]
    pub const fn method(self) -> RestoredRootfsStageMethod {
        self.method
    }
    /// Return bytes in the source rootfs input.
    #[must_use]
    pub const fn source_bytes(self) -> u64 {
        self.source_bytes
    }
    /// Return elapsed time spent staging this rootfs input.
    #[must_use]
    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }
}
/// Restored implicit-VM container plus phase timings.
#[cfg(feature = "snapshot")]
#[derive(Debug)]
pub struct TimedContainerRestore {
    pub(crate) container: Container<Streams>,
    timings: ContainerRestoreTimings,
}
#[cfg(feature = "snapshot")]
impl TimedContainerRestore {
    /// Construct a timed restore result.
    #[must_use]
    pub const fn new(container: Container<Streams>, timings: ContainerRestoreTimings) -> Self {
        Self { container, timings }
    }
    /// Return restore phase timings.
    #[must_use]
    pub const fn timings(&self) -> ContainerRestoreTimings {
        self.timings
    }
    /// Consume into container and timings.
    #[must_use]
    pub fn into_parts(self) -> (Container<Streams>, ContainerRestoreTimings) {
        (self.container, self.timings)
    }
}
