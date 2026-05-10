//! build — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::command::{run, run_shell};
#[allow(unused_imports)]
use crate::model::TemplateBuildJob;
#[allow(unused_imports)]
use crate::snapshot::{SnapshotSinkError, TemplateSnapshotSink};
#[allow(unused_imports)]
use firkin_artifacts::SnapshotArtifactManifest;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::process::Command;
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Template build execution error.
#[derive(Debug, ThisError)]
pub enum TemplateBuildError {
    /// Filesystem operation failed.
    #[error("template build filesystem operation failed while {operation}: {source}")]
    Io {
        /// Operation being attempted.
        operation: &'static str,
        /// Source error.
        #[source]
        source: std::io::Error,
    },
    /// External command failed.
    #[error("template build command failed while {operation}: {status}")]
    Command {
        /// Operation being attempted.
        operation: &'static str,
        /// Command status.
        status: std::process::ExitStatus,
    },
    /// Snapshot sink failed.
    #[error("template build snapshot sink failed: {source}")]
    Snapshot {
        /// Source error.
        #[source]
        source: SnapshotSinkError,
    },
}
/// Local template build executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateBuildExecutor {
    root: PathBuf,
}
impl TemplateBuildExecutor {
    /// Construct an executor rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    /// Execute a template build job.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateBuildError`] when clone, checkout, command execution,
    /// or directory creation fails.
    pub fn execute(
        &self,
        job: &TemplateBuildJob,
        logical_id: impl Into<String>,
    ) -> Result<TemplateBuildReport, TemplateBuildError> {
        self.execute_inner(job, logical_id)
    }
    /// Execute a template build job and save the resulting snapshot artifact.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateBuildError`] when build execution or snapshot saving
    /// fails.
    pub async fn execute_with_snapshot_sink<S: TemplateSnapshotSink>(
        &self,
        job: &TemplateBuildJob,
        logical_id: impl Into<String>,
        snapshot_sink: &S,
    ) -> Result<TemplateBuildReport, TemplateBuildError> {
        self.execute_inner_with_snapshot_sink(job, logical_id, snapshot_sink)
            .await
    }
    fn execute_inner(
        &self,
        job: &TemplateBuildJob,
        logical_id: impl Into<String>,
    ) -> Result<TemplateBuildReport, TemplateBuildError> {
        let logical_id = logical_id.into();
        let checkout = self.prepare_checkout(job, &logical_id)?;
        let elapsed_ms = checkout.started.elapsed().as_secs_f64() * 1000.0;
        let benchmark_samples = vec![BenchmarkSample::new(
            "cold_template_build",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            elapsed_ms,
        )];
        Ok(TemplateBuildReport {
            checkout_dir: checkout.checkout_dir,
            setup_command_count: checkout.setup_command_count,
            cache_warm_command_count: checkout.cache_warm_command_count,
            manifest: job.snapshot_manifest(logical_id),
            benchmark_samples,
        })
    }
    async fn execute_inner_with_snapshot_sink<S: TemplateSnapshotSink>(
        &self,
        job: &TemplateBuildJob,
        logical_id: impl Into<String>,
        snapshot_sink: &S,
    ) -> Result<TemplateBuildReport, TemplateBuildError> {
        let logical_id = logical_id.into();
        let checkout = self.prepare_checkout(job, &logical_id)?;
        let snapshot_started = Instant::now();
        snapshot_sink
            .save_snapshot(job.snapshot_output_path())
            .await
            .map_err(|source| TemplateBuildError::Snapshot { source })?;
        let benchmark_samples = vec![
            BenchmarkSample::new(
                "snapshot_save",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                snapshot_started.elapsed().as_secs_f64() * 1000.0,
            ),
            BenchmarkSample::new(
                "cold_template_build",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                checkout.started.elapsed().as_secs_f64() * 1000.0,
            ),
        ];
        Ok(TemplateBuildReport {
            checkout_dir: checkout.checkout_dir,
            setup_command_count: checkout.setup_command_count,
            cache_warm_command_count: checkout.cache_warm_command_count,
            manifest: job.snapshot_manifest(logical_id),
            benchmark_samples,
        })
    }
    fn prepare_checkout(
        &self,
        job: &TemplateBuildJob,
        logical_id: &str,
    ) -> Result<PreparedTemplateCheckout, TemplateBuildError> {
        let started = Instant::now();
        let checkout_dir = self.root.join("checkouts").join(logical_id);
        if checkout_dir.exists() {
            fs::remove_dir_all(&checkout_dir).map_err(|source| TemplateBuildError::Io {
                operation: "remove previous template checkout",
                source,
            })?;
        }
        let checkout_parent = checkout_dir.parent().expect("checkout path has parent");
        fs::create_dir_all(checkout_parent).map_err(|source| TemplateBuildError::Io {
            operation: "create template checkout parent",
            source,
        })?;
        let snapshot_parent = job
            .snapshot_output_path()
            .parent()
            .expect("snapshot output path has parent");
        fs::create_dir_all(snapshot_parent).map_err(|source| TemplateBuildError::Io {
            operation: "create template snapshot parent",
            source,
        })?;
        run(
            Command::new("git")
                .arg("clone")
                .arg("--quiet")
                .arg(job.repo())
                .arg(&checkout_dir),
            "clone template repository",
        )?;
        run(
            Command::new("git")
                .arg("-C")
                .arg(&checkout_dir)
                .arg("checkout")
                .arg("--quiet")
                .arg(job.checkout_ref()),
            "checkout template ref",
        )?;
        for command in job.setup_commands() {
            run_shell(command, &checkout_dir, "run template setup command")?;
        }
        for command in job.cache_warm_commands() {
            run_shell(command, &checkout_dir, "run template cache-warm command")?;
        }
        Ok(PreparedTemplateCheckout {
            checkout_dir,
            setup_command_count: job.setup_commands().len(),
            cache_warm_command_count: job.cache_warm_commands().len(),
            started,
        })
    }
}
#[derive(Debug)]
struct PreparedTemplateCheckout {
    checkout_dir: PathBuf,
    setup_command_count: usize,
    cache_warm_command_count: usize,
    started: Instant,
}
/// Template build execution report.
#[derive(Clone, Debug, PartialEq)]
pub struct TemplateBuildReport {
    checkout_dir: PathBuf,
    setup_command_count: usize,
    cache_warm_command_count: usize,
    manifest: SnapshotArtifactManifest,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
impl TemplateBuildReport {
    /// Return checkout directory.
    #[must_use]
    pub fn checkout_dir(&self) -> &Path {
        &self.checkout_dir
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
    /// Return output snapshot manifest.
    #[must_use]
    pub const fn manifest(&self) -> &SnapshotArtifactManifest {
        &self.manifest
    }
    /// Return benchmark samples emitted by this build.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
}
