use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use firkin_types::Size;

use super::{Error, Result, SingleNodeConfig};

/// Host disk availability probe used before disk-consuming runtime operations.
pub trait DiskProbe: Send + Sync {
    /// Return available bytes for the filesystem containing `path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DiskPressure`] if availability cannot be measured.
    fn available(&self, path: &Path) -> Result<Size>;
}

/// `df -Pk`-backed disk availability probe.
#[derive(Clone, Debug, Default)]
pub struct DfDiskProbe;

impl DfDiskProbe {
    fn parse_available(output: &str) -> Result<Size> {
        let available_kib = output
            .lines()
            .skip(1)
            .find_map(|line| line.split_whitespace().nth(3))
            .ok_or_else(|| Error::DiskPressure("df output missing available-kib field".into()))?
            .parse::<u64>()
            .map_err(|error| {
                Error::DiskPressure(format!("df available-kib field is invalid: {error}"))
            })?;
        Ok(Size::bytes(available_kib.saturating_mul(1024)))
    }
}

impl DiskProbe for DfDiskProbe {
    fn available(&self, path: &Path) -> Result<Size> {
        let output = Command::new("df")
            .arg("-Pk")
            .arg(path)
            .output()
            .map_err(|error| {
                Error::DiskPressure(format!("run disk probe for {}: {error}", path.display()))
            })?;
        if !output.status.success() {
            return Err(Error::DiskPressure(format!(
                "disk probe for {} exited {}",
                path.display(),
                output.status
            )));
        }
        let stdout = String::from_utf8(output.stdout).map_err(|error| {
            Error::DiskPressure(format!(
                "disk probe output for {} is not utf-8: {error}",
                path.display()
            ))
        })?;
        Self::parse_available(&stdout)
    }
}

/// Cached disk probe wrapper for hot operation paths.
pub struct CachedDiskProbe {
    probe: Arc<dyn DiskProbe>,
    ttl: Duration,
    sample: Mutex<Option<CachedDiskSample>>,
}

impl CachedDiskProbe {
    /// Construct a cached probe around another disk probe.
    #[must_use]
    pub fn new(probe: Arc<dyn DiskProbe>, ttl: Duration) -> Self {
        Self {
            probe,
            ttl,
            sample: Mutex::new(None),
        }
    }

    /// Construct the default cached host probe.
    #[must_use]
    pub fn default_host() -> Self {
        Self::new(Arc::new(DfDiskProbe), Duration::from_secs(30))
    }

    fn lock_sample(&self) -> Result<std::sync::MutexGuard<'_, Option<CachedDiskSample>>> {
        self.sample
            .lock()
            .map_err(|_| Error::DiskPressure("disk probe cache lock poisoned".into()))
    }
}

impl DiskProbe for CachedDiskProbe {
    fn available(&self, path: &Path) -> Result<Size> {
        let mut sample = self.lock_sample()?;
        if let Some(sample) = sample.as_ref()
            && sample.path == path
            && sample.measured_at.elapsed() < self.ttl
        {
            return Ok(sample.available);
        }

        let available = self.probe.available(path)?;
        *sample = Some(CachedDiskSample {
            path: path.to_owned(),
            available,
            measured_at: Instant::now(),
        });
        Ok(available)
    }
}

#[derive(Clone, Debug)]
struct CachedDiskSample {
    path: PathBuf,
    available: Size,
    measured_at: Instant,
}

/// Disk-floor guard for local runtime state, snapshots, and warm-pool artifacts.
#[derive(Clone)]
pub struct DiskPressureGuard {
    path: PathBuf,
    minimum_available: Size,
    probe: Arc<dyn DiskProbe>,
}

impl DiskPressureGuard {
    /// Construct a guard using the default cached host disk probe.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, minimum_available: Size) -> Self {
        Self::with_probe(
            path,
            minimum_available,
            Arc::new(CachedDiskProbe::default_host()),
        )
    }

    /// Construct a guard from single-node runtime configuration.
    #[must_use]
    pub fn from_config(config: &SingleNodeConfig) -> Self {
        Self::new(config.root(), config.minimum_free_disk())
    }

    /// Construct a guard with an explicit disk probe.
    #[must_use]
    pub fn with_probe(
        path: impl Into<PathBuf>,
        minimum_available: Size,
        probe: Arc<dyn DiskProbe>,
    ) -> Self {
        Self {
            path: path.into(),
            minimum_available,
            probe,
        }
    }

    /// Check that the configured filesystem is at or above the disk floor.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DiskPressure`] when free space is below the configured floor.
    pub fn check(&self, operation: &str) -> Result<()> {
        let available = self.probe.available(&self.path)?;
        if available < self.minimum_available {
            return Err(Error::DiskPressure(format!(
                "single-node runtime disk pressure rejected {operation}: available {} bytes is below minimum free disk {} bytes at {}",
                available.as_bytes(),
                self.minimum_available.as_bytes(),
                self.path.display()
            )));
        }
        Ok(())
    }

    /// Return the guarded path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the configured minimum available disk.
    #[must_use]
    pub const fn minimum_available(&self) -> Size {
        self.minimum_available
    }
}
