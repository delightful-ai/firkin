//! Runtime hygiene maintenance scheduling tests.

use std::fs;
use std::time::Duration;

use firkin_artifacts::SnapshotArtifactManifest;
use firkin_runtime::RuntimeHygieneMaintenance;

#[test]
fn runtime_hygiene_maintenance_tick_runs_snapshot_gc_and_log_rotation() {
    let root = tempfile::tempdir().expect("tempdir");
    let snapshot_root = root.path().join("snapshots");
    let log_root = root.path().join("logs");
    fs::create_dir_all(&snapshot_root).expect("snapshots");
    fs::create_dir_all(&log_root).expect("logs");
    let keep = snapshot_root.join("keep.vzstate");
    let delete = snapshot_root.join("delete.vzstate");
    let large_log = log_root.join("runtime.log");
    fs::write(&keep, b"keep").expect("keep");
    fs::write(&delete, b"delete").expect("delete");
    fs::write(&large_log, b"0123456789").expect("large log");
    let manifest = SnapshotArtifactManifest::base("repo-main", &keep);

    let report = RuntimeHygieneMaintenance::new(
        &snapshot_root,
        [manifest],
        &log_root,
        4,
        Duration::from_mins(1),
    )
    .tick()
    .expect("maintenance tick");

    assert_eq!(report.artifact_gc().deleted_count(), 1);
    assert_eq!(report.log_rotation().rotated_count(), 1);
    assert!(keep.exists());
    assert!(!delete.exists());
    assert!(log_root.join("runtime.log.1").exists());
}

#[test]
fn runtime_hygiene_maintenance_can_gzip_rotated_logs() {
    let root = tempfile::tempdir().expect("tempdir");
    let snapshot_root = root.path().join("snapshots");
    let log_root = root.path().join("logs");
    fs::create_dir_all(&snapshot_root).expect("snapshots");
    fs::create_dir_all(&log_root).expect("logs");
    let large_log = log_root.join("runtime.log");
    fs::write(&large_log, b"0123456789").expect("large log");

    let report =
        RuntimeHygieneMaintenance::new(&snapshot_root, [], &log_root, 4, Duration::from_mins(1))
            .with_gzip_log_compression()
            .tick()
            .expect("maintenance tick");
    let rotated = log_root.join("runtime.log.1.gz");

    assert_eq!(
        report.log_rotation().rotated(),
        std::slice::from_ref(&rotated)
    );
    assert!(!large_log.exists());
    assert!(
        fs::read(rotated)
            .expect("read gzip")
            .starts_with(&[0x1f, 0x8b])
    );
}

#[test]
fn runtime_hygiene_maintenance_can_read_manifest_sidecars_each_tick() {
    let root = tempfile::tempdir().expect("tempdir");
    let snapshot_root = root.path().join("snapshots");
    let log_root = root.path().join("logs");
    fs::create_dir_all(&snapshot_root).expect("snapshots");
    fs::create_dir_all(&log_root).expect("logs");
    let keep = snapshot_root.join("keep.vzstate");
    let delete = snapshot_root.join("delete.vzstate");
    fs::write(&keep, b"keep").expect("keep");
    fs::write(&delete, b"delete").expect("delete");
    SnapshotArtifactManifest::base("repo-main", &keep)
        .write_json(snapshot_root.join("keep.manifest.json"))
        .expect("write manifest");

    let report =
        RuntimeHygieneMaintenance::new(&snapshot_root, [], &log_root, 4, Duration::from_mins(1))
            .with_manifest_dir(&snapshot_root)
            .tick()
            .expect("maintenance tick");

    assert_eq!(report.artifact_gc().deleted_count(), 1);
    assert!(keep.exists());
    assert!(!delete.exists());
    assert!(snapshot_root.join("keep.manifest.json").exists());
}

#[tokio::test]
async fn runtime_hygiene_maintenance_spawn_runs_periodic_ticks_and_shuts_down() {
    let root = tempfile::tempdir().expect("tempdir");
    let snapshot_root = root.path().join("snapshots");
    let log_root = root.path().join("logs");
    fs::create_dir_all(&snapshot_root).expect("snapshots");
    fs::create_dir_all(&log_root).expect("logs");
    let large_log = log_root.join("runtime.log");
    fs::write(&large_log, b"0123456789").expect("large log");

    let handle =
        RuntimeHygieneMaintenance::new(&snapshot_root, [], &log_root, 4, Duration::from_millis(1))
            .spawn();

    for _ in 0..20 {
        if log_root.join("runtime.log.1").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let maintenance = handle.shutdown().await.expect("maintenance shuts down");
    assert!(log_root.join("runtime.log.1").exists());
    assert_eq!(maintenance.interval(), Duration::from_millis(1));
}
