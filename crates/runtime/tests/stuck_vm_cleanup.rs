//! Runtime stuck-VM cleanup execution tests.

use std::time::Duration;

use firkin_hygiene::{HostRuntimeScan, StuckVmCleanupPlan, StuckVmObservation};
use firkin_runtime::{
    CommandHostProcessTerminateError, CommandHostProcessTerminator, HostProcessTerminationRequest,
    HostProcessTerminator, RuntimeFilesystemReconciler, RuntimeHostProcessStuckVmCleaner,
    RuntimeStuckVmCleanup, StuckVmCleaner, StuckVmRuntimeError,
};

#[derive(Default)]
struct RecordingStuckVmCleaner {
    preserved: Vec<String>,
    cleaned: Vec<String>,
    quarantined: Vec<String>,
    fail_cleanup: bool,
}

impl StuckVmCleaner for RecordingStuckVmCleaner {
    type Error = &'static str;

    fn preserve(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        self.preserved.push(vm_id.to_owned());
        Ok(())
    }

    fn cleanup(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        self.cleaned.push(vm_id.to_owned());
        if self.fail_cleanup {
            Err("cleanup failed")
        } else {
            Ok(())
        }
    }

    fn quarantine(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        self.quarantined.push(vm_id.to_owned());
        Ok(())
    }
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

#[test]
fn runtime_stuck_vm_cleanup_applies_plan_decisions_in_order() {
    let plan = StuckVmCleanupPlan::from_observations(
        [
            StuckVmObservation::new("vm-old", Duration::from_mins(10)),
            StuckVmObservation::new("vm-recent", Duration::from_secs(30)),
            StuckVmObservation::new("", Duration::from_mins(10)),
        ],
        Duration::from_mins(5),
    );
    let mut cleaner = RecordingStuckVmCleaner::default();

    let report = RuntimeStuckVmCleanup::new(&plan)
        .execute(&mut cleaner)
        .expect("cleanup");

    assert_eq!(cleaner.cleaned, vec!["vm-old"]);
    assert_eq!(cleaner.preserved, vec!["vm-recent"]);
    assert_eq!(cleaner.quarantined, vec![""]);
    assert_eq!(report.cleaned_count(), 1);
    assert_eq!(report.preserved_count(), 1);
    assert_eq!(report.quarantined_count(), 1);
}

#[test]
fn runtime_stuck_vm_cleanup_returns_cleaner_failure_with_vm_id() {
    let plan = StuckVmCleanupPlan::from_observations(
        [StuckVmObservation::new("vm-old", Duration::from_mins(10))],
        Duration::from_mins(5),
    );
    let mut cleaner = RecordingStuckVmCleaner {
        fail_cleanup: true,
        ..RecordingStuckVmCleaner::default()
    };

    let error = RuntimeStuckVmCleanup::new(&plan)
        .execute(&mut cleaner)
        .expect_err("cleanup fails");

    assert!(matches!(
        error,
        StuckVmRuntimeError::Apply { vm_id, .. } if vm_id == "vm-old"
    ));
}

#[test]
fn runtime_stuck_vm_cleanup_executes_from_host_runtime_scan() {
    let scan = HostRuntimeScan::new()
        .active_vm("vm-old", Duration::from_mins(10))
        .active_vm("vm-recent", Duration::from_secs(30));
    let plan = scan.stuck_vm_cleanup_plan(Duration::from_mins(5));
    let mut cleaner = RecordingStuckVmCleaner::default();

    let report = RuntimeStuckVmCleanup::new(&plan)
        .execute(&mut cleaner)
        .expect("cleanup from scan");

    assert_eq!(cleaner.cleaned, vec!["vm-old"]);
    assert_eq!(cleaner.preserved, vec!["vm-recent"]);
    assert_eq!(report.cleaned_count(), 1);
    assert_eq!(report.preserved_count(), 1);
}

#[test]
fn filesystem_reconciler_cleans_stuck_vm_markers_and_quarantines_ambiguous_records() {
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
    std::fs::write(active_vms.join("vm-old"), b"600").expect("old vm marker");
    std::fs::write(active_vms.join("vm-recent"), b"10").expect("recent vm marker");
    let plan = StuckVmCleanupPlan::from_observations(
        [
            StuckVmObservation::new("vm-old", Duration::from_mins(10)),
            StuckVmObservation::new("vm-recent", Duration::from_secs(10)),
            StuckVmObservation::new("", Duration::from_mins(10)),
        ],
        Duration::from_mins(5),
    );
    let mut cleaner =
        RuntimeFilesystemReconciler::new(&active_vms, &snapshots, &logs, &processes, &quarantine);

    let report = RuntimeStuckVmCleanup::new(&plan)
        .execute(&mut cleaner)
        .expect("filesystem stuck VM cleanup");

    assert!(!active_vms.join("vm-old").exists());
    assert!(active_vms.join("vm-recent").exists());
    assert!(quarantine.join("active_vm").join("ambiguous").exists());
    assert_eq!(report.cleaned_count(), 1);
    assert_eq!(report.preserved_count(), 1);
    assert_eq!(report.quarantined_count(), 1);
}

#[test]
fn host_process_stuck_vm_cleaner_terminates_marked_pid_before_marker_cleanup() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let active_vms = tempdir.path().join("active-vms");
    let snapshots = tempdir.path().join("snapshots");
    let logs = tempdir.path().join("logs");
    let processes = tempdir.path().join("processes");
    let quarantine = tempdir.path().join("quarantine");
    let marker = active_vms.join("vm-old");
    std::fs::create_dir_all(&marker).expect("active marker");
    std::fs::create_dir_all(&snapshots).expect("snapshots");
    std::fs::create_dir_all(&logs).expect("logs");
    std::fs::create_dir_all(&processes).expect("processes");
    std::fs::write(marker.join("heartbeat"), b"600").expect("heartbeat");
    std::fs::write(marker.join("runtime.pid"), b"42").expect("pid");
    std::fs::write(marker.join("runtime.executable"), "/bin/fk").expect("executable");
    let plan = StuckVmCleanupPlan::from_observations(
        [StuckVmObservation::with_runtime_pid(
            "vm-old",
            Duration::from_mins(10),
            42,
        )],
        Duration::from_mins(5),
    );
    let marker_cleaner =
        RuntimeFilesystemReconciler::new(&active_vms, &snapshots, &logs, &processes, &quarantine);
    let terminator = RecordingHostProcessTerminator::default();
    let mut cleaner = RuntimeHostProcessStuckVmCleaner::new(marker_cleaner, terminator);

    let report = RuntimeStuckVmCleanup::new(&plan)
        .execute(&mut cleaner)
        .expect("host process cleanup");

    assert_eq!(cleaner.terminator().terminated, vec![42]);
    assert!(!active_vms.join("vm-old").exists());
    assert_eq!(report.cleaned_count(), 1);
}

#[test]
fn command_host_process_terminator_refuses_executable_mismatch() {
    let mut terminator = CommandHostProcessTerminator;
    let request = HostProcessTerminationRequest::new(std::process::id(), "/definitely/not-fk");

    let error = terminator
        .terminate_process(&request)
        .expect_err("mismatch refuses to signal");

    assert!(matches!(
        error,
        CommandHostProcessTerminateError::ExecutableMismatch { pid, .. }
            if pid == std::process::id()
    ));
}
