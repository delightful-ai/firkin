//! soak — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_evidence::{SoakCleanupEvidence, SoakEvidenceReport, SoakStep};
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use {
    firkin_e2b_contract::{BackendError, RuntimeAdapter},
    firkin_e2b_server::LocalRuntimeBackend,
    firkin_e2b_wire::{CreateSnapshotRequest, FollowupSandboxCreateRequest},
    firkin_envd::{EnvdFilesystemAdapter, EnvdProcessAdapter},
};
#[allow(unused_imports)]
use {firkin_e2b_wire::SandboxCreateRequest, firkin_envd::EnvdProcessStartRequest};
/// Product-route single-node soak runner configuration.
#[derive(Clone, Debug)]
pub struct RuntimeProductSoakConfig {
    duration: Duration,
    #[allow(missing_docs)]
    pub create_request: SandboxCreateRequest,
    command_request: EnvdProcessStartRequest,
    file_path: String,
    file_contents: Vec<u8>,
    snapshot_prefix: String,
    iteration_pause: Duration,
    benchmark_artifact: String,
}
impl RuntimeProductSoakConfig {
    /// Construct an Inspect-like soak configuration.
    #[must_use]
    pub fn inspect_like(duration: Duration, create_request: SandboxCreateRequest) -> Self {
        Self {
            duration,
            create_request,
            command_request: EnvdProcessStartRequest {
                cmd: "/bin/sh".to_owned(),
                args: vec!["-lc".to_owned(), "printf firkin-soak".to_owned()],
                ..EnvdProcessStartRequest::default()
            },
            file_path: "/tmp/firkin-soak-marker".to_owned(),
            file_contents: b"firkin-soak\n".to_vec(),
            snapshot_prefix: "firkin-soak".to_owned(),
            iteration_pause: Duration::from_secs(30),
            benchmark_artifact: "target/firkin-live-evidence/live-benchmark-evidence.json"
                .to_owned(),
        }
    }
    /// Return requested soak duration.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }
    /// Override the snapshot id prefix.
    #[must_use]
    pub fn with_snapshot_prefix(mut self, snapshot_prefix: impl Into<String>) -> Self {
        self.snapshot_prefix = snapshot_prefix.into();
        self
    }
    /// Override the pause between loop iterations.
    #[must_use]
    pub const fn with_iteration_pause(mut self, iteration_pause: Duration) -> Self {
        self.iteration_pause = iteration_pause;
        self
    }
    /// Override the referenced lifecycle benchmark artifact.
    #[must_use]
    pub fn with_benchmark_artifact(mut self, benchmark_artifact: impl Into<String>) -> Self {
        self.benchmark_artifact = benchmark_artifact.into();
        self
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct RuntimeProductSoakStepCounts {
    attempts: u64,
    failures: u64,
}
/// Product-route single-node soak runner.
pub struct RuntimeProductSoakRunner<A>
where
    A: RuntimeAdapter,
{
    #[allow(missing_docs)]
    pub backend: LocalRuntimeBackend<A>,
    #[allow(missing_docs)]
    pub config: RuntimeProductSoakConfig,
    steps: [RuntimeProductSoakStepCounts; 7],
}
impl<A> RuntimeProductSoakRunner<A>
where
    A: RuntimeAdapter
        + EnvdProcessAdapter<Error = BackendError>
        + EnvdFilesystemAdapter<Error = BackendError>,
{
    /// Construct a product-route soak runner.
    #[must_use]
    pub const fn new(backend: LocalRuntimeBackend<A>, config: RuntimeProductSoakConfig) -> Self {
        Self {
            backend,
            config,
            steps: [RuntimeProductSoakStepCounts {
                attempts: 0,
                failures: 0,
            }; 7],
        }
    }
    /// Return the backend after the soak run.
    #[must_use]
    pub const fn backend(&self) -> &LocalRuntimeBackend<A> {
        &self.backend
    }
    /// Run the soak loop and return evidence for the executed steps.
    pub async fn run(&mut self) -> SoakEvidenceReport {
        let started = Instant::now();
        let mut iteration = 0_u64;
        loop {
            iteration = iteration.saturating_add(1);
            self.run_iteration(iteration).await;
            if started.elapsed() >= self.config.duration {
                break;
            }
            tokio::time::sleep(self.config.iteration_pause).await;
        }
        SoakEvidenceReport::new(
            self.config.duration,
            SoakStep::required_inspect_loop().map(|step| {
                let counts = self.steps[soak_step_index(step)];
                (step, counts.attempts, counts.failures)
            }),
        )
        .with_benchmark_artifact(self.config.benchmark_artifact.clone())
        .with_cleanup_evidence(self.cleanup_evidence())
    }
    fn cleanup_evidence(&self) -> SoakCleanupEvidence {
        SoakCleanupEvidence::new(
            self.backend.sandboxes().list().len() as u64,
            self.backend.sandboxes().list_snapshots(None).len() as u64,
            0,
            0,
        )
    }
    #[allow(clippy::too_many_lines)]
    async fn run_iteration(&mut self, iteration: u64) {
        let snapshot_id = format!("{}-{iteration}", self.config.snapshot_prefix);
        let create_result = self
            .backend
            .create(self.config.create_request.clone())
            .await;
        let create = self.record_result(SoakStep::CreateSandbox, create_result);
        let Ok(connected) = create else {
            self.record_cleanup(None, None, None).await;
            return;
        };
        let mut source_sandbox_id = Some(connected.sandbox_id.clone());
        if self
            .record_result(
                SoakStep::RunCommand,
                self.backend
                    .adapter()
                    .start_process(self.config.command_request.clone())
                    .await,
            )
            .is_err()
        {
            self.record_cleanup(source_sandbox_id.take(), None, None)
                .await;
            return;
        }
        if self
            .record_result(
                SoakStep::WriteFile,
                self.backend
                    .adapter()
                    .write_file(
                        self.config.file_path.clone(),
                        self.config.file_contents.clone(),
                    )
                    .await,
            )
            .is_err()
        {
            self.record_cleanup(source_sandbox_id.take(), None, None)
                .await;
            return;
        }
        let snapshot_result = self
            .backend
            .create_snapshot(
                &connected.sandbox_id,
                CreateSnapshotRequest {
                    name: Some(snapshot_id.clone()),
                },
            )
            .await;
        if self
            .record_result(SoakStep::SaveSnapshot, snapshot_result)
            .is_err()
        {
            self.record_cleanup(source_sandbox_id.take(), None, None)
                .await;
            return;
        }
        if let Some(source) = source_sandbox_id.take()
            && self.backend.delete(&source).await.is_err()
        {
            self.record_failure(SoakStep::Cleanup);
            return;
        }
        let followup_result = self
            .backend
            .create_followup(FollowupSandboxCreateRequest {
                snapshot_id: snapshot_id.clone(),
                create_request: SandboxCreateRequest::default(),
            })
            .await;
        let followup = self.record_result(SoakStep::RestoreSnapshot, followup_result);
        let Ok(followup) = followup else {
            self.record_cleanup(None, None, Some(snapshot_id)).await;
            return;
        };
        let mut followup_sandbox_id = Some(followup.sandbox_id.clone());
        let followup_command = EnvdProcessStartRequest {
            cmd: "/bin/cat".to_owned(),
            args: vec![self.config.file_path.clone()],
            ..EnvdProcessStartRequest::default()
        };
        let _ = self.record_result(
            SoakStep::FollowUpPrompt,
            self.backend.adapter().start_process(followup_command).await,
        );
        self.record_cleanup(None, followup_sandbox_id.take(), Some(snapshot_id))
            .await;
    }
    fn record_result<T>(
        &mut self,
        step: SoakStep,
        result: Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        let counts = &mut self.steps[soak_step_index(step)];
        counts.attempts = counts.attempts.saturating_add(1);
        if result.is_err() {
            counts.failures = counts.failures.saturating_add(1);
        }
        result
    }
    async fn record_cleanup(
        &mut self,
        source: Option<String>,
        followup: Option<String>,
        snapshot: Option<String>,
    ) {
        let counts = &mut self.steps[soak_step_index(SoakStep::Cleanup)];
        counts.attempts = counts.attempts.saturating_add(1);
        let mut failed = false;
        if let Some(source) = source
            && self.backend.delete(&source).await.is_err()
        {
            failed = true;
        }
        if let Some(followup) = followup
            && self.backend.delete(&followup).await.is_err()
        {
            failed = true;
        }
        if let Some(snapshot) = snapshot
            && self.backend.delete_snapshot(&snapshot).await.is_err()
        {
            failed = true;
        }
        if failed {
            counts.failures = counts.failures.saturating_add(1);
        }
    }
    fn record_failure(&mut self, step: SoakStep) {
        let counts = &mut self.steps[soak_step_index(step)];
        counts.attempts = counts.attempts.saturating_add(1);
        counts.failures = counts.failures.saturating_add(1);
    }
}
const fn soak_step_index(step: SoakStep) -> usize {
    match step {
        SoakStep::CreateSandbox => 0,
        SoakStep::RunCommand => 1,
        SoakStep::WriteFile => 2,
        SoakStep::SaveSnapshot => 3,
        SoakStep::RestoreSnapshot => 4,
        SoakStep::FollowUpPrompt => 5,
        SoakStep::Cleanup => 6,
    }
}
