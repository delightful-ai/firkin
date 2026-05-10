//! template build — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::DEFAULT_RUNTIME_MINIMUM_FREE_DISK;
#[allow(unused_imports)]
use crate::FRESHNESS_SYNC_CHECKOUT_METADATA;
#[allow(unused_imports)]
use crate::adapter::FirkinRuntimeAdapter;
#[allow(unused_imports)]
use crate::continuation::RuntimeContinuationSnapshotSource;
#[allow(unused_imports)]
use crate::disk::{DiskPressureProbe, HostDiskPressureProbe, RuntimeDiskPressureGuard};
#[allow(unused_imports)]
use crate::hygiene::snapshot_artifact_integrity;
#[allow(unused_imports)]
use crate::interactive::RuntimeInteractiveProcessRunner;
#[allow(unused_imports)]
use crate::restore::{
    RuntimeCubeSandboxCreateError, SnapshotRestoreError, SnapshotSessionLauncher,
    disk_pressure_to_capacity_error, snapshot_output_disk_root, write_snapshot_artifact_sidecars,
};
#[allow(unused_imports)]
use crate::session::{
    RuntimeCommandRunner, RuntimeCommandStreamRunner, RuntimePortRouter, RuntimeReadinessProbe,
    RuntimeSessionStop,
};
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use firkin_admission::CapacityError;
#[allow(unused_imports)]
use firkin_artifacts::{SnapshotArtifactIntegrity, SnapshotArtifactManifest};
#[allow(unused_imports)]
use firkin_core::ExitStatus;
#[allow(unused_imports)]
use firkin_core::{ExecConfig, Output, Stdio};
#[allow(unused_imports)]
use firkin_e2b_contract::PreparedTemplateArtifactIntegrity;
#[allow(unused_imports)]
use firkin_e2b_contract::StartSandboxRequest;
#[allow(unused_imports)]
use firkin_template::FreshnessSyncGate;
#[allow(unused_imports)]
use firkin_template::SnapshotSinkError;
#[allow(unused_imports)]
use firkin_template::TemplateBuildJob;
#[allow(unused_imports)]
use firkin_template::TemplateSnapshotSink;
#[allow(unused_imports)]
use firkin_trace::BenchmarkSample;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use std::fmt::Display;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
#[allow(unused_imports)]
use tokio::sync::oneshot;
#[allow(unused_imports)]
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use {
    firkin_e2b_contract::{BackendError, PreparedTemplate},
    firkin_e2b_server::LocalRuntimeBackend,
};
const FRESHNESS_SYNC_BRANCH_METADATA: &str = "firkin.sync.branch";
const FRESHNESS_SYNC_TARGET_METADATA: &str = "firkin.sync.target";
/// Report from maintaining Firkin adapter warm template targets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FirkinWarmTemplateMaintainReport {
    pub(crate) maintained: Vec<String>,
    pub(crate) skipped_already_warm: Vec<String>,
}
impl FirkinWarmTemplateMaintainReport {
    /// Return template ids restored into the warm pool.
    #[must_use]
    pub fn maintained(&self) -> &[String] {
        &self.maintained
    }
    /// Return template ids skipped because a matching warm entry already exists.
    #[must_use]
    pub fn skipped_already_warm(&self) -> &[String] {
        &self.skipped_already_warm
    }
}
/// Long-lived owner for adapter warm-template targets.
///
/// The maintainer keeps CubeAPI/product policy out of the hot create path: a
/// scheduler or background task can call [`Self::run_cycle`] periodically and
/// refill consumed warm templates through the same adapter-owned warm pool used
/// by `start()`.
#[derive(Clone)]
pub struct FirkinWarmTemplateMaintainer<L>
where
    L: SnapshotSessionLauncher,
{
    pub(crate) adapter: FirkinRuntimeAdapter<L>,
    pub(crate) targets: Vec<PreparedTemplate>,
    pub(crate) interval: Duration,
}
impl<L> FirkinWarmTemplateMaintainer<L>
where
    L: SnapshotSessionLauncher,
{
    /// Construct a warm-template maintainer for a fixed target set.
    #[must_use]
    pub fn new(
        adapter: FirkinRuntimeAdapter<L>,
        targets: Vec<PreparedTemplate>,
        interval: Duration,
    ) -> Self {
        Self {
            adapter,
            targets,
            interval,
        }
    }
    /// Construct a maintainer from the backend's latest ready prepared templates.
    #[must_use]
    pub fn from_backend(
        backend: &LocalRuntimeBackend<FirkinRuntimeAdapter<L>>,
        interval: Duration,
    ) -> Self
    where
        L: Clone + Send + 'static,
        L::Error: Display + Send,
        L::Session: RuntimeCommandRunner
            + RuntimeCommandStreamRunner
            + RuntimeInteractiveProcessRunner
            + RuntimePortRouter
            + RuntimeReadinessProbe
            + RuntimeSessionStop
            + RuntimeContinuationSnapshotSource
            + Send
            + Sync
            + 'static,
        <L::Session as RuntimeCommandRunner>::Error: Display + Send,
        <L::Session as RuntimeCommandStreamRunner>::Error: Display + Send,
        <L::Session as RuntimeInteractiveProcessRunner>::Error: Display + Send,
        <L::Session as RuntimePortRouter>::Error: Display,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        <L::Session as RuntimeSessionStop>::Error: Display,
    {
        Self::new(
            backend.adapter().clone(),
            backend.templates().latest_prepared_templates(),
            interval,
        )
    }
    /// Return configured warm-template targets.
    #[must_use]
    pub fn targets(&self) -> &[PreparedTemplate] {
        &self.targets
    }
    /// Return the intended background maintenance cadence.
    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }
    /// Run one maintenance pass over the configured targets.
    ///
    /// # Errors
    ///
    /// Returns runtime restore or capacity errors when any missing target cannot
    /// be retained in the adapter warm pool.
    pub async fn run_cycle(&self) -> Result<FirkinWarmTemplateMaintainReport, BackendError>
    where
        L: Clone + Send,
        L::Error: Display + Send,
        L::Session: RuntimeReadinessProbe,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
    {
        let mut disk_probe = HostDiskPressureProbe::new();
        self.run_cycle_with_disk_probe(&mut disk_probe).await
    }
    /// Run one maintenance pass using an injected disk-pressure probe.
    ///
    /// This is the deterministic/testing counterpart to [`Self::run_cycle`].
    ///
    /// # Errors
    ///
    /// Returns runtime restore or capacity errors when any missing target cannot
    /// be retained in the adapter warm pool.
    pub async fn run_cycle_with_disk_probe<P>(
        &self,
        disk_probe: &mut P,
    ) -> Result<FirkinWarmTemplateMaintainReport, BackendError>
    where
        L: Clone + Send,
        L::Error: Display + Send,
        L::Session: RuntimeReadinessProbe,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
        P: DiskPressureProbe,
    {
        self.adapter
            .maintain_warm_templates_with_disk_probe(self.targets.clone(), disk_probe)
            .await
    }
    /// Spawn periodic warm-template maintenance until the handle shuts it down.
    #[must_use]
    pub fn spawn(self) -> FirkinWarmTemplateMaintainerHandle
    where
        L: Clone + Send + 'static,
        L::Error: Display + Send,
        L::Session: RuntimeReadinessProbe,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
    {
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut disk_probe = HostDiskPressureProbe::new();
            loop {
                tokio::select! {
                    _ = & mut shutdown_rx => return Ok(()), result = self
                    .run_cycle_with_disk_probe(& mut disk_probe) =>
                    { result ?; }
                }
                tokio::select! {
                    _ = & mut shutdown_rx => return Ok(()), () = tokio::time::sleep(self
                    .interval) => {}
                }
            }
        });
        FirkinWarmTemplateMaintainerHandle {
            shutdown_tx: Some(shutdown_tx),
            task,
        }
    }
    /// Spawn periodic warm-template maintenance with an injected disk-pressure
    /// probe.
    ///
    /// This is intended for deterministic tests and custom orchestrators that
    /// have already sampled disk pressure. Production callers should use
    /// [`Self::spawn`].
    #[must_use]
    pub fn spawn_with_disk_probe<P>(self, mut disk_probe: P) -> FirkinWarmTemplateMaintainerHandle
    where
        L: Clone + Send + 'static,
        L::Error: Display + Send,
        L::Session: RuntimeReadinessProbe,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
        P: DiskPressureProbe + Send + 'static,
    {
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = & mut shutdown_rx => return Ok(()), result = self
                    .run_cycle_with_disk_probe(& mut disk_probe) =>
                    { result ?; }
                }
                tokio::select! {
                    _ = & mut shutdown_rx => return Ok(()), () = tokio::time::sleep(self
                    .interval) => {}
                }
            }
        });
        FirkinWarmTemplateMaintainerHandle {
            shutdown_tx: Some(shutdown_tx),
            task,
        }
    }
}
/// Handle for a spawned Firkin warm-template maintainer.
pub struct FirkinWarmTemplateMaintainerHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    pub(crate) task: JoinHandle<Result<(), BackendError>>,
}
impl FirkinWarmTemplateMaintainerHandle {
    /// Stop the background maintainer and wait for its task to finish.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the task panics or when the maintainer
    /// stopped because a maintenance cycle failed.
    pub async fn shutdown(mut self) -> Result<(), BackendError> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        match self.task.await {
            Ok(result) => result,
            Err(error) => Err(BackendError::Runtime(format!(
                "Firkin warm-template maintainer task failed: {error}"
            ))),
        }
    }
}
pub(crate) fn prepared_template_integrity<E>(
    template: &PreparedTemplate,
    manifest: &SnapshotArtifactManifest,
) -> Result<SnapshotArtifactIntegrity, RuntimeCubeSandboxCreateError<E>> {
    snapshot_artifact_integrity(template.artifact_integrity.as_ref(), manifest)
}
pub(crate) fn prepared_template_artifact_integrity(
    integrity: &PreparedTemplateArtifactIntegrity,
) -> SnapshotArtifactIntegrity {
    SnapshotArtifactIntegrity::new(integrity.size_bytes, integrity.sha256_hex.clone())
}
pub(crate) fn verify_prepared_template_integrity<E>(
    template: &PreparedTemplate,
    manifest: &SnapshotArtifactManifest,
) -> Result<(), RuntimeCubeSandboxCreateError<E>> {
    let integrity = prepared_template_integrity(template, manifest)?;
    integrity.verify(manifest).map_err(|error| {
        RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::Integrity {
            reason: error.to_string(),
        })
    })
}
pub(crate) fn freshness_sync_gate_for_request(
    request: &StartSandboxRequest,
) -> Option<FreshnessSyncGate> {
    let branch = request
        .create_request
        .metadata
        .get(FRESHNESS_SYNC_BRANCH_METADATA)?;
    let target = request
        .create_request
        .metadata
        .get(FRESHNESS_SYNC_TARGET_METADATA)?;
    Some(FreshnessSyncGate::restored(branch).begin_sync(target))
}
pub(crate) fn freshness_sync_checkout_for_request(request: &StartSandboxRequest) -> Option<String> {
    request
        .create_request
        .metadata
        .get(FRESHNESS_SYNC_CHECKOUT_METADATA)
        .cloned()
}
/// Request passed to the live template command runner.
#[derive(Clone, Copy, Debug)]
pub struct TemplateBuildRuntimeRequest<'a> {
    job: &'a TemplateBuildJob,
    logical_id: &'a str,
}
impl<'a> TemplateBuildRuntimeRequest<'a> {
    /// Construct a runtime template build request.
    #[must_use]
    pub const fn new(job: &'a TemplateBuildJob, logical_id: &'a str) -> Self {
        Self { job, logical_id }
    }
    /// Return the template build job.
    #[must_use]
    pub const fn job(self) -> &'a TemplateBuildJob {
        self.job
    }
    /// Return the logical id for the produced base snapshot.
    #[must_use]
    pub const fn logical_id(self) -> &'a str {
        self.logical_id
    }
}
/// Runtime-owned adapter for clone/setup/cache-warm commands inside a live template session.
#[async_trait]
pub trait TemplateCommandRunner {
    /// Error returned by the command runner implementation.
    type Error;
    /// Run the template build command path described by `request`.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific command execution errors.
    async fn run_template_commands(
        &mut self,
        request: &TemplateBuildRuntimeRequest<'_>,
    ) -> Result<(), Self::Error>;
}
/// Runtime template build snapshot error.
#[derive(Debug, ThisError)]
pub enum RuntimeTemplateBuildError<E> {
    /// Disk/capacity admission failed before template snapshot work started.
    #[error("runtime template build capacity admission failed: {0}")]
    Capacity(#[from] CapacityError),
    /// Template command path failed before snapshot save.
    #[error("runtime template build commands failed: {source}")]
    Commands {
        /// Source command runner error.
        source: E,
    },
    /// Snapshot sink failed after commands completed.
    #[error("runtime template build snapshot sink failed: {source}")]
    Snapshot {
        /// Source snapshot sink error.
        #[source]
        source: SnapshotSinkError,
    },
}
/// Error returned by the live core template command runner.
#[derive(Debug, ThisError)]
pub enum CoreTemplateCommandError {
    /// Core exec failed.
    #[error("core template command exec failed while {operation}: {source}")]
    Exec {
        /// Operation being attempted.
        operation: &'static str,
        /// Source core error.
        source: firkin_core::Error,
    },
    /// A guest command exited unsuccessfully.
    #[error("core template command failed while {operation}: {status:?}")]
    Status {
        /// Operation being attempted.
        operation: &'static str,
        /// Command exit status.
        status: ExitStatus,
        /// Captured stdout.
        stdout: Vec<u8>,
        /// Captured stderr.
        stderr: Vec<u8>,
    },
}
/// Live `firkin-core` command runner for template build jobs.
#[derive(Debug)]
pub struct CoreTemplateCommandRunner<'a> {
    pub(crate) container: &'a mut firkin_core::Container<firkin_core::Streams>,
    checkout_root: PathBuf,
    next_process_index: u64,
}
impl<'a> CoreTemplateCommandRunner<'a> {
    /// Construct a runner that executes template build commands inside `container`.
    #[must_use]
    pub fn new(container: &'a mut firkin_core::Container<firkin_core::Streams>) -> Self {
        Self {
            container,
            checkout_root: PathBuf::from("/workspace/templates"),
            next_process_index: 0,
        }
    }
    /// Set the guest directory under which template checkouts are created.
    #[must_use]
    pub fn with_checkout_root(mut self, checkout_root: impl Into<PathBuf>) -> Self {
        self.checkout_root = checkout_root.into();
        self
    }
    /// Return the guest checkout directory for `logical_id`.
    #[must_use]
    pub fn checkout_dir_for(&self, logical_id: &str) -> PathBuf {
        Self::checkout_dir_for_root(&self.checkout_root, logical_id)
    }
    /// Return the guest checkout directory for `logical_id` under `checkout_root`.
    #[must_use]
    pub fn checkout_dir_for_root(checkout_root: impl AsRef<Path>, logical_id: &str) -> PathBuf {
        checkout_root.as_ref().join(logical_id)
    }
    fn next_process_id(&mut self) -> String {
        let index = self.next_process_index;
        self.next_process_index += 1;
        format!("template-build-{index}")
    }
    async fn run_argv(
        &mut self,
        operation: &'static str,
        args: Vec<String>,
        working_dir: Option<PathBuf>,
    ) -> Result<Output, CoreTemplateCommandError> {
        let mut builder = ExecConfig::builder()
            .command(args)
            .stdout(Stdio::Piped)
            .stderr(Stdio::Piped);
        if let Some(working_dir) = working_dir {
            builder = builder.working_dir(working_dir);
        }
        let process_id = self.next_process_id();
        let output = self
            .container
            .exec(process_id, builder.build())
            .await
            .map_err(|source| CoreTemplateCommandError::Exec { operation, source })?
            .wait_with_output()
            .await
            .map_err(|source| CoreTemplateCommandError::Exec { operation, source })?;
        if !output.status.success() {
            return Err(CoreTemplateCommandError::Status {
                operation,
                status: output.status,
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        Ok(output)
    }
}
#[async_trait]
impl TemplateCommandRunner for CoreTemplateCommandRunner<'_> {
    type Error = CoreTemplateCommandError;
    async fn run_template_commands(
        &mut self,
        request: &TemplateBuildRuntimeRequest<'_>,
    ) -> Result<(), Self::Error> {
        let checkout_dir = self.checkout_dir_for(request.logical_id());
        let checkout_dir_string = checkout_dir.to_string_lossy().into_owned();
        self.run_argv(
            "create template checkout root",
            vec![
                "mkdir".to_owned(),
                "-p".to_owned(),
                self.checkout_root.to_string_lossy().into_owned(),
            ],
            None,
        )
        .await?;
        self.run_argv(
            "remove previous template checkout",
            vec![
                "rm".to_owned(),
                "-rf".to_owned(),
                checkout_dir_string.clone(),
            ],
            None,
        )
        .await?;
        self.run_argv(
            "clone template repository",
            vec![
                "git".to_owned(),
                "clone".to_owned(),
                "--quiet".to_owned(),
                request.job().repo().to_owned(),
                checkout_dir_string.clone(),
            ],
            None,
        )
        .await?;
        self.run_argv(
            "checkout template ref",
            vec![
                "git".to_owned(),
                "-C".to_owned(),
                checkout_dir_string.clone(),
                "checkout".to_owned(),
                "--quiet".to_owned(),
                request.job().checkout_ref().to_owned(),
            ],
            None,
        )
        .await?;
        for command in request.job().setup_commands() {
            self.run_argv(
                "run template setup command",
                vec!["/bin/sh".to_owned(), "-lc".to_owned(), command.clone()],
                Some(checkout_dir.clone()),
            )
            .await?;
        }
        for command in request.job().cache_warm_commands() {
            self.run_argv(
                "run template cache-warm command",
                vec!["/bin/sh".to_owned(), "-lc".to_owned(), command.clone()],
                Some(checkout_dir.clone()),
            )
            .await?;
        }
        Ok(())
    }
}
/// Report from a runtime template build snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeTemplateBuildReport {
    pub(crate) manifest: SnapshotArtifactManifest,
    setup_command_count: usize,
    cache_warm_command_count: usize,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
impl RuntimeTemplateBuildReport {
    /// Construct a runtime template build report.
    #[must_use]
    pub fn new(
        manifest: SnapshotArtifactManifest,
        setup_command_count: usize,
        cache_warm_command_count: usize,
        benchmark_samples: Vec<BenchmarkSample>,
    ) -> Self {
        Self {
            manifest,
            setup_command_count,
            cache_warm_command_count,
            benchmark_samples,
        }
    }
    /// Return output snapshot manifest.
    #[must_use]
    pub const fn manifest(&self) -> &SnapshotArtifactManifest {
        &self.manifest
    }
    /// Return setup command count.
    #[must_use]
    pub const fn setup_command_count(&self) -> usize {
        self.setup_command_count
    }
    /// Return cache-warm command count.
    #[must_use]
    pub const fn cache_warm_command_count(&self) -> usize {
        self.cache_warm_command_count
    }
    /// Return benchmark samples recorded during template build.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
}
/// Runtime-level template build snapshot orchestrator.
#[derive(Debug)]
pub struct RuntimeTemplateBuildSnapshot<'a> {
    job: &'a TemplateBuildJob,
    logical_id: String,
}
impl<'a> RuntimeTemplateBuildSnapshot<'a> {
    /// Construct a runtime template build snapshot operation.
    #[must_use]
    pub fn new(job: &'a TemplateBuildJob, logical_id: impl Into<String>) -> Self {
        Self {
            job,
            logical_id: logical_id.into(),
        }
    }
    /// Run template commands, save the snapshot, and record caller-provided elapsed samples.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeTemplateBuildError`] when command execution or snapshot save fails.
    pub async fn execute_with_elapsed<R, S>(
        self,
        command_runner: &mut R,
        snapshot_sink: &S,
        build_elapsed: Duration,
        snapshot_elapsed: Duration,
    ) -> Result<RuntimeTemplateBuildReport, RuntimeTemplateBuildError<R::Error>>
    where
        R: TemplateCommandRunner,
        S: TemplateSnapshotSink,
    {
        let mut probe = HostDiskPressureProbe::new();
        self.execute_with_disk_probe_elapsed(
            command_runner,
            snapshot_sink,
            build_elapsed,
            snapshot_elapsed,
            &mut probe,
        )
        .await
    }
    /// Run template commands and save the snapshot after checking host disk pressure.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeTemplateBuildError`] when disk admission, command
    /// execution, or snapshot save fails.
    pub async fn execute_with_disk_probe_elapsed<R, S, P>(
        self,
        command_runner: &mut R,
        snapshot_sink: &S,
        build_elapsed: Duration,
        snapshot_elapsed: Duration,
        disk_probe: &mut P,
    ) -> Result<RuntimeTemplateBuildReport, RuntimeTemplateBuildError<R::Error>>
    where
        R: TemplateCommandRunner,
        S: TemplateSnapshotSink,
        P: DiskPressureProbe,
    {
        let disk_root = snapshot_output_disk_root(self.job.snapshot_output_path());
        RuntimeDiskPressureGuard::new(disk_root, DEFAULT_RUNTIME_MINIMUM_FREE_DISK)
            .check(disk_probe)
            .map_err(|error| {
                RuntimeTemplateBuildError::Capacity(disk_pressure_to_capacity_error(&error))
            })?;
        let request = TemplateBuildRuntimeRequest::new(self.job, &self.logical_id);
        command_runner
            .run_template_commands(&request)
            .await
            .map_err(|source| RuntimeTemplateBuildError::Commands { source })?;
        snapshot_sink
            .save_snapshot(self.job.snapshot_output_path())
            .await
            .map_err(|source| RuntimeTemplateBuildError::Snapshot { source })?;
        let manifest = self.job.snapshot_manifest(self.logical_id);
        write_snapshot_artifact_sidecars(&manifest)
            .map_err(|source| RuntimeTemplateBuildError::Snapshot { source })?;
        let benchmark_samples = vec![
            BenchmarkSample::new(
                "cold_template_build",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                build_elapsed.as_secs_f64() * 1000.0,
            ),
            BenchmarkSample::new(
                "snapshot_save",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                snapshot_elapsed.as_secs_f64() * 1000.0,
            ),
        ];
        Ok(RuntimeTemplateBuildReport::new(
            manifest,
            self.job.setup_commands().len(),
            self.job.cache_warm_commands().len(),
            benchmark_samples,
        ))
    }
}
