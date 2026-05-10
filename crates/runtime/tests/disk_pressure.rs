//! Runtime disk-pressure guard tests.

use std::path::{Path, PathBuf};

use firkin_runtime::types::Size;
use firkin_runtime::{
    DiskPressureError, DiskPressureProbe, HostDiskPressureProbe, RuntimeDiskPressureGuard,
};

struct RecordingDiskProbe {
    available: Size,
    fail: bool,
    probed_paths: Vec<PathBuf>,
}

impl Default for RecordingDiskProbe {
    fn default() -> Self {
        Self {
            available: Size::bytes(0),
            fail: false,
            probed_paths: Vec::new(),
        }
    }
}

impl DiskPressureProbe for RecordingDiskProbe {
    type Error = &'static str;

    fn available_disk(&mut self, path: &Path) -> Result<Size, Self::Error> {
        self.probed_paths.push(path.into());
        if self.fail {
            Err("probe failed")
        } else {
            Ok(self.available)
        }
    }
}

#[test]
fn disk_pressure_guard_reports_available_space_above_floor() {
    let mut probe = RecordingDiskProbe {
        available: Size::gib(24),
        ..RecordingDiskProbe::default()
    };

    let report = RuntimeDiskPressureGuard::new("/var/firkin", Size::gib(10))
        .check(&mut probe)
        .expect("disk floor passes");

    assert_eq!(probe.probed_paths, vec![PathBuf::from("/var/firkin")]);
    assert_eq!(report.root(), Path::new("/var/firkin"));
    assert_eq!(report.minimum_free(), Size::gib(10));
    assert_eq!(report.available_free(), Size::gib(24));
}

#[test]
fn disk_pressure_guard_rejects_space_below_floor() {
    let mut probe = RecordingDiskProbe {
        available: Size::gib(9),
        ..RecordingDiskProbe::default()
    };

    let error = RuntimeDiskPressureGuard::new("/var/firkin", Size::gib(10))
        .check(&mut probe)
        .expect_err("disk floor fails");

    assert!(matches!(
        error,
        DiskPressureError::BelowMinimum {
            minimum,
            available
        } if minimum == Size::gib(10) && available == Size::gib(9)
    ));
}

#[test]
fn disk_pressure_guard_returns_probe_failure() {
    let mut probe = RecordingDiskProbe {
        available: Size::gib(24),
        fail: true,
        ..RecordingDiskProbe::default()
    };

    let error = RuntimeDiskPressureGuard::new("/var/firkin", Size::gib(10))
        .check(&mut probe)
        .expect_err("probe fails");

    assert!(matches!(error, DiskPressureError::Probe { .. }));
}

#[test]
fn host_disk_pressure_probe_parses_df_kilobyte_output() {
    let output = "\
Filesystem 1024-blocks Used Available Capacity Mounted on
/dev/disk3s5 471000000 430000000 12345678 98% /System/Volumes/Data
";

    let available = HostDiskPressureProbe::parse_df_available(output).expect("parse df output");

    assert_eq!(available, Size::kib(12_345_678));
}

#[test]
fn host_disk_pressure_probe_reports_tempdir_available_space() {
    let root = tempfile::tempdir().expect("tempdir");
    let mut probe = HostDiskPressureProbe::new();

    let available = probe
        .available_disk(root.path())
        .expect("probe tempdir disk");

    assert!(available > Size::bytes(0));
}
