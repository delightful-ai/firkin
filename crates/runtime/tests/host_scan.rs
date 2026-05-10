//! Runtime host scan adapter tests.

use std::fs;
use std::time::{Duration, UNIX_EPOCH};

use firkin_hygiene::{ReconciliationDecision, StuckVmCleanupDecision};
use firkin_runtime::{RuntimeHostScanError, RuntimeHostScanner};

fn write_active_marker(root: &std::path::Path, id: &str, heartbeat: &[u8], pid: &[u8]) {
    let marker = root.join(id);
    fs::create_dir_all(&marker).expect("active vm marker dir");
    fs::write(marker.join("heartbeat"), heartbeat).expect("vm heartbeat");
    fs::write(marker.join("runtime.pid"), pid).expect("runtime pid");
}

#[test]
fn runtime_host_scanner_discovers_filesystem_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let active_vms = temp.path().join("active-vms");
    let snapshots = temp.path().join("snapshots");
    let logs = temp.path().join("logs");
    let processes = temp.path().join("processes");
    fs::create_dir_all(&active_vms).expect("active vm dir");
    fs::create_dir_all(&snapshots).expect("snapshot dir");
    fs::create_dir_all(&logs).expect("log dir");
    fs::create_dir_all(&processes).expect("process dir");
    write_active_marker(&active_vms, "vm-live", b"970", b"41");
    write_active_marker(&active_vms, "vm-stuck", b"400", b"42");
    fs::write(snapshots.join("snapshot-1"), b"").expect("snapshot marker");
    fs::write(logs.join("log-1"), b"").expect("log marker");
    fs::write(processes.join("pid-123"), b"").expect("process marker");

    let scan = RuntimeHostScanner::new(&active_vms, &snapshots, &logs, &processes)
        .with_now(UNIX_EPOCH + Duration::from_secs(1_000))
        .scan()
        .expect("host scan");
    let reconciliation = scan.reconciliation_plan();
    let stuck_vms = scan.stuck_vm_cleanup_plan(Duration::from_mins(5));

    assert_eq!(
        reconciliation.decision_for("vm-live"),
        Some(ReconciliationDecision::Recover)
    );
    assert_eq!(
        reconciliation.decision_for("snapshot-1"),
        Some(ReconciliationDecision::Recover)
    );
    assert_eq!(
        reconciliation.decision_for("log-1"),
        Some(ReconciliationDecision::Cleanup)
    );
    assert_eq!(
        reconciliation.decision_for("pid-123"),
        Some(ReconciliationDecision::Cleanup)
    );
    assert_eq!(
        stuck_vms.decision_for("vm-live"),
        Some(StuckVmCleanupDecision::Preserve)
    );
    assert_eq!(
        stuck_vms.decision_for("vm-stuck"),
        Some(StuckVmCleanupDecision::Cleanup)
    );
}

#[test]
fn runtime_host_scanner_treats_missing_marker_roots_as_empty() {
    let temp = tempfile::tempdir().expect("tempdir");
    let active_vms = temp.path().join("missing-active-vms");
    let snapshots = temp.path().join("missing-snapshots");
    let logs = temp.path().join("missing-logs");
    let processes = temp.path().join("missing-processes");

    let scan = RuntimeHostScanner::new(&active_vms, &snapshots, &logs, &processes)
        .scan()
        .expect("missing roots are empty");

    assert_eq!(scan.reconciliation_plan().decisions().len(), 0);
    assert_eq!(
        scan.stuck_vm_cleanup_plan(Duration::from_mins(5))
            .decisions()
            .len(),
        0
    );
}

#[test]
fn runtime_host_scanner_rejects_invalid_heartbeat_markers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let active_vms = temp.path().join("active-vms");
    let snapshots = temp.path().join("snapshots");
    let logs = temp.path().join("logs");
    let processes = temp.path().join("processes");
    fs::create_dir_all(&active_vms).expect("active vm dir");
    fs::create_dir_all(&snapshots).expect("snapshot dir");
    fs::create_dir_all(&logs).expect("log dir");
    fs::create_dir_all(&processes).expect("process dir");
    write_active_marker(&active_vms, "vm-bad", b"not-seconds", b"42");

    let error = RuntimeHostScanner::new(&active_vms, &snapshots, &logs, &processes)
        .scan()
        .expect_err("invalid heartbeat rejects");

    assert!(matches!(
        error,
        RuntimeHostScanError::InvalidHeartbeat { path, .. }
            if path.parent()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()) == Some("vm-bad")
    ));
}

#[test]
fn runtime_host_scanner_reads_active_vm_marker_directory_with_runtime_pid() {
    let temp = tempfile::tempdir().expect("tempdir");
    let active_vms = temp.path().join("active-vms");
    let snapshots = temp.path().join("snapshots");
    let logs = temp.path().join("logs");
    let processes = temp.path().join("processes");
    let marker = active_vms.join("vm-stuck");
    fs::create_dir_all(&marker).expect("active vm marker dir");
    fs::create_dir_all(&snapshots).expect("snapshot dir");
    fs::create_dir_all(&logs).expect("log dir");
    fs::create_dir_all(&processes).expect("process dir");
    fs::write(marker.join("heartbeat"), b"400").expect("vm heartbeat");
    fs::write(marker.join("runtime.pid"), b"42").expect("runtime pid");

    let scan = RuntimeHostScanner::new(&active_vms, &snapshots, &logs, &processes)
        .with_now(UNIX_EPOCH + Duration::from_secs(1_000))
        .scan()
        .expect("host scan");
    let observation = &scan.stuck_vm_observations()[0];

    assert_eq!(observation.id(), "vm-stuck");
    assert_eq!(observation.heartbeat_age(), Duration::from_mins(10));
    assert_eq!(observation.runtime_pid(), Some(42));
}
