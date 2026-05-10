//! State-store tests for the single-node runtime facade.

#![cfg(any())]
// Scaffolding: this runtime integration test depends on firkin-single-node.
// It is disabled during the crates.io bootstrap to keep the publish graph acyclic.

use std::fs;

use firkin_single_node::{
    ActiveSessionRecord, FileStateStore, LogStore, SandboxResources, SnapshotRecord, StateStore,
};
use firkin_types::Size;

#[test]
fn state_store_reconciles_active_records_in_memory() {
    let store = StateStore::new();
    let active = vec![active_record("expired", 100), active_record("live", 300)];
    store.save_active(active).unwrap();
    let reconciled = store.reconcile_active_entries(200).unwrap();
    assert_eq!(
        reconciled
            .iter()
            .map(|record| record.sandbox_id.as_str())
            .collect::<Vec<_>>(),
        vec!["live"]
    );
    assert_eq!(store.load_active().unwrap(), reconciled);
}

#[test]
fn file_state_store_persists_and_reconciles_recovery_records() {
    let tempdir = tempfile::tempdir().unwrap();
    let files = FileStateStore::new(tempdir.path()).unwrap();
    assert_eq!(files.active_path(), tempdir.path().join("active.json"));
    assert_eq!(files.logs_path(), tempdir.path().join("logs.json"));
    assert_eq!(
        files.snapshots_path(),
        tempdir.path().join("snapshots.json")
    );

    let state = StateStore::new();
    state
        .save_active(vec![active_record("expired", 100)])
        .unwrap();
    files.save_state(&state).unwrap();
    assert_eq!(files.load_state().unwrap().load_active().unwrap().len(), 1);
    assert!(files.reconcile_active_entries(200).unwrap().is_empty());
    files.save_active(&[active_record("live", 300)]).unwrap();

    let artifact_dir = files.snapshot_artifacts_dir();
    fs::create_dir_all(&artifact_dir).unwrap();
    let live_artifact = artifact_dir.join("live.vzstate");
    let missing_artifact = artifact_dir.join("missing.vzstate");
    let orphan_artifact = artifact_dir.join("orphan.vzstate");
    fs::write(&live_artifact, b"live").unwrap();
    fs::write(&orphan_artifact, b"orphan").unwrap();

    files
        .save_snapshots(&[
            snapshot_record("live-snapshot", &live_artifact),
            snapshot_record("missing-snapshot", &missing_artifact),
        ])
        .unwrap();
    let snapshots = files
        .reconcile_snapshot_artifacts(files.load_snapshots().unwrap())
        .unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].snapshot_id, "live-snapshot");
    assert!(live_artifact.exists());
    assert!(!orphan_artifact.exists());
    assert_eq!(files.load_snapshots().unwrap(), snapshots);
}

#[test]
fn file_backed_log_store_persists_entries_and_removals() {
    let dir = tempfile::tempdir().unwrap();
    let files = FileStateStore::new(dir.path()).unwrap();
    let logs = LogStore::with_file_state_store(files.clone()).unwrap();

    logs.record("sandbox-1", "stdout: persisted".to_string())
        .unwrap();
    let reloaded = files.load_state().unwrap();
    assert_eq!(
        reloaded.load_logs().unwrap()["sandbox-1"][0].message(),
        "stdout: persisted"
    );

    logs.remove_sandbox("sandbox-1").unwrap();
    let reloaded = files.load_state().unwrap();
    assert!(!reloaded.load_logs().unwrap().contains_key("sandbox-1"));
}

#[test]
fn log_store_bounds_and_pages_in_memory_events() {
    let store = StateStore::new();
    let logs = LogStore::new(store.clone());

    logs.record("sbx", "first".to_string()).unwrap();
    logs.record("sbx", "second".to_string()).unwrap();

    let page = logs.entries("sbx", Some(1), 10).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].message(), "second");

    let persisted = store.load_logs().unwrap();
    assert_eq!(persisted["sbx"].len(), 2);

    logs.remove_sandbox("sbx").unwrap();
    assert!(logs.entries("sbx", None, 10).unwrap().is_empty());
    assert!(!store.load_logs().unwrap().contains_key("sbx"));
}

fn active_record(sandbox_id: &str, end_at_unix_seconds: i64) -> ActiveSessionRecord {
    ActiveSessionRecord {
        sandbox_id: sandbox_id.to_string(),
        template_id: "base".to_string(),
        client_id: "client".to_string(),
        envd_access_token: Some("token".to_string()),
        started_at_unix_seconds: 1,
        end_at_unix_seconds,
        resources: SandboxResources::new(2, Size::gib(1)),
        runtime_attached: true,
    }
}

fn snapshot_record(snapshot_id: &str, path: &std::path::Path) -> SnapshotRecord {
    let mut record = SnapshotRecord::new(snapshot_id, "sbx");
    record.location = Some(path.display().to_string());
    record
}
