//! reconciliation — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::restart::{RestartResourceKind, RestartStateRecord};
/// Restart reconciliation action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciliationDecision {
    /// Reattach or preserve the resource as live state.
    Recover,
    /// Remove the resource as disposable stale state.
    Cleanup,
    /// Preserve the resource outside normal operation for operator inspection.
    Quarantine,
}
/// Decision for one restart state record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciliationPlanEntry {
    record: RestartStateRecord,
    action: ReconciliationDecision,
}
impl ReconciliationPlanEntry {
    /// Construct a reconciliation plan entry.
    #[must_use]
    pub const fn new(record: RestartStateRecord, action: ReconciliationDecision) -> Self {
        Self { record, action }
    }
    /// Return the source record.
    #[must_use]
    pub const fn record(&self) -> &RestartStateRecord {
        &self.record
    }
    /// Return the chosen action.
    #[must_use]
    pub const fn action(&self) -> ReconciliationDecision {
        self.action
    }
}
/// Restart reconciliation plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconciliationPlan {
    pub(crate) decisions: Vec<ReconciliationPlanEntry>,
}
impl ReconciliationPlan {
    /// Build a plan from discovered restart state records.
    #[must_use]
    pub fn from_records(records: impl IntoIterator<Item = RestartStateRecord>) -> Self {
        let decisions = records
            .into_iter()
            .map(|record| {
                let action = if record.id().is_empty() {
                    ReconciliationDecision::Quarantine
                } else {
                    match record.kind() {
                        RestartResourceKind::ActiveVm | RestartResourceKind::SnapshotArtifact => {
                            ReconciliationDecision::Recover
                        }
                        RestartResourceKind::LogStream
                        | RestartResourceKind::StaleRuntimeProcess => {
                            ReconciliationDecision::Cleanup
                        }
                    }
                };
                ReconciliationPlanEntry::new(record, action)
            })
            .collect();
        Self { decisions }
    }
    /// Return all planned decisions.
    #[must_use]
    pub fn decisions(&self) -> &[ReconciliationPlanEntry] {
        &self.decisions
    }
    /// Return the decision for a record id.
    #[must_use]
    pub fn decision_for(&self, id: &str) -> Option<ReconciliationDecision> {
        self.decisions
            .iter()
            .find(|entry| entry.record.id() == id)
            .map(ReconciliationPlanEntry::action)
    }
}
