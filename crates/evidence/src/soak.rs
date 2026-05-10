//! soak — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Step required by an Inspect-like single-node soak loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum SoakStep {
    /// Create a sandbox.
    CreateSandbox,
    /// Run a command in the sandbox.
    RunCommand,
    /// Write a file in the sandbox.
    WriteFile,
    /// Save a durable snapshot.
    SaveSnapshot,
    /// Restore from a snapshot.
    RestoreSnapshot,
    /// Run a follow-up prompt against restored state.
    FollowUpPrompt,
    /// Clean up sandbox and artifacts.
    Cleanup,
}
impl SoakStep {
    /// Return the required Inspect-like loop steps.
    #[must_use]
    pub const fn required_inspect_loop() -> [Self; 7] {
        [
            Self::CreateSandbox,
            Self::RunCommand,
            Self::WriteFile,
            Self::SaveSnapshot,
            Self::RestoreSnapshot,
            Self::FollowUpPrompt,
            Self::Cleanup,
        ]
    }
}
/// Single-node soak scenario contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoakScenario {
    duration: Duration,
    steps: Vec<SoakStep>,
}
impl SoakScenario {
    /// Construct a soak scenario.
    #[must_use]
    pub fn new(duration: Duration, steps: impl IntoIterator<Item = SoakStep>) -> Self {
        Self {
            duration,
            steps: steps.into_iter().collect(),
        }
    }
    /// Construct the required Inspect-like scenario.
    #[must_use]
    pub fn inspect_like(duration: Duration) -> Self {
        Self::new(duration, SoakStep::required_inspect_loop())
    }
    /// Return the scenario duration.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }
    /// Return the scenario steps.
    #[must_use]
    pub fn steps(&self) -> &[SoakStep] {
        &self.steps
    }
    /// Return whether the scenario contains a step.
    #[must_use]
    pub fn contains_step(&self, step: SoakStep) -> bool {
        self.steps.contains(&step)
    }
    /// Return whether this scenario can count as production soak evidence.
    #[must_use]
    pub fn is_production_evidence(&self) -> bool {
        self.duration >= Duration::from_hours(24)
            && SoakStep::required_inspect_loop()
                .into_iter()
                .all(|step| self.contains_step(step))
    }
}
/// Evidence for one executed soak step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SoakStepEvidence {
    step: SoakStep,
    attempts: u64,
    failures: u64,
}
impl SoakStepEvidence {
    /// Construct step evidence.
    #[must_use]
    pub const fn new(step: SoakStep, attempts: u64, failures: u64) -> Self {
        Self {
            step,
            attempts,
            failures,
        }
    }
    /// Return the soak step.
    #[must_use]
    pub const fn step(&self) -> SoakStep {
        self.step
    }
    /// Return attempt count.
    #[must_use]
    pub const fn attempts(&self) -> u64 {
        self.attempts
    }
    /// Return failure count.
    #[must_use]
    pub const fn failures(&self) -> u64 {
        self.failures
    }
}
/// Single-node soak evidence artifact content.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SoakEvidenceReport {
    duration_seconds: u64,
    steps: Vec<SoakStepEvidence>,
    benchmark_artifact: Option<String>,
    cleanup_evidence: Option<SoakCleanupEvidence>,
}
impl SoakEvidenceReport {
    /// Construct a soak evidence report.
    #[must_use]
    pub fn new(duration: Duration, steps: impl IntoIterator<Item = (SoakStep, u64, u64)>) -> Self {
        Self {
            duration_seconds: duration.as_secs(),
            steps: steps
                .into_iter()
                .map(|(step, attempts, failures)| SoakStepEvidence::new(step, attempts, failures))
                .collect(),
            benchmark_artifact: None,
            cleanup_evidence: None,
        }
    }
    /// Attach the lifecycle benchmark artifact used by this soak run.
    #[must_use]
    pub fn with_benchmark_artifact(mut self, benchmark_artifact: impl Into<String>) -> Self {
        self.benchmark_artifact = Some(benchmark_artifact.into());
        self
    }
    /// Attach cleanup evidence captured after the soak run.
    #[must_use]
    pub const fn with_cleanup_evidence(mut self, cleanup_evidence: SoakCleanupEvidence) -> Self {
        self.cleanup_evidence = Some(cleanup_evidence);
        self
    }
    /// Return the measured soak duration.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        Duration::from_secs(self.duration_seconds)
    }
    /// Return step evidence.
    #[must_use]
    pub fn steps(&self) -> &[SoakStepEvidence] {
        &self.steps
    }
    /// Return the referenced lifecycle benchmark artifact.
    #[must_use]
    pub fn benchmark_artifact(&self) -> Option<&str> {
        self.benchmark_artifact.as_deref()
    }
    /// Return final cleanup evidence.
    #[must_use]
    pub const fn cleanup_evidence(&self) -> Option<SoakCleanupEvidence> {
        self.cleanup_evidence
    }
    /// Return evidence for one step.
    #[must_use]
    pub fn step(&self, step: SoakStep) -> Option<&SoakStepEvidence> {
        self.steps.iter().find(|evidence| evidence.step() == step)
    }
    /// Validate this artifact as production 24-hour soak evidence.
    ///
    /// # Errors
    ///
    /// Returns [`SoakEvidenceError`] when duration is too short, a required
    /// step is missing, a required step was not attempted, or a step failed.
    pub fn validate_production(
        &self,
    ) -> std::result::Result<SoakEvidenceGateReport, SoakEvidenceError> {
        let required_seconds = Duration::from_hours(24).as_secs();
        if self.duration_seconds < required_seconds {
            return Err(SoakEvidenceError::DurationTooShort {
                required_seconds,
                actual_seconds: self.duration_seconds,
            });
        }
        for step in SoakStep::required_inspect_loop() {
            let evidence = self
                .step(step)
                .ok_or(SoakEvidenceError::MissingStep { step })?;
            if evidence.attempts() == 0 {
                return Err(SoakEvidenceError::StepNotAttempted { step });
            }
            if evidence.failures() > 0 {
                return Err(SoakEvidenceError::StepFailed {
                    step,
                    failures: evidence.failures(),
                });
            }
        }
        let benchmark_artifact = self
            .benchmark_artifact()
            .filter(|artifact| !artifact.is_empty())
            .ok_or(SoakEvidenceError::MissingBenchmarkArtifact)?;
        let cleanup_evidence = self
            .cleanup_evidence
            .ok_or(SoakEvidenceError::MissingCleanupEvidence)?;
        if !cleanup_evidence.is_clean() {
            return Err(SoakEvidenceError::CleanupLeaked {
                orphaned_vms: cleanup_evidence.orphaned_vms(),
                orphaned_snapshots: cleanup_evidence.orphaned_snapshots(),
                orphaned_logs: cleanup_evidence.orphaned_logs(),
                leaked_capacity_reservations: cleanup_evidence.leaked_capacity_reservations(),
            });
        }
        Ok(SoakEvidenceGateReport {
            covered_steps: SoakStep::required_inspect_loop(),
            benchmark_artifact: benchmark_artifact.to_owned(),
            cleanup_evidence,
        })
    }
}
/// Final cleanup evidence for a single-node soak run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SoakCleanupEvidence {
    orphaned_vms: u64,
    orphaned_snapshots: u64,
    orphaned_logs: u64,
    leaked_capacity_reservations: u64,
}
impl SoakCleanupEvidence {
    /// Construct cleanup evidence.
    #[must_use]
    pub const fn new(
        orphaned_vms: u64,
        orphaned_snapshots: u64,
        orphaned_logs: u64,
        leaked_capacity_reservations: u64,
    ) -> Self {
        Self {
            orphaned_vms,
            orphaned_snapshots,
            orphaned_logs,
            leaked_capacity_reservations,
        }
    }
    /// Construct clean evidence with no orphaned runtime state.
    #[must_use]
    pub const fn clean() -> Self {
        Self::new(0, 0, 0, 0)
    }
    /// Return orphaned VM count.
    #[must_use]
    pub const fn orphaned_vms(&self) -> u64 {
        self.orphaned_vms
    }
    /// Return orphaned snapshot count.
    #[must_use]
    pub const fn orphaned_snapshots(&self) -> u64 {
        self.orphaned_snapshots
    }
    /// Return orphaned log count.
    #[must_use]
    pub const fn orphaned_logs(&self) -> u64 {
        self.orphaned_logs
    }
    /// Return leaked capacity reservation count.
    #[must_use]
    pub const fn leaked_capacity_reservations(&self) -> u64 {
        self.leaked_capacity_reservations
    }
    /// Return whether no orphaned runtime state was observed.
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.orphaned_vms == 0
            && self.orphaned_snapshots == 0
            && self.orphaned_logs == 0
            && self.leaked_capacity_reservations == 0
    }
}
/// Soak evidence validation error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum SoakEvidenceError {
    /// The measured soak duration was shorter than required.
    #[error("soak duration too short: required {required_seconds}s, actual {actual_seconds}s")]
    DurationTooShort {
        /// Required duration in seconds.
        required_seconds: u64,
        /// Actual duration in seconds.
        actual_seconds: u64,
    },
    /// A required step is absent.
    #[error("soak evidence missing required step {step:?}")]
    MissingStep {
        /// Missing step.
        step: SoakStep,
    },
    /// A required step was present but never attempted.
    #[error("soak evidence step {step:?} was not attempted")]
    StepNotAttempted {
        /// Unattempted step.
        step: SoakStep,
    },
    /// A required step recorded failures.
    #[error("soak evidence step {step:?} recorded {failures} failures")]
    StepFailed {
        /// Failed step.
        step: SoakStep,
        /// Failure count.
        failures: u64,
    },
    /// A benchmark evidence artifact reference is absent.
    #[error("soak evidence missing benchmark artifact reference")]
    MissingBenchmarkArtifact,
    /// Cleanup evidence is absent.
    #[error("soak evidence missing cleanup evidence")]
    MissingCleanupEvidence,
    /// Cleanup evidence recorded orphaned runtime state.
    #[error(
        "soak cleanup leaked runtime state: orphaned_vms={orphaned_vms} orphaned_snapshots={orphaned_snapshots} orphaned_logs={orphaned_logs} leaked_capacity_reservations={leaked_capacity_reservations}"
    )]
    CleanupLeaked {
        /// Orphaned VM count.
        orphaned_vms: u64,
        /// Orphaned snapshot count.
        orphaned_snapshots: u64,
        /// Orphaned log count.
        orphaned_logs: u64,
        /// Leaked capacity reservation count.
        leaked_capacity_reservations: u64,
    },
}
/// Successful production soak evidence gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoakEvidenceGateReport {
    covered_steps: [SoakStep; 7],
    benchmark_artifact: String,
    cleanup_evidence: SoakCleanupEvidence,
}
impl SoakEvidenceGateReport {
    /// Return the required steps covered by the evidence.
    #[must_use]
    pub const fn covered_steps(&self) -> [SoakStep; 7] {
        self.covered_steps
    }
    /// Return the referenced benchmark artifact path.
    #[must_use]
    pub fn benchmark_artifact(&self) -> &str {
        &self.benchmark_artifact
    }
    /// Return validated cleanup evidence.
    #[must_use]
    pub const fn cleanup_evidence(&self) -> SoakCleanupEvidence {
        self.cleanup_evidence
    }
}
/// Durable soak evidence artifact.
pub struct SoakEvidenceArtifact;
impl SoakEvidenceArtifact {
    /// Write a soak evidence report as pretty JSON.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] when serialization or writing fails.
    pub fn write_json(path: impl AsRef<Path>, report: &SoakEvidenceReport) -> io::Result<()> {
        let bytes = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;
        fs::write(path, bytes)
    }
    /// Read a soak evidence report from JSON.
    ///
    /// # Errors
    ///
    /// Returns [`io::Error`] when reading or deserialization fails.
    pub fn read_json(path: impl AsRef<Path>) -> io::Result<SoakEvidenceReport> {
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
}
