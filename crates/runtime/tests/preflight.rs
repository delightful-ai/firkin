//! Runtime preflight tests.

use std::path::{Path, PathBuf};

use firkin_runtime::types::Size;
use firkin_runtime::{DiskPressureProbe, RuntimePreflight, RuntimePreflightError};

struct RecordingDiskProbe {
    available: Size,
    probed_paths: Vec<PathBuf>,
}

impl DiskPressureProbe for RecordingDiskProbe {
    type Error = &'static str;

    fn available_disk(&mut self, path: &Path) -> Result<Size, Self::Error> {
        self.probed_paths.push(path.to_path_buf());
        Ok(self.available)
    }
}

#[test]
fn runtime_preflight_checks_required_roots_and_disk_floor() {
    let root = tempfile::tempdir().expect("tempdir");
    let snapshots = root.path().join("snapshots");
    let logs = root.path().join("logs");
    std::fs::create_dir_all(&snapshots).expect("snapshots");
    std::fs::create_dir_all(&logs).expect("logs");
    let mut probe = RecordingDiskProbe {
        available: Size::gib(32),
        probed_paths: Vec::new(),
    };

    let report = RuntimePreflight::new(&snapshots, &logs, Size::gib(10))
        .check_with_disk_probe(&mut probe)
        .expect("preflight passes");

    assert_eq!(report.snapshot_root(), snapshots.as_path());
    assert_eq!(report.log_root(), logs.as_path());
    assert_eq!(probe.probed_paths, vec![snapshots, logs]);
}

#[test]
fn runtime_preflight_rejects_missing_required_roots() {
    let root = tempfile::tempdir().expect("tempdir");
    let snapshots = root.path().join("snapshots");
    let logs = root.path().join("logs");
    std::fs::create_dir_all(&logs).expect("logs");
    let mut probe = RecordingDiskProbe {
        available: Size::gib(32),
        probed_paths: Vec::new(),
    };

    let error = RuntimePreflight::new(&snapshots, &logs, Size::gib(10))
        .check_with_disk_probe(&mut probe)
        .expect_err("missing root rejects");

    assert!(matches!(
        error,
        RuntimePreflightError::MissingRoot { path } if path == snapshots
    ));
    assert!(probe.probed_paths.is_empty());
}

#[test]
fn runtime_preflight_rejects_low_disk_before_runtime_work() {
    let root = tempfile::tempdir().expect("tempdir");
    let snapshots = root.path().join("snapshots");
    let logs = root.path().join("logs");
    std::fs::create_dir_all(&snapshots).expect("snapshots");
    std::fs::create_dir_all(&logs).expect("logs");
    let mut probe = RecordingDiskProbe {
        available: Size::gib(9),
        probed_paths: Vec::new(),
    };

    let error = RuntimePreflight::new(&snapshots, &logs, Size::gib(10))
        .check_with_disk_probe(&mut probe)
        .expect_err("disk floor rejects");

    assert!(matches!(error, RuntimePreflightError::Disk { .. }));
    assert_eq!(probe.probed_paths, vec![snapshots]);
}
