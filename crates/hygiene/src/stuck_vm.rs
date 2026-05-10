//! stuck vm — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use std::time::Duration;
/// Host observation for one VM that may be stuck.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StuckVmObservation {
    pub(crate) id: String,
    heartbeat_age: Duration,
    runtime_pid: Option<u32>,
}
impl StuckVmObservation {
    /// Construct a stuck-VM observation.
    #[must_use]
    pub fn new(id: impl Into<String>, heartbeat_age: Duration) -> Self {
        Self {
            id: id.into(),
            heartbeat_age,
            runtime_pid: None,
        }
    }
    /// Construct a stuck-VM observation with its owning host runtime process.
    #[must_use]
    pub fn with_runtime_pid(
        id: impl Into<String>,
        heartbeat_age: Duration,
        runtime_pid: u32,
    ) -> Self {
        Self {
            id: id.into(),
            heartbeat_age,
            runtime_pid: Some(runtime_pid),
        }
    }
    /// Return the VM identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Return how long ago the VM last reported progress.
    #[must_use]
    pub const fn heartbeat_age(&self) -> Duration {
        self.heartbeat_age
    }
    /// Return the owning host runtime process id, when discovered.
    #[must_use]
    pub const fn runtime_pid(&self) -> Option<u32> {
        self.runtime_pid
    }
}
/// Planned stuck-VM cleanup action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StuckVmCleanupDecision {
    /// VM is still fresh enough to preserve.
    Preserve,
    /// VM exceeded the heartbeat timeout and should be cleaned up.
    Cleanup,
    /// VM record is ambiguous and should be preserved for operator inspection.
    Quarantine,
}
/// Decision for one stuck-VM observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StuckVmCleanupPlanEntry {
    observation: StuckVmObservation,
    #[allow(missing_docs)]
    pub decision: StuckVmCleanupDecision,
}
impl StuckVmCleanupPlanEntry {
    /// Construct a stuck-VM cleanup plan entry.
    #[must_use]
    pub const fn new(observation: StuckVmObservation, decision: StuckVmCleanupDecision) -> Self {
        Self {
            observation,
            decision,
        }
    }
    /// Return the source observation.
    #[must_use]
    pub const fn observation(&self) -> &StuckVmObservation {
        &self.observation
    }
    /// Return the planned cleanup decision.
    #[must_use]
    pub const fn decision(&self) -> StuckVmCleanupDecision {
        self.decision
    }
}
/// Stuck-VM cleanup plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StuckVmCleanupPlan {
    pub(crate) decisions: Vec<StuckVmCleanupPlanEntry>,
}
impl StuckVmCleanupPlan {
    /// Build a cleanup plan from VM heartbeat observations.
    #[must_use]
    pub fn from_observations(
        observations: impl IntoIterator<Item = StuckVmObservation>,
        heartbeat_timeout: Duration,
    ) -> Self {
        let decisions = observations
            .into_iter()
            .map(|observation| {
                let decision = if observation.id().is_empty() {
                    StuckVmCleanupDecision::Quarantine
                } else if observation.heartbeat_age() >= heartbeat_timeout {
                    StuckVmCleanupDecision::Cleanup
                } else {
                    StuckVmCleanupDecision::Preserve
                };
                StuckVmCleanupPlanEntry::new(observation, decision)
            })
            .collect();
        Self { decisions }
    }
    /// Return planned decisions.
    #[must_use]
    pub fn decisions(&self) -> &[StuckVmCleanupPlanEntry] {
        &self.decisions
    }
    /// Return the decision for one VM id.
    #[must_use]
    pub fn decision_for(&self, id: &str) -> Option<StuckVmCleanupDecision> {
        self.decisions
            .iter()
            .find(|entry| entry.observation().id() == id)
            .map(StuckVmCleanupPlanEntry::decision)
    }
    /// Return VM ids that should be cleaned up.
    #[must_use]
    pub fn cleanup_ids(&self) -> Vec<&str> {
        self.decisions
            .iter()
            .filter(|entry| entry.decision() == StuckVmCleanupDecision::Cleanup)
            .map(|entry| entry.observation().id())
            .collect()
    }
    /// Return VM ids that should be quarantined.
    #[must_use]
    pub fn quarantine_ids(&self) -> Vec<&str> {
        self.decisions
            .iter()
            .filter(|entry| entry.decision() == StuckVmCleanupDecision::Quarantine)
            .map(|entry| entry.observation().id())
            .collect()
    }
}
