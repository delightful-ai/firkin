//! replenish — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::budget::ResourceBudget;
#[allow(unused_imports)]
use crate::budget_fits;
#[allow(unused_imports)]
use crate::capacity::CapacityLedger;
#[allow(unused_imports)]
use crate::warm_pool::{WarmPoolKey, WarmPoolLedger};
#[allow(unused_imports)]
use firkin_artifacts::SnapshotArtifactManifest;
#[allow(unused_imports)]
use firkin_types::Size;
/// Desired warm-pool entry for background replenishment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmPoolReplenishmentTarget {
    pub(crate) key: WarmPoolKey,
    manifest: SnapshotArtifactManifest,
    pub(crate) budget: ResourceBudget,
}
impl WarmPoolReplenishmentTarget {
    /// Construct a desired warm-pool replenishment target.
    #[must_use]
    pub const fn new(
        key: WarmPoolKey,
        manifest: SnapshotArtifactManifest,
        budget: ResourceBudget,
    ) -> Self {
        Self {
            key,
            manifest,
            budget,
        }
    }
    /// Return the warm-pool key.
    #[must_use]
    pub const fn key(&self) -> &WarmPoolKey {
        &self.key
    }
    /// Return the snapshot manifest to restore for this warm entry.
    #[must_use]
    pub const fn manifest(&self) -> &SnapshotArtifactManifest {
        &self.manifest
    }
    /// Return the resources required by this warm entry.
    #[must_use]
    pub const fn budget(&self) -> ResourceBudget {
        self.budget
    }
}
/// Reason a warm-pool replenishment target was not scheduled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarmPoolReplenishmentSkipReason {
    /// A warm entry already exists for the requested key.
    AlreadyWarm,
    /// Scheduling this target would exceed currently available capacity.
    InsufficientCapacity {
        /// Requested warm-entry resources.
        requested: ResourceBudget,
        /// Capacity remaining after earlier planned replenishment.
        available: ResourceBudget,
    },
}
/// Warm-pool replenishment target skipped by the planner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmPoolReplenishmentSkip {
    pub(crate) key: WarmPoolKey,
    #[allow(missing_docs)]
    pub reason: WarmPoolReplenishmentSkipReason,
}
impl WarmPoolReplenishmentSkip {
    /// Construct a skipped replenishment record.
    #[must_use]
    pub const fn new(key: WarmPoolKey, reason: WarmPoolReplenishmentSkipReason) -> Self {
        Self { key, reason }
    }
    /// Return the skipped warm-pool key.
    #[must_use]
    pub const fn key(&self) -> &WarmPoolKey {
        &self.key
    }
    /// Return why the target was skipped.
    #[must_use]
    pub const fn reason(&self) -> WarmPoolReplenishmentSkipReason {
        self.reason
    }
}
/// Immutable background warm-pool replenishment plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarmPoolReplenishmentPlan {
    maintain: Vec<WarmPoolReplenishmentTarget>,
    skipped: Vec<WarmPoolReplenishmentSkip>,
}
impl WarmPoolReplenishmentPlan {
    /// Plan which desired warm entries should be restored.
    #[must_use]
    pub fn from_targets(
        targets: &[WarmPoolReplenishmentTarget],
        pool: &WarmPoolLedger,
        capacity: CapacityLedger,
    ) -> Self {
        let mut maintain = Vec::new();
        let mut skipped = Vec::new();
        let mut planned = ResourceBudget::new(0, Size::bytes(0), Size::bytes(0));
        for target in targets {
            if pool.contains(target.key()) {
                skipped.push(WarmPoolReplenishmentSkip::new(
                    target.key().clone(),
                    WarmPoolReplenishmentSkipReason::AlreadyWarm,
                ));
                continue;
            }
            let available = capacity.available() - planned;
            if !budget_fits(target.budget(), available) {
                skipped.push(WarmPoolReplenishmentSkip::new(
                    target.key().clone(),
                    WarmPoolReplenishmentSkipReason::InsufficientCapacity {
                        requested: target.budget(),
                        available,
                    },
                ));
                continue;
            }
            planned = planned + target.budget();
            maintain.push(target.clone());
        }
        Self { maintain, skipped }
    }
    /// Return warm entries that should be restored.
    #[must_use]
    pub fn maintain(&self) -> &[WarmPoolReplenishmentTarget] {
        &self.maintain
    }
    /// Return desired warm entries that were not scheduled.
    #[must_use]
    pub fn skipped(&self) -> &[WarmPoolReplenishmentSkip] {
        &self.skipped
    }
}
