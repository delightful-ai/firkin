//! host scan — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::reconciliation::ReconciliationPlan;
#[allow(unused_imports)]
use crate::restart::{RestartResourceKind, RestartStateRecord};
#[allow(unused_imports)]
use crate::stuck_vm::{StuckVmCleanupPlan, StuckVmObservation};
#[allow(unused_imports)]
use std::time::Duration;
/// Host runtime state discovered after controller restart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostRuntimeScan {
    restart_records: Vec<RestartStateRecord>,
    stuck_vm_observations: Vec<StuckVmObservation>,
}
impl HostRuntimeScan {
    /// Construct an empty host runtime scan.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            restart_records: Vec::new(),
            stuck_vm_observations: Vec::new(),
        }
    }
    /// Add an active VM discovered on the host.
    #[must_use]
    pub fn active_vm(mut self, id: impl Into<String>, heartbeat_age: Duration) -> Self {
        let id = id.into();
        self.restart_records.push(RestartStateRecord::new(
            id.clone(),
            RestartResourceKind::ActiveVm,
        ));
        self.stuck_vm_observations
            .push(StuckVmObservation::new(id, heartbeat_age));
        self
    }
    /// Add an active VM with the owning host runtime process discovered on the host.
    #[must_use]
    pub fn active_vm_with_runtime_pid(
        mut self,
        id: impl Into<String>,
        heartbeat_age: Duration,
        runtime_pid: u32,
    ) -> Self {
        let id = id.into();
        self.restart_records.push(RestartStateRecord::new(
            id.clone(),
            RestartResourceKind::ActiveVm,
        ));
        self.stuck_vm_observations
            .push(StuckVmObservation::with_runtime_pid(
                id,
                heartbeat_age,
                runtime_pid,
            ));
        self
    }
    /// Add a durable snapshot artifact discovered on the host.
    #[must_use]
    pub fn snapshot_artifact(mut self, id: impl Into<String>) -> Self {
        self.restart_records.push(RestartStateRecord::new(
            id,
            RestartResourceKind::SnapshotArtifact,
        ));
        self
    }
    /// Add a log stream or log artifact discovered on the host.
    #[must_use]
    pub fn log_stream(mut self, id: impl Into<String>) -> Self {
        self.restart_records
            .push(RestartStateRecord::new(id, RestartResourceKind::LogStream));
        self
    }
    /// Add a stale runtime process discovered on the host.
    #[must_use]
    pub fn stale_runtime_process(mut self, id: impl Into<String>) -> Self {
        self.restart_records.push(RestartStateRecord::new(
            id,
            RestartResourceKind::StaleRuntimeProcess,
        ));
        self
    }
    /// Return restart records discovered by this scan.
    #[must_use]
    pub fn restart_records(&self) -> &[RestartStateRecord] {
        &self.restart_records
    }
    /// Return VM heartbeat observations discovered by this scan.
    #[must_use]
    pub fn stuck_vm_observations(&self) -> &[StuckVmObservation] {
        &self.stuck_vm_observations
    }
    /// Build a restart reconciliation plan from discovered host state.
    #[must_use]
    pub fn reconciliation_plan(&self) -> ReconciliationPlan {
        ReconciliationPlan::from_records(self.restart_records.clone())
    }
    /// Build a stuck-VM cleanup plan from discovered VM heartbeat state.
    #[must_use]
    pub fn stuck_vm_cleanup_plan(&self, heartbeat_timeout: Duration) -> StuckVmCleanupPlan {
        StuckVmCleanupPlan::from_observations(self.stuck_vm_observations.clone(), heartbeat_timeout)
    }
}
