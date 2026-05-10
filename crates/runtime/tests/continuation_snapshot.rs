//! Runtime continuation snapshot capture tests.
#![cfg(feature = "snapshot")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use firkin_runtime::{
    ContinuationSnapshotError, DiskPressureProbe, RuntimeContinuationSnapshotCapture,
    RuntimeContinuationSnapshotRestore, RuntimeCubeSandboxFollowup,
    RuntimeCubeSandboxFollowupConfig, SnapshotRestoreRequest,
};
use firkin_template::{SnapshotSinkError, TemplateSnapshotSink};
use firkin_types::Size;
use {
    firkin_admission::{CapacityLedger, ResourceBudget},
    firkin_artifacts::{
        ContinuationSnapshotPlan, ContinuationSnapshotReason, SnapshotArtifactIntegrity,
        SnapshotArtifactKind, SnapshotArtifactManifest,
    },
};
use {firkin_e2b_contract::StartSandboxRequest, firkin_e2b_wire::SandboxCreateRequest};

#[derive(Default)]
struct RecordingSnapshotSink {
    saved_paths: std::sync::Mutex<Vec<PathBuf>>,
    fail: bool,
}

#[async_trait]
impl TemplateSnapshotSink for RecordingSnapshotSink {
    async fn save_snapshot(&self, path: &Path) -> Result<(), SnapshotSinkError> {
        self.saved_paths
            .lock()
            .expect("lock paths")
            .push(path.into());
        if self.fail {
            Err("snapshot failed".into())
        } else {
            std::fs::write(path, b"snapshot")
                .map_err(|source| Box::new(source) as SnapshotSinkError)?;
            Ok(())
        }
    }
}

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

#[derive(Default)]
struct RecordingLauncher {
    restored_paths: std::sync::Mutex<Vec<PathBuf>>,
}

#[async_trait]
impl firkin_runtime::SnapshotSessionLauncher for RecordingLauncher {
    type Error = &'static str;
    type Session = String;

    async fn restore_from_snapshot(
        &mut self,
        request: &SnapshotRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        self.restored_paths
            .lock()
            .expect("lock paths")
            .push(request.manifest().path().to_path_buf());
        Ok(format!("continued:{}", request.manifest().logical_id()))
    }
}

#[tokio::test]
async fn capture_continuation_snapshot_saves_manifest_and_records_latency() {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("session-1-followup.vz");
    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Idle,
        &snapshot_path,
    );
    let sink = RecordingSnapshotSink::default();

    let report = RuntimeContinuationSnapshotCapture::new(&plan)
        .execute_with_elapsed(&sink, Duration::from_millis(23))
        .await
        .expect("snapshot capture succeeds");

    assert_eq!(
        *sink.saved_paths.lock().expect("lock paths"),
        vec![snapshot_path.clone()]
    );
    assert_eq!(report.manifest().kind(), SnapshotArtifactKind::Continuation);
    assert_eq!(report.manifest().logical_id(), "session-1");
    assert_eq!(report.manifest().path(), snapshot_path.as_path());
    assert_eq!(
        SnapshotArtifactManifest::read_json(SnapshotArtifactManifest::sidecar_path_for_artifact(
            report.manifest().path()
        ))
        .expect("manifest sidecar"),
        *report.manifest()
    );
    SnapshotArtifactIntegrity::read_json(SnapshotArtifactIntegrity::sidecar_path_for_artifact(
        report.manifest().path(),
    ))
    .expect("integrity sidecar")
    .verify(report.manifest())
    .expect("integrity sidecar verifies");
    assert_eq!(report.reason(), ContinuationSnapshotReason::Idle);
    assert_eq!(report.benchmark_samples()[0].metric(), "snapshot_save");
    assert!((report.benchmark_samples()[0].value() - 23.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn capture_continuation_snapshot_checks_disk_before_snapshot_save() {
    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Idle,
        "/snapshots/session-1-followup.vz",
    );
    let sink = RecordingSnapshotSink::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(9),
        probed_paths: Vec::new(),
    };

    let error = RuntimeContinuationSnapshotCapture::new(&plan)
        .execute_with_disk_probe_elapsed(&sink, Duration::from_millis(23), &mut disk_probe)
        .await
        .expect_err("disk floor blocks continuation snapshot");

    assert!(matches!(
        error,
        ContinuationSnapshotError::Capacity(firkin_admission::CapacityError::Disk {
            requested,
            available,
        }) if requested == Size::gib(10) && available == Size::gib(9)
    ));
    assert_eq!(disk_probe.probed_paths, vec![PathBuf::from("/snapshots")]);
    assert!(sink.saved_paths.lock().expect("lock paths").is_empty());
}

#[tokio::test]
async fn capture_continuation_snapshot_returns_sink_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("session-1-followup.vz");
    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Stopped,
        &snapshot_path,
    );
    let sink = RecordingSnapshotSink {
        fail: true,
        ..RecordingSnapshotSink::default()
    };

    let error = RuntimeContinuationSnapshotCapture::new(&plan)
        .execute_with_elapsed(&sink, Duration::from_millis(23))
        .await
        .expect_err("snapshot capture fails");

    assert!(matches!(error, ContinuationSnapshotError::Snapshot { .. }));
}

#[tokio::test]
async fn restore_continuation_snapshot_preserves_reason_and_records_latency() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(4, Size::gib(32), Size::gib(256)));
    let budget = ResourceBudget::new(1, Size::gib(4), Size::gib(32));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("session-1-followup.vz");
    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Exited,
        &snapshot_path,
    );
    let mut launcher = RecordingLauncher::default();

    let report = RuntimeContinuationSnapshotRestore::new(&mut ledger, &plan, budget)
        .execute_with_elapsed(&mut launcher, Duration::from_millis(19))
        .await
        .expect("restore succeeds");

    assert_eq!(report.session(), "continued:session-1");
    assert_eq!(report.reason(), ContinuationSnapshotReason::Exited);
    assert_eq!(
        *launcher.restored_paths.lock().expect("lock paths"),
        vec![snapshot_path]
    );
    assert_eq!(ledger.active(), budget);
    assert_eq!(report.reservation().budget(), budget);
    assert_eq!(
        report.benchmark_samples()[0].metric(),
        "warm_snapshot_restore"
    );
    assert!((report.benchmark_samples()[0].value() - 19.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn cube_followup_create_restores_continuation_snapshot() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(4, Size::gib(32), Size::gib(256)));
    let budget = ResourceBudget::new(1, Size::gib(4), Size::gib(32));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("session-1-followup.vz");
    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Idle,
        &snapshot_path,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: None,
    };
    let config = RuntimeCubeSandboxFollowupConfig::new(
        "sbx_followup_1",
        "localhost",
        "firkin-envd",
        "2026-05-04T00:00:00Z",
        "2026-05-04T00:05:00Z",
        1,
        4096,
    );
    let mut launcher = RecordingLauncher::default();

    let report = RuntimeCubeSandboxFollowup::new(&mut ledger, &request, &plan, budget, config)
        .execute_with_elapsed(&mut launcher, Duration::from_millis(21))
        .await
        .expect("follow-up create succeeds");

    assert_eq!(report.session(), "continued:session-1");
    assert_eq!(report.reason(), ContinuationSnapshotReason::Idle);
    assert_eq!(
        *launcher.restored_paths.lock().expect("lock paths"),
        vec![snapshot_path]
    );
    assert_eq!(report.runtime_sandbox().config.sandbox_id, "sbx_followup_1");
    assert_eq!(
        report.runtime_sandbox().exposed_ports,
        vec![49983, 49999, 50005]
    );
    assert_eq!(ledger.active(), budget);
}
