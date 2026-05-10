//! Runtime restart reconciliation execution tests.

use std::time::Duration;

use firkin_hygiene::{
    HostRuntimeScan, ReconciliationPlan, RestartResourceKind, RestartStateRecord,
};
use firkin_runtime::{
    HostProcessTerminationRequest, HostProcessTerminator, ReconciliationExecutor,
    ReconciliationRuntimeError, RestartReconciliation, RuntimeFilesystemReconciler,
    RuntimeRestartRecovery,
};

#[derive(Default)]
struct RecordingReconciliationExecutor {
    recovered: Vec<String>,
    cleaned: Vec<String>,
    quarantined: Vec<String>,
    fail_cleanup: bool,
}

#[derive(Default)]
struct RecordingHostProcessTerminator {
    terminated: Vec<u32>,
}

impl HostProcessTerminator for RecordingHostProcessTerminator {
    type Error = &'static str;

    fn terminate_process(
        &mut self,
        request: &HostProcessTerminationRequest,
    ) -> Result<(), Self::Error> {
        self.terminated.push(request.pid());
        Ok(())
    }
}

impl ReconciliationExecutor for RecordingReconciliationExecutor {
    type Error = &'static str;

    fn recover(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error> {
        self.recovered.push(record.id().to_owned());
        Ok(())
    }

    fn cleanup(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error> {
        self.cleaned.push(record.id().to_owned());
        if self.fail_cleanup {
            Err("cleanup failed")
        } else {
            Ok(())
        }
    }

    fn quarantine(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error> {
        self.quarantined.push(record.id().to_owned());
        Ok(())
    }
}

#[test]
fn restart_reconciliation_executes_recover_cleanup_and_quarantine_decisions() {
    let plan = ReconciliationPlan::from_records([
        RestartStateRecord::new("vm-1", RestartResourceKind::ActiveVm),
        RestartStateRecord::new("snapshot-1", RestartResourceKind::SnapshotArtifact),
        RestartStateRecord::new("log-1", RestartResourceKind::LogStream),
        RestartStateRecord::new("pid-1", RestartResourceKind::StaleRuntimeProcess),
        RestartStateRecord::new("", RestartResourceKind::SnapshotArtifact),
    ]);
    let mut executor = RecordingReconciliationExecutor::default();

    let report = RestartReconciliation::new(&plan)
        .execute(&mut executor)
        .expect("reconcile");

    assert_eq!(executor.recovered, vec!["vm-1", "snapshot-1"]);
    assert_eq!(executor.cleaned, vec!["log-1", "pid-1"]);
    assert_eq!(executor.quarantined, vec![""]);
    assert_eq!(report.recovered_count(), 2);
    assert_eq!(report.cleaned_count(), 2);
    assert_eq!(report.quarantined_count(), 1);
}

#[test]
fn restart_reconciliation_returns_adapter_failure_with_record_id() {
    let plan = ReconciliationPlan::from_records([
        RestartStateRecord::new("vm-1", RestartResourceKind::ActiveVm),
        RestartStateRecord::new("log-1", RestartResourceKind::LogStream),
    ]);
    let mut executor = RecordingReconciliationExecutor {
        fail_cleanup: true,
        ..RecordingReconciliationExecutor::default()
    };

    let error = RestartReconciliation::new(&plan)
        .execute(&mut executor)
        .expect_err("cleanup fails");

    assert!(matches!(
        error,
        ReconciliationRuntimeError::Apply { record_id, .. } if record_id == "log-1"
    ));
}

#[test]
fn restart_reconciliation_executes_from_host_runtime_scan() {
    let scan = HostRuntimeScan::new()
        .active_vm("vm-1", Duration::from_secs(30))
        .snapshot_artifact("snapshot-1")
        .log_stream("log-1")
        .stale_runtime_process("pid-1");
    let plan = scan.reconciliation_plan();
    let mut executor = RecordingReconciliationExecutor::default();

    let report = RestartReconciliation::new(&plan)
        .execute(&mut executor)
        .expect("reconcile from scan");

    assert_eq!(executor.recovered, vec!["vm-1", "snapshot-1"]);
    assert_eq!(executor.cleaned, vec!["log-1", "pid-1"]);
    assert_eq!(report.recovered_count(), 2);
    assert_eq!(report.cleaned_count(), 2);
}

#[test]
fn filesystem_reconciler_cleans_runtime_markers_and_quarantines_ambiguous_records() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let active_vms = tempdir.path().join("active-vms");
    let snapshots = tempdir.path().join("snapshots");
    let logs = tempdir.path().join("logs");
    let processes = tempdir.path().join("processes");
    let quarantine = tempdir.path().join("quarantine");
    std::fs::create_dir_all(&active_vms).expect("active vms");
    std::fs::create_dir_all(&snapshots).expect("snapshots");
    std::fs::create_dir_all(&logs).expect("logs");
    std::fs::create_dir_all(&processes).expect("processes");
    std::fs::write(active_vms.join("vm-live"), b"30").expect("vm marker");
    std::fs::write(snapshots.join("snapshot-1"), b"").expect("snapshot marker");
    std::fs::write(logs.join("runtime.log"), b"").expect("log marker");
    std::fs::write(processes.join("pid-123"), b"").expect("process marker");
    let plan = HostRuntimeScan::new()
        .active_vm("vm-live", Duration::from_secs(30))
        .snapshot_artifact("snapshot-1")
        .log_stream("runtime.log")
        .stale_runtime_process("pid-123")
        .snapshot_artifact("")
        .reconciliation_plan();
    let mut executor =
        RuntimeFilesystemReconciler::new(&active_vms, &snapshots, &logs, &processes, &quarantine);

    let report = RestartReconciliation::new(&plan)
        .execute(&mut executor)
        .expect("filesystem reconcile");

    assert!(active_vms.join("vm-live").exists());
    assert!(snapshots.join("snapshot-1").exists());
    assert!(!logs.join("runtime.log").exists());
    assert!(!processes.join("pid-123").exists());
    assert!(
        quarantine
            .join("snapshot_artifact")
            .join("ambiguous")
            .exists()
    );
    assert_eq!(report.recovered_count(), 2);
    assert_eq!(report.cleaned_count(), 2);
    assert_eq!(report.quarantined_count(), 1);
}

#[test]
fn runtime_restart_recovery_scans_reconciles_and_cleans_stuck_vms() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let active_vms = tempdir.path().join("active-vms");
    let snapshots = tempdir.path().join("snapshots");
    let logs = tempdir.path().join("logs");
    let processes = tempdir.path().join("processes");
    let quarantine = tempdir.path().join("quarantine");
    std::fs::create_dir_all(active_vms.join("vm-stuck")).expect("active marker");
    std::fs::create_dir_all(&snapshots).expect("snapshots");
    std::fs::create_dir_all(&logs).expect("logs");
    std::fs::create_dir_all(&processes).expect("processes");
    std::fs::write(active_vms.join("vm-stuck/heartbeat"), b"1").expect("heartbeat");
    std::fs::write(active_vms.join("vm-stuck/runtime.pid"), b"42").expect("pid");
    std::fs::write(active_vms.join("vm-stuck/runtime.executable"), "/bin/fk").expect("executable");
    std::fs::write(snapshots.join("snapshot-1"), b"").expect("snapshot marker");
    std::fs::write(logs.join("runtime.log"), b"").expect("log marker");
    std::fs::write(processes.join("pid-123"), b"").expect("process marker");
    let mut terminator = RecordingHostProcessTerminator::default();

    let report = RuntimeRestartRecovery::new(
        &active_vms,
        &snapshots,
        &logs,
        &processes,
        &quarantine,
        Duration::from_mins(5),
    )
    .execute_with_terminator(&mut terminator)
    .expect("runtime restart recovery");

    assert_eq!(report.restart().recovered_count(), 2);
    assert_eq!(report.restart().cleaned_count(), 2);
    assert_eq!(report.stuck_vm().cleaned_count(), 1);
    assert_eq!(terminator.terminated, vec![42]);
    assert!(!active_vms.join("vm-stuck").exists());
    assert!(snapshots.join("snapshot-1").exists());
    assert!(!logs.join("runtime.log").exists());
    assert!(!processes.join("pid-123").exists());
}
