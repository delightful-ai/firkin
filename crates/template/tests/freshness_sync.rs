//! Freshness sync executor tests.

use firkin_template::FreshnessSyncExecutor;
use firkin_template::FreshnessSyncPhase;
use std::fs;
use std::process::Command;

#[test]
fn freshness_sync_executor_fast_forwards_checkout_and_unlocks_writes() {
    let source = committed_repo();
    let checkout = tempfile::tempdir().expect("checkout");
    run_git(
        ["clone", "--quiet", source.path().to_str().unwrap(), "."],
        checkout.path(),
    );

    fs::write(source.path().join("README.md"), "updated").expect("update");
    run_git(["add", "."], source.path());
    run_git_commit("update", source.path());
    let target = git_rev_parse("HEAD", source.path());

    let report = FreshnessSyncExecutor::new("refs/heads/main")
        .sync_checkout(checkout.path(), &target)
        .expect("sync");

    assert_eq!(report.gate().phase(), FreshnessSyncPhase::Ready);
    assert!(report.gate().reads_allowed());
    assert!(report.gate().writes_allowed());
    assert_eq!(report.gate().synced_commit(), Some(target.as_str()));
    assert_eq!(
        fs::read_to_string(checkout.path().join("README.md")).expect("readme"),
        "updated"
    );
    assert!(
        report
            .benchmark_samples()
            .iter()
            .any(|sample| sample.metric() == "freshness_sync")
    );
}

fn committed_repo() -> tempfile::TempDir {
    let source = tempfile::tempdir().expect("source");
    fs::write(source.path().join("README.md"), "template").expect("readme");
    run_git(["init", "--quiet"], source.path());
    run_git(["add", "."], source.path());
    run_git_commit("init", source.path());
    source
}

fn run_git<const N: usize>(args: [&str; N], dir: &std::path::Path) {
    assert!(
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git")
            .success()
    );
}

fn run_git_commit(message: &str, dir: &std::path::Path) {
    assert!(
        Command::new("git")
            .args(["commit", "--quiet", "-m", message])
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Firkin")
            .env("GIT_AUTHOR_EMAIL", "firkin@example.invalid")
            .env("GIT_COMMITTER_NAME", "Firkin")
            .env("GIT_COMMITTER_EMAIL", "firkin@example.invalid")
            .status()
            .expect("git commit")
            .success()
    );
}

fn git_rev_parse(rev: &str, dir: &std::path::Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(dir)
        .output()
        .expect("git rev-parse");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_owned()
}
