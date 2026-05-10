//! preflight — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::disk::{
    DiskPressureError, DiskPressureProbe, DiskPressureReport, HostDiskPressureProbe,
    RuntimeDiskPressureGuard,
};
#[allow(unused_imports)]
use firkin_types::Size;
#[allow(unused_imports)]
use std::fmt::Display;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Runtime startup preflight that validates required roots and host disk floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePreflight {
    pub(crate) snapshot_root: PathBuf,
    pub(crate) log_root: PathBuf,
    minimum_free_disk: Size,
}
impl RuntimePreflight {
    /// Construct a runtime preflight check.
    #[must_use]
    pub fn new(
        snapshot_root: impl Into<PathBuf>,
        log_root: impl Into<PathBuf>,
        minimum_free_disk: Size,
    ) -> Self {
        Self {
            snapshot_root: snapshot_root.into(),
            log_root: log_root.into(),
            minimum_free_disk,
        }
    }
    /// Return the required snapshot root.
    #[must_use]
    pub fn snapshot_root(&self) -> &Path {
        &self.snapshot_root
    }
    /// Return the required log root.
    #[must_use]
    pub fn log_root(&self) -> &Path {
        &self.log_root
    }
    /// Return the host free-space floor enforced for runtime roots.
    #[must_use]
    pub const fn minimum_free_disk(&self) -> Size {
        self.minimum_free_disk
    }
    /// Run startup preflight using the host disk-pressure probe.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimePreflightError`] when a required root is missing, the
    /// disk probe fails, or host free space is below the configured floor.
    pub fn check(&self) -> Result<RuntimePreflightReport, RuntimePreflightError> {
        let mut probe = HostDiskPressureProbe::new();
        self.check_with_disk_probe(&mut probe)
    }
    /// Run startup preflight using an injected disk-pressure probe.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimePreflightError`] when a required root is missing, the
    /// disk probe fails, or host free space is below the configured floor.
    pub fn check_with_disk_probe<P>(
        &self,
        disk_probe: &mut P,
    ) -> Result<RuntimePreflightReport, RuntimePreflightError>
    where
        P: DiskPressureProbe,
        P::Error: Display,
    {
        Self::check_required_root(&self.snapshot_root)?;
        Self::check_required_root(&self.log_root)?;
        let snapshot_disk =
            RuntimeDiskPressureGuard::new(self.snapshot_root.clone(), self.minimum_free_disk)
                .check(disk_probe)
                .map_err(|error| RuntimePreflightError::from_disk(&error))?;
        let log_disk = RuntimeDiskPressureGuard::new(self.log_root.clone(), self.minimum_free_disk)
            .check(disk_probe)
            .map_err(|error| RuntimePreflightError::from_disk(&error))?;
        Ok(RuntimePreflightReport::new(
            self.snapshot_root.clone(),
            self.log_root.clone(),
            snapshot_disk,
            log_disk,
        ))
    }
    fn check_required_root(path: &Path) -> Result<(), RuntimePreflightError> {
        if path.is_dir() {
            Ok(())
        } else {
            Err(RuntimePreflightError::MissingRoot {
                path: path.to_path_buf(),
            })
        }
    }
}
/// Successful runtime startup preflight report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePreflightReport {
    pub(crate) snapshot_root: PathBuf,
    pub(crate) log_root: PathBuf,
    snapshot_disk: DiskPressureReport,
    log_disk: DiskPressureReport,
}
impl RuntimePreflightReport {
    /// Construct a runtime preflight report.
    #[must_use]
    pub fn new(
        snapshot_root: PathBuf,
        log_root: PathBuf,
        snapshot_disk: DiskPressureReport,
        log_disk: DiskPressureReport,
    ) -> Self {
        Self {
            snapshot_root,
            log_root,
            snapshot_disk,
            log_disk,
        }
    }
    /// Return the checked snapshot root.
    #[must_use]
    pub fn snapshot_root(&self) -> &Path {
        &self.snapshot_root
    }
    /// Return the checked log root.
    #[must_use]
    pub fn log_root(&self) -> &Path {
        &self.log_root
    }
    /// Return the snapshot-root disk-pressure report.
    #[must_use]
    pub const fn snapshot_disk(&self) -> &DiskPressureReport {
        &self.snapshot_disk
    }
    /// Return the log-root disk-pressure report.
    #[must_use]
    pub const fn log_disk(&self) -> &DiskPressureReport {
        &self.log_disk
    }
}
/// Runtime startup preflight error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum RuntimePreflightError {
    /// A required runtime root does not exist as a directory.
    #[error("required runtime root is missing: {path}")]
    MissingRoot {
        /// Missing runtime root path.
        path: PathBuf,
    },
    /// Host disk pressure blocked runtime startup.
    #[error("runtime disk preflight failed: {reason}")]
    Disk {
        /// Human-readable disk-pressure failure.
        reason: String,
    },
}
impl RuntimePreflightError {
    fn from_disk<E>(error: &DiskPressureError<E>) -> Self
    where
        E: Display,
    {
        Self::Disk {
            reason: error.to_string(),
        }
    }
}
