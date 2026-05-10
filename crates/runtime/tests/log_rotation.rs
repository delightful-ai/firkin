//! Runtime log rotation orchestration tests.

use std::fs;

use firkin_runtime::RuntimeLogRotation;

#[test]
fn runtime_log_rotation_executes_substrate_plan() {
    let root = tempfile::tempdir().expect("tempdir");
    let large = root.path().join("large.log");
    let small = root.path().join("small.log");
    fs::write(&large, b"0123456789").expect("large");
    fs::write(&small, b"ok").expect("small");

    let report = RuntimeLogRotation::new(root.path(), 4)
        .execute()
        .expect("rotation executes");

    assert_eq!(report.rotated_count(), 1);
    assert!(root.path().join("large.log.1").exists());
    assert!(!large.exists());
    assert!(small.exists());
}

#[test]
fn runtime_log_rotation_can_retain_bounded_generations() {
    let root = tempfile::tempdir().expect("tempdir");
    let active = root.path().join("app.log");
    let first = root.path().join("app.log.1");
    let second = root.path().join("app.log.2");
    fs::write(&active, b"0123456789").expect("active");
    fs::write(&first, b"previous").expect("first");
    fs::write(&second, b"stale").expect("second");

    let report = RuntimeLogRotation::new(root.path(), 4)
        .with_max_rotated_files(2)
        .execute()
        .expect("rotation executes");

    assert_eq!(report.rotated_count(), 1);
    assert!(!active.exists());
    assert_eq!(fs::read(&first).expect("read first"), b"0123456789");
    assert_eq!(fs::read(&second).expect("read second"), b"previous");
}

#[test]
fn runtime_log_rotation_can_gzip_rotated_logs() {
    let root = tempfile::tempdir().expect("tempdir");
    let active = root.path().join("app.log");
    fs::write(&active, b"0123456789").expect("active");

    let report = RuntimeLogRotation::new(root.path(), 4)
        .with_max_rotated_files(2)
        .with_gzip_compression()
        .execute()
        .expect("rotation executes");
    let rotated = root.path().join("app.log.1.gz");

    assert_eq!(report.rotated(), std::slice::from_ref(&rotated));
    assert!(!active.exists());
    assert!(
        fs::read(rotated)
            .expect("read gzip")
            .starts_with(&[0x1f, 0x8b])
    );
}
