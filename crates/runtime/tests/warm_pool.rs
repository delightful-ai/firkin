//! Runtime warm-pool orchestration tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use firkin_runtime::types::Size;
use firkin_runtime::{
    DiskPressureProbe, RuntimeSnapshotWarmPool, RuntimeWarmPoolCheckout, RuntimeWarmPoolMaintain,
    RuntimeWarmPoolService, RuntimeWarmPoolSupervisor, SnapshotRestoreRequest,
    SnapshotSessionLauncher, WarmPoolCheckoutError, WarmPoolCheckoutRequest,
    WarmPoolMaintenanceError, WarmPoolRestoreRequest, WarmPoolSessionCheckout,
    WarmPoolSessionLauncher,
};
use {
    firkin_admission::{
        CapacityLedger, ResourceBudget, WarmPoolEntry, WarmPoolKey, WarmPoolLedger,
        WarmPoolReplenishmentSkipReason, WarmPoolReplenishmentTarget,
    },
    firkin_artifacts::SnapshotArtifactManifest,
};

#[derive(Default)]
struct RecordingWarmLauncher {
    restored_paths: Vec<PathBuf>,
    fail: bool,
}

impl WarmPoolSessionLauncher for RecordingWarmLauncher {
    type Error = &'static str;
    type Session = String;

    fn restore_warm_pool_entry(
        &mut self,
        request: &WarmPoolRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        self.restored_paths
            .push(request.manifest().path().to_path_buf());
        if self.fail {
            Err("restore failed")
        } else {
            Ok(format!("warm:{}", request.key().repo()))
        }
    }
}

#[derive(Default)]
struct RecordingWarmCheckout {
    checked_out_snapshot_ids: Vec<String>,
    fail: bool,
}

#[derive(Default)]
struct AsyncRecordingSnapshotLauncher {
    restored_paths: Vec<PathBuf>,
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

fn ample_disk_probe() -> RecordingDiskProbe {
    RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    }
}

#[async_trait::async_trait]
impl SnapshotSessionLauncher for AsyncRecordingSnapshotLauncher {
    type Error = &'static str;
    type Session = String;

    async fn restore_from_snapshot(
        &mut self,
        request: &SnapshotRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        self.restored_paths
            .push(request.manifest().path().to_path_buf());
        Ok(format!("warm-live:{}", request.manifest().logical_id()))
    }
}

impl WarmPoolSessionCheckout for RecordingWarmCheckout {
    type Error = &'static str;
    type Session = String;

    fn checkout_warm_pool_entry(
        &mut self,
        request: &WarmPoolCheckoutRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        self.checked_out_snapshot_ids
            .push(request.entry().snapshot_id().to_owned());
        if self.fail {
            Err("checkout failed")
        } else {
            Ok(format!("active:{}", request.entry().key().repo()))
        }
    }
}

#[test]
fn maintain_warm_pool_restores_snapshot_and_reserves_warm_capacity() {
    let mut capacity = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let mut pool = WarmPoolLedger::default();
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let manifest = SnapshotArtifactManifest::base("snapshot-1", "/snapshots/repo-main.vz");
    let mut launcher = RecordingWarmLauncher::default();

    let report =
        RuntimeWarmPoolMaintain::new(&mut capacity, &mut pool, key.clone(), &manifest, budget)
            .execute(&mut launcher)
            .expect("maintain warm entry");

    assert_eq!(report.session(), "warm:repo-main");
    assert_eq!(
        launcher.restored_paths,
        vec![PathBuf::from("/snapshots/repo-main.vz")]
    );
    assert_eq!(report.entry().key(), &key);
    assert_eq!(report.entry().snapshot_id(), "snapshot-1");
    assert_eq!(capacity.warm_pool(), budget);
}

#[test]
fn maintain_warm_pool_does_not_reserve_capacity_when_restore_fails() {
    let mut capacity = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let mut pool = WarmPoolLedger::default();
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let manifest = SnapshotArtifactManifest::base("snapshot-1", "/snapshots/repo-main.vz");
    let mut launcher = RecordingWarmLauncher {
        fail: true,
        ..RecordingWarmLauncher::default()
    };

    let error = RuntimeWarmPoolMaintain::new(&mut capacity, &mut pool, key, &manifest, budget)
        .execute(&mut launcher)
        .expect_err("restore fails");

    assert!(matches!(error, WarmPoolMaintenanceError::Launch { .. }));
    assert_eq!(
        capacity.warm_pool(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[test]
fn checkout_warm_pool_promotes_capacity_and_records_latency() {
    let mut capacity = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let mut pool = WarmPoolLedger::default();
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    pool.maintain(
        WarmPoolEntry::new(key.clone(), "snapshot-1", budget),
        &mut capacity,
    )
    .expect("warm entry");
    let mut checkout = RecordingWarmCheckout::default();

    let report = RuntimeWarmPoolCheckout::new(&mut capacity, &mut pool, &key)
        .execute_with_elapsed(&mut checkout, Duration::from_millis(11))
        .expect("checkout warm entry")
        .expect("entry exists");

    assert_eq!(report.session(), "active:repo-main");
    assert_eq!(
        checkout.checked_out_snapshot_ids,
        vec!["snapshot-1".to_owned()]
    );
    assert_eq!(
        capacity.warm_pool(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
    assert_eq!(capacity.active(), budget);
    assert_eq!(report.reservation().budget(), budget);
    assert_eq!(report.benchmark_samples()[0].metric(), "warm_pool_checkout");
    assert!((report.benchmark_samples()[0].value() - 11.0).abs() < f64::EPSILON);
}

#[test]
fn checkout_warm_pool_reverts_capacity_when_checkout_fails() {
    let mut capacity = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let mut pool = WarmPoolLedger::default();
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    pool.maintain(
        WarmPoolEntry::new(key.clone(), "snapshot-1", budget),
        &mut capacity,
    )
    .expect("warm entry");
    let mut checkout = RecordingWarmCheckout {
        fail: true,
        ..RecordingWarmCheckout::default()
    };

    let error = RuntimeWarmPoolCheckout::new(&mut capacity, &mut pool, &key)
        .execute_with_elapsed(&mut checkout, Duration::from_millis(11))
        .expect("entry exists")
        .expect_err("checkout fails");

    assert!(matches!(error, WarmPoolCheckoutError::Checkout { .. }));
    assert_eq!(
        capacity.active(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
    assert_eq!(capacity.warm_pool(), budget);
}

#[tokio::test]
async fn snapshot_warm_pool_retains_restored_session_until_checkout() {
    let capacity = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let mut pool = RuntimeSnapshotWarmPool::new(capacity);
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let manifest = SnapshotArtifactManifest::base("snapshot-1", &snapshot_path);
    let mut launcher = AsyncRecordingSnapshotLauncher::default();
    let mut disk_probe = ample_disk_probe();

    let maintained = pool
        .maintain_with_disk_probe_elapsed(
            key.clone(),
            &manifest,
            budget,
            &mut launcher,
            Duration::from_millis(42),
            &mut disk_probe,
        )
        .await
        .expect("maintain warm snapshot");

    assert_eq!(maintained.session(), "warm-live:snapshot-1");
    assert_eq!(launcher.restored_paths, vec![snapshot_path]);
    assert_eq!(pool.capacity().warm_pool(), budget);
    assert!(pool.contains(&key));

    let checked_out = pool
        .checkout_with_elapsed(&key, Duration::from_millis(3))
        .expect("checkout warm snapshot")
        .expect("warm entry exists");

    assert_eq!(checked_out.session(), "warm-live:snapshot-1");
    assert_eq!(checked_out.entry().snapshot_id(), "snapshot-1");
    assert_eq!(pool.capacity().active(), budget);
    assert_eq!(
        pool.capacity().warm_pool(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
    assert!(!pool.contains(&key));
    assert_eq!(checked_out.reservation().budget(), budget);
    assert_eq!(
        checked_out.benchmark_samples()[0].metric(),
        "warm_pool_checkout"
    );
    assert!((checked_out.benchmark_samples()[0].value() - 3.0).abs() < f64::EPSILON);
}

#[test]
fn snapshot_warm_pool_empty_checkout_returns_none() {
    let capacity = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let mut pool = RuntimeSnapshotWarmPool::<String>::new(capacity);
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");

    let checked_out = pool
        .checkout_with_elapsed(&key, Duration::from_millis(3))
        .expect("checkout does not error");

    assert!(checked_out.is_none());
}

#[tokio::test]
async fn snapshot_warm_pool_capacity_failure_preserves_retained_entry() {
    let capacity = CapacityLedger::new(ResourceBudget::new(2, Size::gib(8), Size::gib(64)));
    let mut pool = RuntimeSnapshotWarmPool::new(capacity);
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let manifest = SnapshotArtifactManifest::base("snapshot-1", &snapshot_path);
    let mut launcher = AsyncRecordingSnapshotLauncher::default();
    let mut disk_probe = ample_disk_probe();

    pool.maintain_with_disk_probe_elapsed(
        key.clone(),
        &manifest,
        ResourceBudget::new(1, Size::gib(1), Size::gib(1)),
        &mut launcher,
        Duration::from_millis(1),
        &mut disk_probe,
    )
    .await
    .expect("initial warm entry");

    let error = pool
        .maintain_with_disk_probe_elapsed(
            key.clone(),
            &manifest,
            ResourceBudget::new(99, Size::gib(1), Size::gib(1)),
            &mut launcher,
            Duration::from_millis(1),
            &mut disk_probe,
        )
        .await
        .expect_err("oversized additional entry rejects");

    assert!(matches!(error, WarmPoolMaintenanceError::Capacity(_)));
    assert!(pool.contains(&key));
    assert_eq!(
        pool.capacity().warm_pool(),
        ResourceBudget::new(1, Size::gib(1), Size::gib(1))
    );
    let checked_out = pool
        .checkout_with_elapsed(&key, Duration::from_millis(1))
        .expect("checkout preserved entry")
        .expect("preserved entry exists");
    assert_eq!(checked_out.entry().snapshot_id(), "snapshot-1");
}

#[tokio::test]
async fn snapshot_warm_pool_refill_stops_before_active_restore_disk_floor() {
    let capacity = CapacityLedger::new(ResourceBudget::new(4, Size::gib(16), Size::gib(200)));
    let mut pool = RuntimeSnapshotWarmPool::new(capacity);
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let manifest = SnapshotArtifactManifest::base("snapshot-main", &snapshot_path);
    let budget = ResourceBudget::new(1, Size::gib(1), Size::gib(1));
    let mut launcher = AsyncRecordingSnapshotLauncher::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(15),
        probed_paths: Vec::new(),
    };

    let error = pool
        .maintain_with_disk_probe_elapsed(
            key,
            &manifest,
            budget,
            &mut launcher,
            Duration::from_millis(1),
            &mut disk_probe,
        )
        .await
        .expect_err("warm-pool refill should stop at soft disk pressure");

    assert!(matches!(
        error,
        WarmPoolMaintenanceError::Capacity(firkin_admission::CapacityError::Disk {
            requested,
            available,
        }) if requested == Size::gib(20) && available == Size::gib(15)
    ));
    assert_eq!(disk_probe.probed_paths, vec![temp.path().to_path_buf()]);
    assert!(launcher.restored_paths.is_empty());
    assert_eq!(
        pool.capacity().warm_pool(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[tokio::test]
async fn snapshot_warm_pool_replenishes_missing_targets_and_skips_warm_entries() {
    let capacity = CapacityLedger::new(ResourceBudget::new(4, Size::gib(16), Size::gib(200)));
    let mut pool = RuntimeSnapshotWarmPool::new(capacity);
    let warm = WarmPoolKey::new("repo-a", "base-template", "apple-vz-arm64");
    let missing = WarmPoolKey::new("repo-b", "base-template", "apple-vz-arm64");
    let temp = tempfile::tempdir().expect("tempdir");
    let warm_manifest = SnapshotArtifactManifest::base("snapshot-a", temp.path().join("a.vz"));
    let missing_manifest = SnapshotArtifactManifest::base("snapshot-b", temp.path().join("b.vz"));
    let budget = ResourceBudget::new(1, Size::gib(1), Size::gib(1));
    let mut launcher = AsyncRecordingSnapshotLauncher::default();
    let mut disk_probe = ample_disk_probe();

    pool.maintain_with_disk_probe_elapsed(
        warm.clone(),
        &warm_manifest,
        budget,
        &mut launcher,
        Duration::from_millis(1),
        &mut disk_probe,
    )
    .await
    .expect("initial warm entry");
    launcher.restored_paths.clear();

    let report = pool
        .replenish_with_disk_probe_elapsed(
            &[
                WarmPoolReplenishmentTarget::new(warm.clone(), warm_manifest, budget),
                WarmPoolReplenishmentTarget::new(missing.clone(), missing_manifest.clone(), budget),
            ],
            &mut launcher,
            Duration::from_millis(2),
            &mut disk_probe,
        )
        .await;

    assert_eq!(report.maintained(), std::slice::from_ref(&missing));
    assert_eq!(report.skipped().len(), 1);
    assert_eq!(report.skipped()[0].key(), &warm);
    assert_eq!(
        report.skipped()[0].reason(),
        WarmPoolReplenishmentSkipReason::AlreadyWarm
    );
    assert!(report.failed().is_empty());
    assert_eq!(launcher.restored_paths, vec![missing_manifest.path()]);
    assert!(pool.contains(&warm));
    assert!(pool.contains(&missing));
}

#[tokio::test]
async fn warm_pool_supervisor_runs_repeated_replenishment_cycles() {
    let capacity = CapacityLedger::new(ResourceBudget::new(4, Size::gib(16), Size::gib(200)));
    let mut pool = RuntimeSnapshotWarmPool::new(capacity);
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = SnapshotArtifactManifest::base("snapshot-main", temp.path().join("main.vz"));
    let budget = ResourceBudget::new(1, Size::gib(1), Size::gib(1));
    let mut launcher = AsyncRecordingSnapshotLauncher::default();
    let supervisor = RuntimeWarmPoolSupervisor::new(
        vec![WarmPoolReplenishmentTarget::new(
            key.clone(),
            manifest.clone(),
            budget,
        )],
        Duration::ZERO,
    );

    let mut disk_probe = ample_disk_probe();
    let reports = supervisor
        .run_cycles_with_disk_probe(&mut pool, &mut launcher, 2, &mut disk_probe)
        .await;

    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].maintained(), std::slice::from_ref(&key));
    assert!(reports[0].skipped().is_empty());
    assert!(reports[0].failed().is_empty());
    assert!(reports[1].maintained().is_empty());
    assert_eq!(reports[1].skipped().len(), 1);
    assert_eq!(
        reports[1].skipped()[0].reason(),
        WarmPoolReplenishmentSkipReason::AlreadyWarm
    );
    assert_eq!(launcher.restored_paths, vec![manifest.path()]);
    assert!(pool.contains(&key));
}

#[tokio::test]
async fn warm_pool_service_owns_pool_and_does_not_refill_over_active_capacity() {
    let capacity = CapacityLedger::new(ResourceBudget::new(1, Size::gib(1), Size::gib(10)));
    let pool = RuntimeSnapshotWarmPool::new(capacity);
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = SnapshotArtifactManifest::base("snapshot-main", temp.path().join("main.vz"));
    let budget = ResourceBudget::new(1, Size::gib(1), Size::gib(10));
    let supervisor = RuntimeWarmPoolSupervisor::new(
        vec![WarmPoolReplenishmentTarget::new(
            key.clone(),
            manifest.clone(),
            budget,
        )],
        Duration::ZERO,
    );
    let launcher = AsyncRecordingSnapshotLauncher::default();
    let mut service = RuntimeWarmPoolService::new(pool, supervisor, launcher);
    let mut disk_probe = ample_disk_probe();

    let first = service
        .tick_with_disk_probe(Duration::from_millis(1), &mut disk_probe)
        .await;
    assert_eq!(first.maintained(), std::slice::from_ref(&key));
    assert!(service.contains(&key));

    let checked_out = service
        .checkout_with_elapsed(&key, Duration::from_millis(2))
        .expect("checkout")
        .expect("warm entry exists");
    assert_eq!(checked_out.session(), "warm-live:snapshot-main");
    assert_eq!(service.capacity().active(), budget);

    let second = service
        .tick_with_disk_probe(Duration::from_millis(3), &mut disk_probe)
        .await;
    assert!(second.maintained().is_empty());
    assert_eq!(second.skipped().len(), 1);
    assert_eq!(
        second.skipped()[0].reason(),
        WarmPoolReplenishmentSkipReason::InsufficientCapacity {
            requested: budget,
            available: ResourceBudget::new(0, Size::bytes(0), Size::bytes(0)),
        }
    );
    assert_eq!(service.launcher().restored_paths, vec![manifest.path()]);
}

#[tokio::test]
async fn warm_pool_service_spawn_runs_background_refill_and_shuts_down_cleanly() {
    let capacity = CapacityLedger::new(ResourceBudget::new(2, Size::gib(2), Size::gib(20)));
    let pool = RuntimeSnapshotWarmPool::new(capacity);
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = SnapshotArtifactManifest::base("snapshot-main", temp.path().join("main.vz"));
    let budget = ResourceBudget::new(1, Size::gib(1), Size::gib(10));
    let supervisor = RuntimeWarmPoolSupervisor::new(
        vec![WarmPoolReplenishmentTarget::new(
            key.clone(),
            manifest.clone(),
            budget,
        )],
        Duration::from_millis(1),
    );
    let launcher = AsyncRecordingSnapshotLauncher::default();
    let service = RuntimeWarmPoolService::new(pool, supervisor, launcher);

    let handle = service.spawn_with_disk_probe(ample_disk_probe());
    for _ in 0..20 {
        if handle.contains(&key).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(handle.contains(&key).await);

    let service = handle
        .shutdown()
        .await
        .expect("warm-pool service task exits");
    assert!(service.contains(&key));
    assert_eq!(service.launcher().restored_paths, vec![manifest.path()]);
}
