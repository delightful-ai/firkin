//! Runtime-level snapshot restore orchestration tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
#[cfg(feature = "snapshot")]
use firkin_runtime::PersistedContainerSnapshotState;
#[cfg(feature = "snapshot")]
use firkin_runtime::core::ContainerSnapshotState;
use firkin_runtime::types::Size;
use firkin_runtime::{
    ActiveSessionReservation, DiskPressureProbe, RuntimeCubeSandboxCreate,
    RuntimeCubeSandboxCreateConfig, RuntimeCubeSandboxCreateError, RuntimeSnapshotRestore,
    SnapshotRestoreError, SnapshotRestoreRequest,
};
use {
    firkin_admission::{CapacityLedger, ResourceBudget},
    firkin_artifacts::{SnapshotArtifactIntegrity, SnapshotArtifactManifest},
};
use {
    firkin_e2b_contract::{
        PreparedTemplate, PreparedTemplateArtifactIntegrity, StartSandboxRequest,
    },
    firkin_e2b_wire::SandboxCreateRequest,
};

#[derive(Default)]
struct RecordingLauncher {
    restored_paths: Vec<PathBuf>,
    fail: bool,
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

#[async_trait]
impl firkin_runtime::SnapshotSessionLauncher for RecordingLauncher {
    type Error = &'static str;
    type Session = String;

    async fn restore_from_snapshot(
        &mut self,
        request: &SnapshotRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        self.restored_paths
            .push(request.manifest().path().to_path_buf());
        if self.fail {
            Err("restore failed")
        } else {
            Ok(format!("session:{}", request.manifest().logical_id()))
        }
    }
}

#[tokio::test]
async fn create_from_snapshot_reserves_active_capacity_and_records_latency() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let mut launcher = RecordingLauncher::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    };

    let report = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_disk_probe_elapsed(&mut launcher, Duration::from_millis(17), &mut disk_probe)
        .await
        .expect("restore succeeds");

    assert_eq!(report.session(), "session:repo-main");
    assert_eq!(launcher.restored_paths, vec![snapshot_path]);
    assert_eq!(ledger.active(), budget);
    assert_eq!(report.reservation().budget(), budget);
    assert_eq!(report.benchmark_samples().len(), 1);
    assert_eq!(
        report.benchmark_samples()[0].metric(),
        "warm_snapshot_restore"
    );
    assert!((report.benchmark_samples()[0].value() - 17.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn create_from_snapshot_releases_capacity_when_restore_fails() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = SnapshotArtifactManifest::base("repo-main", temp.path().join("repo-main.vz"));
    let mut launcher = RecordingLauncher {
        fail: true,
        ..RecordingLauncher::default()
    };
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    };

    let error = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_disk_probe_elapsed(&mut launcher, Duration::from_millis(17), &mut disk_probe)
        .await
        .expect_err("restore fails");

    assert!(matches!(error, SnapshotRestoreError::Launch { .. }));
    assert_eq!(
        ledger.active(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[tokio::test]
async fn create_from_snapshot_checks_host_disk_floor_before_capacity_and_launch() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let manifest = SnapshotArtifactManifest::base("repo-main", "/snapshots/repo-main.vz");
    let mut launcher = RecordingLauncher::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(9),
        probed_paths: Vec::new(),
    };

    let error = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_disk_probe_elapsed(&mut launcher, Duration::from_millis(17), &mut disk_probe)
        .await
        .expect_err("disk floor blocks restore");

    assert!(matches!(
        error,
        SnapshotRestoreError::Capacity(firkin_admission::CapacityError::Disk {
            requested,
            available,
        }) if requested == Size::gib(10) && available == Size::gib(9)
    ));
    assert_eq!(disk_probe.probed_paths, vec![PathBuf::from("/snapshots")]);
    assert!(launcher.restored_paths.is_empty());
    assert_eq!(
        ledger.active(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[tokio::test]
async fn create_from_snapshot_verifies_integrity_before_capacity_and_launch() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    std::fs::write(&snapshot_path, b"snapshot-before").expect("snapshot");
    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("integrity");
    std::fs::write(&snapshot_path, b"snapshot-after").expect("mutate snapshot");
    let mut launcher = RecordingLauncher::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    };

    let error = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_integrity_disk_probe_elapsed(
            &mut launcher,
            Duration::from_millis(17),
            &integrity,
            &mut disk_probe,
        )
        .await
        .expect_err("integrity mismatch blocks restore");

    assert!(matches!(error, SnapshotRestoreError::Integrity { .. }));
    assert!(launcher.restored_paths.is_empty());
    assert!(disk_probe.probed_paths.is_empty());
    assert_eq!(
        ledger.active(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[tokio::test]
async fn create_from_snapshot_with_matching_integrity_restores_session() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    std::fs::write(&snapshot_path, b"snapshot").expect("snapshot");
    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("integrity");
    let mut launcher = RecordingLauncher::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    };

    let report = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_integrity_disk_probe_elapsed(
            &mut launcher,
            Duration::from_millis(17),
            &integrity,
            &mut disk_probe,
        )
        .await
        .expect("integrity verified restore succeeds");

    assert_eq!(report.session(), "session:repo-main");
    assert_eq!(launcher.restored_paths, vec![snapshot_path]);
    assert_eq!(disk_probe.probed_paths, vec![temp.path().to_path_buf()]);
    assert_eq!(ledger.active(), budget);
}

#[tokio::test]
async fn create_from_snapshot_can_verify_integrity_sidecar_before_restore() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    std::fs::write(&snapshot_path, b"snapshot").expect("snapshot");
    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("integrity");
    let sidecar = SnapshotArtifactIntegrity::sidecar_path_for_artifact(&snapshot_path);
    integrity.write_json(&sidecar).expect("write sidecar");
    let mut launcher = RecordingLauncher::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    };

    let report = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_integrity_sidecar_disk_probe_elapsed(
            &mut launcher,
            Duration::from_millis(17),
            &sidecar,
            &mut disk_probe,
        )
        .await
        .expect("integrity sidecar verified restore succeeds");

    assert_eq!(report.session(), "session:repo-main");
    assert_eq!(launcher.restored_paths, vec![snapshot_path]);
    assert_eq!(disk_probe.probed_paths, vec![temp.path().to_path_buf()]);
    assert_eq!(ledger.active(), budget);
}

#[test]
fn active_session_reservation_releases_into_ledger_once() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    ledger.reserve_active(budget).expect("reservation fits");

    let mut reservation = ActiveSessionReservation::new(budget);
    assert_eq!(reservation.release_into(&mut ledger), Some(budget));
    assert_eq!(reservation.release_into(&mut ledger), None);
    assert_eq!(
        ledger.active(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[tokio::test]
async fn cube_sandbox_create_uses_prepared_snapshot_as_primary_create_path() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    std::fs::write(&snapshot_path, b"snapshot").expect("snapshot");
    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("integrity");
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: snapshot_path.display().to_string(),
            has_envd: true,
            artifact_integrity: Some(PreparedTemplateArtifactIntegrity {
                size_bytes: integrity.size_bytes(),
                sha256_hex: integrity.sha256_hex().to_owned(),
            }),
        }),
    };
    let config = RuntimeCubeSandboxCreateConfig::new(
        "sbx_firkin_1",
        "localhost",
        "firkin-envd",
        "2026-05-04T00:00:00Z",
        "2026-05-04T00:05:00Z",
        2,
        8192,
    );
    let mut launcher = RecordingLauncher::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    };

    let report = RuntimeCubeSandboxCreate::new(&mut ledger, &request, budget, config)
        .execute_with_disk_probe_elapsed(&mut launcher, Duration::from_millis(17), &mut disk_probe)
        .await
        .expect("create succeeds");

    assert_eq!(report.session(), "session:repo-main");
    assert_eq!(launcher.restored_paths, vec![snapshot_path]);
    assert_eq!(report.runtime_sandbox().config.sandbox_id, "sbx_firkin_1");
    assert_eq!(
        report.runtime_sandbox().exposed_ports,
        vec![49983, 49999, 50005]
    );
    assert_eq!(ledger.active(), budget);
    assert_eq!(report.reservation().budget(), budget);
    assert_eq!(
        report.benchmark_samples()[0].metric(),
        "warm_snapshot_restore"
    );
}

#[tokio::test]
async fn cube_sandbox_create_can_read_prepared_snapshot_integrity_sidecar() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    std::fs::write(&snapshot_path, b"snapshot").expect("snapshot");
    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    SnapshotArtifactIntegrity::from_file(&manifest)
        .expect("integrity")
        .write_json(SnapshotArtifactIntegrity::sidecar_path_for_artifact(
            &snapshot_path,
        ))
        .expect("write integrity sidecar");
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: snapshot_path.display().to_string(),
            has_envd: true,
            artifact_integrity: None,
        }),
    };
    let config = RuntimeCubeSandboxCreateConfig::new(
        "sbx_firkin_1",
        "localhost",
        "firkin-envd",
        "2026-05-04T00:00:00Z",
        "2026-05-04T00:05:00Z",
        2,
        8192,
    );
    let mut launcher = RecordingLauncher::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    };

    let report = RuntimeCubeSandboxCreate::new(&mut ledger, &request, budget, config)
        .execute_with_disk_probe_elapsed(&mut launcher, Duration::from_millis(17), &mut disk_probe)
        .await
        .expect("create succeeds with integrity sidecar");

    assert_eq!(report.session(), "session:repo-main");
    assert_eq!(launcher.restored_paths, vec![snapshot_path]);
    assert_eq!(ledger.active(), budget);
}

#[tokio::test]
async fn cube_sandbox_create_verifies_prepared_snapshot_integrity_before_launch() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    std::fs::write(&snapshot_path, b"snapshot-before").expect("snapshot");
    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("integrity");
    std::fs::write(&snapshot_path, b"snapshot-after").expect("mutate snapshot");
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: snapshot_path.display().to_string(),
            has_envd: true,
            artifact_integrity: Some(PreparedTemplateArtifactIntegrity {
                size_bytes: integrity.size_bytes(),
                sha256_hex: integrity.sha256_hex().to_owned(),
            }),
        }),
    };
    let config = RuntimeCubeSandboxCreateConfig::new(
        "sbx_firkin_1",
        "localhost",
        "firkin-envd",
        "2026-05-04T00:00:00Z",
        "2026-05-04T00:05:00Z",
        2,
        8192,
    );
    let mut launcher = RecordingLauncher::default();

    let error = RuntimeCubeSandboxCreate::new(&mut ledger, &request, budget, config)
        .execute_with_elapsed(&mut launcher, Duration::from_millis(17))
        .await
        .expect_err("integrity mismatch blocks cube create");

    assert!(matches!(
        error,
        RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::Integrity { .. })
    ));
    assert!(launcher.restored_paths.is_empty());
    assert_eq!(
        ledger.active(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[cfg(feature = "snapshot")]
#[test]
fn persisted_container_snapshot_state_round_trips_core_restore_state() {
    let state = ContainerSnapshotState::new(
        "/runtime/staging/repo-main",
        vec![1, 2, 3],
        vec!["02:00:00:00:00:01".to_owned()],
    );

    let persisted = PersistedContainerSnapshotState::from_snapshot_state(&state);
    let restored = persisted.to_snapshot_state();

    assert_eq!(restored.staging_dir(), state.staging_dir());
    assert_eq!(restored.machine_identifier(), state.machine_identifier());
    assert_eq!(restored.network_macs(), state.network_macs());
}
