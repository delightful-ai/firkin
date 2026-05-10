//! warm pool — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::budget::{CapacityError, ResourceBudget};
#[allow(unused_imports)]
use crate::capacity::CapacityLedger;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::collections::VecDeque;
/// Warm-pool identity for pre-restored sandboxes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WarmPoolKey {
    #[allow(missing_docs)]
    pub repo: String,
    template: String,
    runtime_profile: String,
}
impl WarmPoolKey {
    /// Construct a warm-pool key.
    #[must_use]
    pub fn new(
        repo: impl Into<String>,
        template: impl Into<String>,
        runtime_profile: impl Into<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            template: template.into(),
            runtime_profile: runtime_profile.into(),
        }
    }
    /// Return the repository key.
    #[must_use]
    pub fn repo(&self) -> &str {
        &self.repo
    }
    /// Return the template key.
    #[must_use]
    pub fn template(&self) -> &str {
        &self.template
    }
    /// Return the runtime profile key.
    #[must_use]
    pub fn runtime_profile(&self) -> &str {
        &self.runtime_profile
    }
}
/// Pre-restored warm-pool entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmPoolEntry {
    pub(crate) key: WarmPoolKey,
    snapshot_id: String,
    pub(crate) budget: ResourceBudget,
}
impl WarmPoolEntry {
    /// Construct a warm-pool entry.
    #[must_use]
    pub fn new(key: WarmPoolKey, snapshot_id: impl Into<String>, budget: ResourceBudget) -> Self {
        Self {
            key,
            snapshot_id: snapshot_id.into(),
            budget,
        }
    }
    /// Return the warm-pool key.
    #[must_use]
    pub const fn key(&self) -> &WarmPoolKey {
        &self.key
    }
    /// Return the source snapshot id.
    #[must_use]
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }
    /// Return the reserved resources.
    #[must_use]
    pub const fn budget(&self) -> ResourceBudget {
        self.budget
    }
}
/// In-memory warm-pool ledger.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarmPoolLedger {
    pub(crate) entries: BTreeMap<WarmPoolKey, VecDeque<WarmPoolEntry>>,
}
impl WarmPoolLedger {
    /// Add a warm-pool entry and reserve its resources.
    ///
    /// # Errors
    ///
    /// Returns [`CapacityError`] when the new entry cannot be reserved.
    pub fn maintain(
        &mut self,
        entry: WarmPoolEntry,
        capacity: &mut CapacityLedger,
    ) -> std::result::Result<(), CapacityError> {
        capacity.reserve_warm_pool(entry.budget())?;
        self.entries
            .entry(entry.key().clone())
            .or_default()
            .push_back(entry);
        Ok(())
    }
    /// Checkout a warm-pool entry, promoting its reservation to active use.
    ///
    /// # Errors
    ///
    /// Returns [`CapacityError`] when the reserved entry cannot be promoted.
    pub fn checkout(
        &mut self,
        key: &WarmPoolKey,
        capacity: &mut CapacityLedger,
    ) -> std::result::Result<Option<WarmPoolEntry>, CapacityError> {
        let Some(entries) = self.entries.get_mut(key) else {
            return Ok(None);
        };
        let Some(entry) = entries.pop_front() else {
            self.entries.remove(key);
            return Ok(None);
        };
        if entries.is_empty() {
            self.entries.remove(key);
        }
        capacity.promote_warm_pool_to_active(entry.budget())?;
        Ok(Some(entry))
    }
    /// Expire one warm-pool entry and release its reservation.
    pub fn expire(
        &mut self,
        key: &WarmPoolKey,
        capacity: &mut CapacityLedger,
    ) -> Option<WarmPoolEntry> {
        let entries = self.entries.get_mut(key)?;
        let entry = entries.pop_front()?;
        if entries.is_empty() {
            self.entries.remove(key);
        }
        capacity.release_warm_pool(entry.budget());
        Some(entry)
    }
    /// Return whether a warm-pool entry exists for `key`.
    #[must_use]
    pub fn contains(&self, key: &WarmPoolKey) -> bool {
        self.entries
            .get(key)
            .is_some_and(|entries| !entries.is_empty())
    }
    /// Return retained warm-entry count for `key`.
    #[must_use]
    pub fn count(&self, key: &WarmPoolKey) -> usize {
        self.entries.get(key).map_or(0, VecDeque::len)
    }
    /// Evict warm-pool entries until `request` fits available capacity.
    ///
    /// Entries are evicted in key order. This is intentionally deterministic;
    /// richer LRU/priority policy belongs above this substrate model.
    pub fn evict_until_available(
        &mut self,
        request: ResourceBudget,
        capacity: &mut CapacityLedger,
    ) -> Vec<WarmPoolEntry> {
        let mut evicted = Vec::new();
        while request.cpus > capacity.available().cpus
            || request.memory > capacity.available().memory
            || request.disk > capacity.available().disk
        {
            let Some(key) = self.entries.keys().next().cloned() else {
                break;
            };
            if let Some(entry) = self.expire(&key, capacity) {
                evicted.push(entry);
            }
        }
        evicted
    }
}
