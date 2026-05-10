//! Compile-time API-shape tests for firkin-core.

#[test]
fn pty_exec_builder_hides_stdout_and_stderr() {
    clear_trybuild_scratch();
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/pty_container_accessor.rs");
    tests.pass("tests/ui/pty_process_accessor.rs");
    tests.compile_fail("tests/ui/pty_exec_stdout.rs");
    tests.compile_fail("tests/ui/pty_exec_stderr.rs");
    tests.compile_fail("tests/ui/pty_container_stdout.rs");
    tests.compile_fail("tests/ui/pty_container_stderr.rs");
    tests.compile_fail("tests/ui/stream_container_pty.rs");
    tests.compile_fail("tests/ui/stream_process_pty.rs");
}

fn clear_trybuild_scratch() {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
        },
        std::path::PathBuf::from,
    );

    let _ = std::fs::remove_dir_all(target_dir.join("tests/trybuild/firkin-core"));
    if let Ok(canonical_target_dir) = target_dir.canonicalize() {
        let _ = std::fs::remove_dir_all(canonical_target_dir.join("tests/trybuild/firkin-core"));
    }
}
