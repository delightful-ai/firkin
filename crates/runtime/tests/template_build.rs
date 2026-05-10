//! Runtime template build snapshot orchestration tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use firkin_runtime::types::Size;
use firkin_runtime::{
    CoreTemplateCommandRunner, DiskPressureProbe, RuntimeTemplateBuildError,
    RuntimeTemplateBuildSnapshot, TemplateBuildRuntimeRequest, TemplateCommandRunner,
};
use firkin_template::{SnapshotSinkError, TemplateSnapshotSink};
use {
    firkin_artifacts::{SnapshotArtifactIntegrity, SnapshotArtifactKind, SnapshotArtifactManifest},
    firkin_template::TemplateBuildJob,
};

#[derive(Default)]
struct RecordingCommandRunner {
    repos: Vec<String>,
    setup_commands: Vec<String>,
    cache_warm_commands: Vec<String>,
    fail: bool,
}

#[async_trait]
impl TemplateCommandRunner for RecordingCommandRunner {
    type Error = &'static str;

    async fn run_template_commands(
        &mut self,
        request: &TemplateBuildRuntimeRequest<'_>,
    ) -> Result<(), Self::Error> {
        self.repos.push(request.job().repo().to_owned());
        self.setup_commands
            .extend(request.job().setup_commands().iter().cloned());
        self.cache_warm_commands
            .extend(request.job().cache_warm_commands().iter().cloned());
        if self.fail {
            Err("template commands failed")
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct RecordingSnapshotSink {
    saved_paths: std::sync::Mutex<Vec<PathBuf>>,
}

#[async_trait]
impl TemplateSnapshotSink for RecordingSnapshotSink {
    async fn save_snapshot(&self, path: &Path) -> Result<(), SnapshotSinkError> {
        self.saved_paths
            .lock()
            .expect("lock paths")
            .push(path.to_path_buf());
        std::fs::write(path, b"snapshot")
            .map_err(|source| Box::new(source) as SnapshotSinkError)?;
        Ok(())
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

#[tokio::test]
async fn runtime_template_build_runs_commands_then_saves_snapshot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let job = TemplateBuildJob::new("file:///repo", "main", &snapshot_path)
        .setup_command("npm install")
        .cache_warm_command("npm test -- --warm-cache");
    let mut runner = RecordingCommandRunner::default();
    let sink = RecordingSnapshotSink::default();

    let report = RuntimeTemplateBuildSnapshot::new(&job, "repo-main")
        .execute_with_elapsed(
            &mut runner,
            &sink,
            Duration::from_millis(120),
            Duration::from_millis(25),
        )
        .await
        .expect("template build succeeds");

    assert_eq!(runner.repos, vec!["file:///repo"]);
    assert_eq!(runner.setup_commands, vec!["npm install"]);
    assert_eq!(runner.cache_warm_commands, vec!["npm test -- --warm-cache"]);
    assert_eq!(
        *sink.saved_paths.lock().expect("lock paths"),
        vec![snapshot_path]
    );
    assert_eq!(report.manifest().kind(), SnapshotArtifactKind::BaseTemplate);
    assert_eq!(report.manifest().logical_id(), "repo-main");
    assert_eq!(report.setup_command_count(), 1);
    assert_eq!(report.cache_warm_command_count(), 1);
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
    assert_eq!(
        report.benchmark_samples()[0].metric(),
        "cold_template_build"
    );
    assert_eq!(report.benchmark_samples()[1].metric(), "snapshot_save");
}

#[tokio::test]
async fn runtime_template_build_checks_disk_before_commands_or_snapshot_save() {
    let job = TemplateBuildJob::new("file:///repo", "main", "/snapshots/repo-main.vz")
        .setup_command("npm install")
        .cache_warm_command("npm test -- --warm-cache");
    let mut runner = RecordingCommandRunner::default();
    let sink = RecordingSnapshotSink::default();
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(9),
        probed_paths: Vec::new(),
    };

    let error = RuntimeTemplateBuildSnapshot::new(&job, "repo-main")
        .execute_with_disk_probe_elapsed(
            &mut runner,
            &sink,
            Duration::from_millis(120),
            Duration::from_millis(25),
            &mut disk_probe,
        )
        .await
        .expect_err("disk floor blocks template snapshot");

    assert!(matches!(
        error,
        RuntimeTemplateBuildError::Capacity(firkin_admission::CapacityError::Disk {
            requested,
            available,
        }) if requested == Size::gib(10) && available == Size::gib(9)
    ));
    assert_eq!(disk_probe.probed_paths, vec![PathBuf::from("/snapshots")]);
    assert!(runner.repos.is_empty());
    assert!(sink.saved_paths.lock().expect("lock paths").is_empty());
}

#[test]
fn core_template_command_runner_places_checkouts_under_guest_root() {
    let checkout_dir =
        CoreTemplateCommandRunner::checkout_dir_for_root("/runtime/templates", "repo-main");

    assert_eq!(checkout_dir, PathBuf::from("/runtime/templates/repo-main"));
}
