//! Template build executor tests.

use firkin_template::{SnapshotSinkError, TemplateBuildExecutor, TemplateSnapshotSink};
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
use std::fs;
use std::path::Path;
use std::process::Command;
use {firkin_artifacts::SnapshotArtifactKind, firkin_template::TemplateBuildJob};

use async_trait::async_trait;

#[test]
fn template_build_executor_clones_checkout_and_runs_setup_and_cache_warm_commands() {
    let source = committed_repo();
    let root = tempfile::tempdir().expect("root");
    let job = TemplateBuildJob::new(
        source.path().display().to_string(),
        "HEAD",
        root.path().join("snapshots/repo-main.vzstate"),
    )
    .setup_command("printf setup > setup.txt")
    .cache_warm_command("printf warm > warm.txt");

    let report = TemplateBuildExecutor::new(root.path())
        .execute(&job, "repo-main")
        .expect("execute build");

    assert_eq!(report.manifest().kind(), SnapshotArtifactKind::BaseTemplate);
    assert_eq!(report.setup_command_count(), 1);
    assert_eq!(report.cache_warm_command_count(), 1);
    assert!(report.checkout_dir().join("README.md").exists());
    assert_eq!(
        fs::read_to_string(report.checkout_dir().join("setup.txt")).expect("setup"),
        "setup"
    );
    assert_eq!(
        fs::read_to_string(report.checkout_dir().join("warm.txt")).expect("warm"),
        "warm"
    );
    let sample = report
        .benchmark_samples()
        .iter()
        .find(|sample| sample.metric() == "cold_template_build")
        .expect("cold template build sample");
    assert_eq!(sample.kind(), BenchmarkMetricKind::LifecycleLatency);
    assert_eq!(sample.unit(), BenchmarkUnit::Milliseconds);
    assert!(sample.value() >= 0.0);
}

#[tokio::test]
async fn template_build_executor_saves_snapshot_through_sink() {
    let source = committed_repo();
    let root = tempfile::tempdir().expect("root");
    let snapshot_path = root.path().join("snapshots/repo-main.vzstate");
    let job = TemplateBuildJob::new(source.path().display().to_string(), "HEAD", &snapshot_path);

    let report = TemplateBuildExecutor::new(root.path())
        .execute_with_snapshot_sink(&job, "repo-main", &FileSnapshotSink)
        .await
        .expect("execute build");

    assert_eq!(report.manifest().path(), snapshot_path);
    assert_eq!(
        fs::read_to_string(snapshot_path).expect("snapshot"),
        "snapshot"
    );
    let sample = report
        .benchmark_samples()
        .iter()
        .find(|sample| sample.metric() == "snapshot_save")
        .expect("snapshot save sample");
    assert_eq!(sample.kind(), BenchmarkMetricKind::LifecycleLatency);
    assert_eq!(sample.unit(), BenchmarkUnit::Milliseconds);
}

struct FileSnapshotSink;

#[async_trait]
impl TemplateSnapshotSink for FileSnapshotSink {
    async fn save_snapshot(&self, path: &Path) -> Result<(), SnapshotSinkError> {
        fs::write(path, "snapshot")?;
        Ok(())
    }
}

fn committed_repo() -> tempfile::TempDir {
    let source = tempfile::tempdir().expect("source");
    fs::write(source.path().join("README.md"), "template").expect("readme");
    Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(source.path())
        .status()
        .expect("git init");
    Command::new("git")
        .args(["add", "."])
        .current_dir(source.path())
        .status()
        .expect("git add");
    Command::new("git")
        .args(["commit", "--quiet", "-m", "init"])
        .current_dir(source.path())
        .env("GIT_AUTHOR_NAME", "Firkin")
        .env("GIT_AUTHOR_EMAIL", "firkin@example.invalid")
        .env("GIT_COMMITTER_NAME", "Firkin")
        .env("GIT_COMMITTER_EMAIL", "firkin@example.invalid")
        .status()
        .expect("git commit");
    source
}
