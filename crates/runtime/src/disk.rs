//! disk — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_types::Size;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::process::Command;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Host disk-space probe used by runtime admission guards.
pub trait DiskPressureProbe {
    /// Error returned when the probe cannot read host free-space data.
    type Error;
    /// Return free disk space available at `path`.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific probe errors.
    fn available_disk(&mut self, path: &Path) -> Result<Size, Self::Error>;
}
/// Error returned by the host `df` disk-pressure probe.
#[derive(Debug, ThisError)]
pub enum HostDiskPressureProbeError {
    /// Failed to execute `df`.
    #[error("failed to execute df for host disk probe: {0}")]
    Command(#[source] std::io::Error),
    /// `df` exited unsuccessfully.
    #[error("df host disk probe failed: {status}")]
    CommandStatus {
        /// Command exit status.
        status: std::process::ExitStatus,
    },
    /// `df` output was not UTF-8.
    #[error("df host disk probe output was not UTF-8: {0}")]
    Utf8(#[source] std::string::FromUtf8Error),
    /// `df` output did not contain an available-kilobytes field.
    #[error("df host disk probe output missing available-kilobytes field")]
    MissingAvailable,
    /// Available-kilobytes field was not an integer.
    #[error("df host disk probe available-kilobytes field was invalid: {0}")]
    InvalidAvailable(#[source] std::num::ParseIntError),
}
/// Host free-space probe backed by `df -Pk`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostDiskPressureProbe;
impl HostDiskPressureProbe {
    /// Construct a host disk-pressure probe.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
    /// Parse `df -Pk` output and return available free space.
    ///
    /// # Errors
    ///
    /// Returns [`HostDiskPressureProbeError`] when the output does not include
    /// the expected available-kilobytes field.
    pub fn parse_df_available(output: &str) -> Result<Size, HostDiskPressureProbeError> {
        let available_kib = output
            .lines()
            .skip(1)
            .find_map(|line| line.split_whitespace().nth(3))
            .ok_or(HostDiskPressureProbeError::MissingAvailable)?
            .parse::<u64>()
            .map_err(HostDiskPressureProbeError::InvalidAvailable)?;
        Ok(Size::kib(available_kib))
    }
}
impl DiskPressureProbe for HostDiskPressureProbe {
    type Error = HostDiskPressureProbeError;
    fn available_disk(&mut self, path: &Path) -> Result<Size, Self::Error> {
        let output = Command::new("df")
            .arg("-Pk")
            .arg(path)
            .output()
            .map_err(HostDiskPressureProbeError::Command)?;
        if !output.status.success() {
            return Err(HostDiskPressureProbeError::CommandStatus {
                status: output.status,
            });
        }
        let stdout = String::from_utf8(output.stdout).map_err(HostDiskPressureProbeError::Utf8)?;
        Self::parse_df_available(&stdout)
    }
}
/// Host disk pressure guard error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum DiskPressureError<E> {
    /// The host disk probe failed.
    #[error("disk pressure probe failed: {source}")]
    Probe {
        /// Source probe error.
        source: E,
    },
    /// Available disk space is below the required floor.
    #[error("available disk space {available} is below required minimum {minimum}")]
    BelowMinimum {
        /// Required minimum free space.
        minimum: Size,
        /// Observed available free space.
        available: Size,
    },
}
/// Report from a successful host disk-pressure guard check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskPressureReport {
    pub(crate) root: PathBuf,
    minimum_free: Size,
    available_free: Size,
}
impl DiskPressureReport {
    /// Construct a disk-pressure report.
    #[must_use]
    pub fn new(root: PathBuf, minimum_free: Size, available_free: Size) -> Self {
        Self {
            root,
            minimum_free,
            available_free,
        }
    }
    /// Return the checked runtime root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Return the required free-space floor.
    #[must_use]
    pub const fn minimum_free(&self) -> Size {
        self.minimum_free
    }
    /// Return observed available free space.
    #[must_use]
    pub const fn available_free(&self) -> Size {
        self.available_free
    }
}
/// Runtime guard that blocks work when host disk free space is below a floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeDiskPressureGuard {
    pub(crate) root: PathBuf,
    minimum_free: Size,
}
impl RuntimeDiskPressureGuard {
    /// Construct a runtime disk-pressure guard.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, minimum_free: Size) -> Self {
        Self {
            root: root.into(),
            minimum_free,
        }
    }
    /// Return the checked runtime root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Return the required free-space floor.
    #[must_use]
    pub const fn minimum_free(&self) -> Size {
        self.minimum_free
    }
    /// Check that the host still has at least the configured free-space floor.
    ///
    /// # Errors
    ///
    /// Returns [`DiskPressureError`] when probing fails or available free space
    /// is below the configured floor.
    pub fn check<P>(&self, probe: &mut P) -> Result<DiskPressureReport, DiskPressureError<P::Error>>
    where
        P: DiskPressureProbe,
    {
        let available_free = probe
            .available_disk(&self.root)
            .map_err(|source| DiskPressureError::Probe { source })?;
        if available_free < self.minimum_free {
            return Err(DiskPressureError::BelowMinimum {
                minimum: self.minimum_free,
                available: available_free,
            });
        }
        Ok(DiskPressureReport::new(
            self.root.clone(),
            self.minimum_free,
            available_free,
        ))
    }
}
