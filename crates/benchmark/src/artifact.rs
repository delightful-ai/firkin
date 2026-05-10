//! artifact — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_evidence::AutoscaleEfficiencyScorecardError;
#[allow(unused_imports)]
use firkin_evidence::{AgentBenchmarkScorecardArtifact, AgentBenchmarkScorecardReport};
#[allow(unused_imports)]
use firkin_evidence::{AgentBenchmarkScorecardError, BenchmarkEvidenceError};
#[allow(unused_imports)]
use firkin_evidence::{
    AgentComputerScorecardArtifact, AgentComputerScorecardError, AgentComputerScorecardReport,
};
#[allow(unused_imports)]
use firkin_evidence::{AutoscaleEfficiencyScorecardArtifact, AutoscaleEfficiencyScorecardReport};
#[allow(unused_imports)]
use firkin_evidence::{BenchmarkEvidenceArtifact, BenchmarkEvidenceReport};
#[allow(unused_imports)]
use firkin_evidence::{BenchmarkOverheadEvidenceArtifact, BenchmarkOverheadEvidenceReport};
#[allow(unused_imports)]
use firkin_trace::{BenchmarkSample, SandboxEventTrace};
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Runtime benchmark evidence write error.
#[derive(Debug, ThisError)]
pub enum RuntimeBenchmarkEvidenceError {
    /// Benchmark samples did not satisfy production evidence requirements.
    #[error("runtime benchmark evidence is invalid: {0}")]
    Evidence(#[from] BenchmarkEvidenceError),
    /// Benchmark samples did not satisfy agent scorecard evidence requirements.
    #[error("runtime agent scorecard evidence is invalid: {0}")]
    Scorecard(#[from] AgentBenchmarkScorecardError),
    /// Benchmark samples did not satisfy autoscale efficiency scorecard evidence requirements.
    #[error("runtime autoscale efficiency scorecard evidence is invalid: {0}")]
    AutoscaleScorecard(#[from] AutoscaleEfficiencyScorecardError),
    /// Benchmark samples did not satisfy agent-computer scorecard evidence requirements.
    #[error("runtime agent-computer scorecard evidence is invalid: {0}")]
    AgentComputerScorecard(#[from] AgentComputerScorecardError),
    /// Benchmark evidence artifact could not be written.
    #[error("runtime benchmark evidence artifact write failed: {0}")]
    Io(#[from] io::Error),
}
/// Runtime-owned writer for validated benchmark evidence artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeBenchmarkEvidenceWriter {
    #[allow(missing_docs)]
    pub path: PathBuf,
}
impl RuntimeBenchmarkEvidenceWriter {
    /// Construct a benchmark evidence writer for `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    /// Return the artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Return the raw sample sidecar path for this lifecycle artifact.
    #[must_use]
    pub fn samples_path(&self) -> PathBuf {
        sidecar_path_for(&self.path, "samples")
    }
    /// Return the raw event trace sidecar path for this lifecycle artifact.
    #[must_use]
    pub fn traces_path(&self) -> PathBuf {
        sidecar_path_for(&self.path, "traces")
    }
    /// Validate runtime samples and persist a benchmark evidence artifact.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeBenchmarkEvidenceError`] when samples do not cover the
    /// required lifecycle latency metrics or the artifact cannot be written.
    pub fn write_samples(
        &self,
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> Result<BenchmarkEvidenceReport, RuntimeBenchmarkEvidenceError> {
        self.write_samples_with_traces(samples, [])
    }
    /// Validate runtime samples and persist lifecycle evidence plus raw sidecars.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeBenchmarkEvidenceError`] when samples do not cover the
    /// required lifecycle latency metrics or any artifact cannot be written.
    pub fn write_samples_with_traces(
        &self,
        samples: impl IntoIterator<Item = BenchmarkSample>,
        traces: impl IntoIterator<Item = SandboxEventTrace>,
    ) -> Result<BenchmarkEvidenceReport, RuntimeBenchmarkEvidenceError> {
        let samples = samples.into_iter().collect::<Vec<_>>();
        let traces = traces.into_iter().collect::<Vec<_>>();
        let report = BenchmarkEvidenceReport::from_samples(samples.clone())?;
        BenchmarkEvidenceArtifact::write_json(&self.path, &report)?;
        write_raw_samples_json(&self.samples_path(), &samples)?;
        write_raw_traces_json(&self.traces_path(), &traces)?;
        Ok(report)
    }
}

/// Runtime-owned writer for validated agent scorecard evidence artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeAgentScorecardEvidenceWriter {
    #[allow(missing_docs)]
    pub path: PathBuf,
    min_samples: usize,
}

impl RuntimeAgentScorecardEvidenceWriter {
    /// Construct an agent scorecard evidence writer for `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            min_samples: 1,
        }
    }

    /// Require at least `min_samples` for every P0 dashboard metric.
    #[must_use]
    pub const fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.min_samples = min_samples;
        self
    }

    /// Return the artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Validate runtime samples and persist an agent scorecard artifact.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeBenchmarkEvidenceError`] when samples do not cover the
    /// required P0 dashboard metrics or the artifact cannot be written.
    pub fn write_samples(
        &self,
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> Result<AgentBenchmarkScorecardReport, RuntimeBenchmarkEvidenceError> {
        let report = AgentBenchmarkScorecardReport::from_samples_with_min_samples(
            samples,
            self.min_samples,
        )?;
        AgentBenchmarkScorecardArtifact::write_json(&self.path, &report)?;
        Ok(report)
    }
}

/// Runtime-owned writer for validated autoscale efficiency scorecard artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeAutoscaleScorecardEvidenceWriter {
    #[allow(missing_docs)]
    pub path: PathBuf,
    min_samples: usize,
}

impl RuntimeAutoscaleScorecardEvidenceWriter {
    /// Construct an autoscale efficiency scorecard evidence writer for `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            min_samples: 1,
        }
    }

    /// Require at least `min_samples` for every autoscale efficiency scorecard metric.
    #[must_use]
    pub const fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.min_samples = min_samples;
        self
    }

    /// Return the artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the raw sample sidecar path for this scorecard artifact.
    #[must_use]
    pub fn samples_path(&self) -> PathBuf {
        sidecar_path_for(&self.path, "samples")
    }

    /// Return the raw event trace sidecar path for this scorecard artifact.
    #[must_use]
    pub fn traces_path(&self) -> PathBuf {
        sidecar_path_for(&self.path, "traces")
    }

    /// Validate runtime samples and persist an autoscale efficiency scorecard artifact.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeBenchmarkEvidenceError`] when samples do not cover the
    /// required autoscale efficiency dashboard metrics or the artifact cannot be written.
    pub fn write_samples(
        &self,
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> Result<AutoscaleEfficiencyScorecardReport, RuntimeBenchmarkEvidenceError> {
        self.write_samples_with_traces(samples, [])
    }

    /// Validate runtime samples and persist an autoscale scorecard plus raw trace sidecars.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeBenchmarkEvidenceError`] when samples do not cover the
    /// required autoscale efficiency dashboard metrics or any artifact cannot be written.
    pub fn write_samples_with_traces(
        &self,
        samples: impl IntoIterator<Item = BenchmarkSample>,
        traces: impl IntoIterator<Item = SandboxEventTrace>,
    ) -> Result<AutoscaleEfficiencyScorecardReport, RuntimeBenchmarkEvidenceError> {
        let samples = samples.into_iter().collect::<Vec<_>>();
        let traces = traces.into_iter().collect::<Vec<_>>();
        let report = AutoscaleEfficiencyScorecardReport::from_samples_with_min_samples(
            samples.clone(),
            self.min_samples,
        )?;
        AutoscaleEfficiencyScorecardArtifact::write_json(&self.path, &report)?;
        write_raw_samples_json(&self.samples_path(), &samples)?;
        write_raw_traces_json(&self.traces_path(), &traces)?;
        Ok(report)
    }
}

/// Runtime-owned writer for validated agent-computer scorecard artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeAgentComputerScorecardEvidenceWriter {
    #[allow(missing_docs)]
    pub path: PathBuf,
    min_samples: usize,
}

impl RuntimeAgentComputerScorecardEvidenceWriter {
    /// Construct an agent-computer scorecard evidence writer for `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            min_samples: 1,
        }
    }

    /// Require at least `min_samples` for every agent-computer scorecard metric.
    #[must_use]
    pub const fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.min_samples = min_samples;
        self
    }

    /// Return the artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the raw sample sidecar path for this scorecard artifact.
    #[must_use]
    pub fn samples_path(&self) -> PathBuf {
        sidecar_path_for(&self.path, "samples")
    }

    /// Return the raw event trace sidecar path for this scorecard artifact.
    #[must_use]
    pub fn traces_path(&self) -> PathBuf {
        sidecar_path_for(&self.path, "traces")
    }

    /// Validate runtime samples and persist an agent-computer scorecard artifact.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeBenchmarkEvidenceError`] when samples do not cover the
    /// required product-path dashboard metrics or the artifact cannot be written.
    pub fn write_samples(
        &self,
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> Result<AgentComputerScorecardReport, RuntimeBenchmarkEvidenceError> {
        self.write_samples_with_traces(samples, [])
    }

    /// Validate runtime samples and persist an agent-computer scorecard plus raw trace sidecars.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeBenchmarkEvidenceError`] when samples do not cover the
    /// required product-path dashboard metrics or any artifact cannot be written.
    pub fn write_samples_with_traces(
        &self,
        samples: impl IntoIterator<Item = BenchmarkSample>,
        traces: impl IntoIterator<Item = SandboxEventTrace>,
    ) -> Result<AgentComputerScorecardReport, RuntimeBenchmarkEvidenceError> {
        let samples = samples.into_iter().collect::<Vec<_>>();
        let traces = traces.into_iter().collect::<Vec<_>>();
        let report = AgentComputerScorecardReport::from_samples_with_min_samples(
            samples.clone(),
            self.min_samples,
        )?;
        AgentComputerScorecardArtifact::write_json(&self.path, &report)?;
        write_raw_samples_json(&self.samples_path(), &samples)?;
        write_raw_traces_json(&self.traces_path(), &traces)?;
        Ok(report)
    }
}

fn sidecar_path_for(path: &Path, kind: &str) -> PathBuf {
    let stem = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .and_then(|name| name.strip_suffix(".json"))
        .unwrap_or("benchmark");
    let file_name = format!("{stem}.{kind}.json");
    path.with_file_name(file_name)
}

fn write_raw_samples_json(path: &Path, samples: &[BenchmarkSample]) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(samples).map_err(io::Error::other)?;
    std::fs::write(path, bytes)
}

fn write_raw_traces_json(path: &Path, traces: &[SandboxEventTrace]) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(traces).map_err(io::Error::other)?;
    std::fs::write(path, bytes)
}
/// Runtime-owned writer for validated Firkin overhead evidence artifacts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeOverheadEvidenceWriter {
    #[allow(missing_docs)]
    pub path: PathBuf,
}
impl RuntimeOverheadEvidenceWriter {
    /// Construct an overhead evidence writer for `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    /// Return the artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Validate runtime overhead samples and persist an evidence artifact.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeBenchmarkEvidenceError`] when samples do not cover the
    /// required Firkin overhead metrics or the artifact cannot be written.
    pub fn write_samples(
        &self,
        samples: impl IntoIterator<Item = BenchmarkSample>,
    ) -> Result<BenchmarkOverheadEvidenceReport, RuntimeBenchmarkEvidenceError> {
        let report = BenchmarkOverheadEvidenceReport::from_samples(samples)?;
        BenchmarkOverheadEvidenceArtifact::write_json(&self.path, &report)?;
        Ok(report)
    }
}
