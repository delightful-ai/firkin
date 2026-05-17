//! Disk pressure tests for the single-node runtime.

use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use firkin_single_node::{CachedDiskProbe, DiskPressureGuard, DiskProbe, Error, Result};
use firkin_types::Size;

#[derive(Debug)]
struct FixedDiskProbe {
    available: Size,
}

impl DiskProbe for FixedDiskProbe {
    fn available(&self, _path: &Path) -> Result<Size> {
        Ok(self.available)
    }
}

#[derive(Debug)]
struct CountingDiskProbe {
    available: Size,
    calls: AtomicUsize,
}

impl CountingDiskProbe {
    fn new(available: Size) -> Self {
        Self {
            available,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl DiskProbe for CountingDiskProbe {
    fn available(&self, _path: &Path) -> Result<Size> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.available)
    }
}

#[test]
fn disk_pressure_guard_rejects_below_minimum_floor() {
    let guard = DiskPressureGuard::with_probe(
        "/tmp/firkin-single-node",
        Size::gib(10),
        Arc::new(FixedDiskProbe {
            available: Size::gib(10) - Size::bytes(1),
        }),
    );

    let result = guard.check("sandbox create");

    assert!(matches!(
        result,
        Err(Error::DiskPressure(message))
            if message.contains("sandbox create")
                && message.contains("minimum free disk")
                && message.contains("/tmp/firkin-single-node")
    ));
}

#[test]
fn disk_pressure_guard_allows_at_minimum_floor() {
    let guard = DiskPressureGuard::with_probe(
        "/tmp/firkin-single-node",
        Size::gib(10),
        Arc::new(FixedDiskProbe {
            available: Size::gib(10),
        }),
    );

    guard.check("snapshot create").unwrap();
}

#[test]
fn cached_disk_probe_reuses_recent_sample_for_hot_path_checks() {
    let probe = Arc::new(CountingDiskProbe::new(Size::gib(32)));
    let cached = Arc::new(CachedDiskProbe::new(probe.clone(), Duration::from_secs(30)));
    let guard = DiskPressureGuard::with_probe("/tmp/firkin-single-node", Size::gib(10), cached);

    guard.check("sandbox create").unwrap();
    guard.check("snapshot create").unwrap();
    guard.check("template warm").unwrap();

    assert_eq!(probe.calls(), 1);
}
