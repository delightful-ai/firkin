//! Runtime snapshot artifact GC orchestration tests.

use std::fs;
use std::time::Duration;

use firkin_artifacts::SnapshotArtifactManifest;
use firkin_runtime::RuntimeSnapshotArtifactGc;

#[test]
fn runtime_snapshot_artifact_gc_executes_substrate_plan() {
    let root = tempfile::tempdir().expect("tempdir");
    let keep = root.path().join("keep.vzstate");
    let delete = root.path().join("delete.vzstate");
    let delete_dir = root.path().join("delete-dir.vz");
    fs::write(&keep, b"keep").expect("keep");
    fs::write(&delete, b"delete").expect("delete");
    fs::create_dir(&delete_dir).expect("delete dir");
    fs::write(delete_dir.join("state"), b"delete").expect("delete dir child");
    let manifest = SnapshotArtifactManifest::base("repo-main", &keep);

    let report = RuntimeSnapshotArtifactGc::new(root.path(), [manifest])
        .execute()
        .expect("gc executes");

    assert_eq!(report.deleted_count(), 2);
    assert!(keep.exists());
    assert!(!delete.exists());
    assert!(!delete_dir.exists());
}

#[test]
fn runtime_snapshot_artifact_gc_can_retain_recent_unreferenced_artifacts() {
    let root = tempfile::tempdir().expect("tempdir");
    let recent = root.path().join("recent.vzstate");
    let recent_dir = root.path().join("recent.vz");
    fs::write(&recent, b"recent").expect("recent");
    fs::create_dir(&recent_dir).expect("recent dir");

    let report = RuntimeSnapshotArtifactGc::new(root.path(), [])
        .with_min_unreferenced_age(Duration::from_hours(1))
        .execute()
        .expect("gc executes");

    assert_eq!(report.deleted_count(), 0);
    assert!(recent.exists());
    assert!(recent_dir.exists());
}

#[test]
fn runtime_snapshot_artifact_gc_can_read_manifest_sidecars() {
    let root = tempfile::tempdir().expect("tempdir");
    let keep = root.path().join("keep.vzstate");
    let delete = root.path().join("delete.vzstate");
    fs::write(&keep, b"keep").expect("keep");
    fs::write(&delete, b"delete").expect("delete");
    let manifest = SnapshotArtifactManifest::base("repo-main", &keep);
    manifest
        .write_json(root.path().join("keep.manifest.json"))
        .expect("write manifest");

    let report = RuntimeSnapshotArtifactGc::from_manifest_dir(root.path(), root.path())
        .expect("gc from sidecars")
        .execute()
        .expect("gc executes");

    assert_eq!(report.deleted_count(), 1);
    assert!(keep.exists());
    assert!(!delete.exists());
}
