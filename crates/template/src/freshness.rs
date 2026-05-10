//! freshness — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::build::TemplateBuildError;
#[allow(unused_imports)]
use crate::command::run;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::process::Command;
#[allow(unused_imports)]
use std::time::Instant;
/// Freshness sync phase after restoring a prepared snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreshnessSyncPhase {
    /// Snapshot has been restored but branch sync has not started.
    Restored,
    /// Branch sync is running; read-only operations may proceed.
    Syncing,
    /// Sync completed and writes may proceed.
    Ready,
    /// Sync failed; writes remain blocked.
    Failed,
}
/// Read/write gate for restored snapshot freshness sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshnessSyncGate {
    branch: String,
    phase: FreshnessSyncPhase,
    sync_target: Option<String>,
    synced_commit: Option<String>,
    failure_reason: Option<String>,
}
impl FreshnessSyncGate {
    /// Construct a gate immediately after snapshot restore.
    #[must_use]
    pub fn restored(branch: impl Into<String>) -> Self {
        Self {
            branch: branch.into(),
            phase: FreshnessSyncPhase::Restored,
            sync_target: None,
            synced_commit: None,
            failure_reason: None,
        }
    }
    /// Mark branch sync as started.
    #[must_use]
    pub fn begin_sync(mut self, target: impl Into<String>) -> Self {
        self.phase = FreshnessSyncPhase::Syncing;
        self.sync_target = Some(target.into());
        self
    }
    /// Mark branch sync as complete.
    #[must_use]
    pub fn complete_sync(mut self, commit: impl Into<String>) -> Self {
        self.phase = FreshnessSyncPhase::Ready;
        self.synced_commit = Some(commit.into());
        self.failure_reason = None;
        self
    }
    /// Mark branch sync as failed.
    #[must_use]
    pub fn fail_sync(mut self, reason: impl Into<String>) -> Self {
        self.phase = FreshnessSyncPhase::Failed;
        self.failure_reason = Some(reason.into());
        self
    }
    /// Return the requested branch.
    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }
    /// Return the sync phase.
    #[must_use]
    pub const fn phase(&self) -> FreshnessSyncPhase {
        self.phase
    }
    /// Return whether read-only operations are allowed.
    #[must_use]
    pub const fn reads_allowed(&self) -> bool {
        matches!(
            self.phase,
            FreshnessSyncPhase::Restored
                | FreshnessSyncPhase::Syncing
                | FreshnessSyncPhase::Ready
                | FreshnessSyncPhase::Failed
        )
    }
    /// Return whether write operations are allowed.
    #[must_use]
    pub const fn writes_allowed(&self) -> bool {
        matches!(self.phase, FreshnessSyncPhase::Ready)
    }
    /// Return the sync target commit or revision.
    #[must_use]
    pub fn sync_target(&self) -> Option<&str> {
        self.sync_target.as_deref()
    }
    /// Return the synced commit.
    #[must_use]
    pub fn synced_commit(&self) -> Option<&str> {
        self.synced_commit.as_deref()
    }
    /// Return the failure reason.
    #[must_use]
    pub fn failure_reason(&self) -> Option<&str> {
        self.failure_reason.as_deref()
    }
}
/// Git-backed freshness sync executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreshnessSyncExecutor {
    branch: String,
}
impl FreshnessSyncExecutor {
    /// Construct a freshness sync executor for `branch`.
    #[must_use]
    pub fn new(branch: impl Into<String>) -> Self {
        Self {
            branch: branch.into(),
        }
    }
    /// Fast-forward a checkout to `target`.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateBuildError`] when git sync commands fail.
    pub fn sync_checkout(
        &self,
        checkout_dir: impl AsRef<Path>,
        target: impl Into<String>,
    ) -> Result<FreshnessSyncReport, TemplateBuildError> {
        let started = Instant::now();
        let checkout_dir = checkout_dir.as_ref();
        let target = target.into();
        let gate = FreshnessSyncGate::restored(&self.branch).begin_sync(&target);
        let branch_name = self
            .branch
            .strip_prefix("refs/heads/")
            .unwrap_or(&self.branch);
        run(
            Command::new("git")
                .arg("-C")
                .arg(checkout_dir)
                .arg("fetch")
                .arg("--quiet")
                .arg("origin")
                .arg(branch_name),
            "fetch freshness sync target",
        )?;
        run(
            Command::new("git")
                .arg("-C")
                .arg(checkout_dir)
                .arg("checkout")
                .arg("--quiet")
                .arg(branch_name),
            "checkout freshness sync branch",
        )?;
        run(
            Command::new("git")
                .arg("-C")
                .arg(checkout_dir)
                .arg("reset")
                .arg("--hard")
                .arg("--quiet")
                .arg(&target),
            "reset freshness sync target",
        )?;
        Ok(FreshnessSyncReport {
            gate: gate.complete_sync(&target),
            benchmark_samples: vec![BenchmarkSample::new(
                "freshness_sync",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                started.elapsed().as_secs_f64() * 1000.0,
            )],
        })
    }
}
/// Freshness sync execution report.
#[derive(Clone, Debug, PartialEq)]
pub struct FreshnessSyncReport {
    gate: FreshnessSyncGate,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
impl FreshnessSyncReport {
    /// Return the final freshness sync gate.
    #[must_use]
    pub const fn gate(&self) -> &FreshnessSyncGate {
        &self.gate
    }
    /// Return benchmark samples emitted by freshness sync.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
}
