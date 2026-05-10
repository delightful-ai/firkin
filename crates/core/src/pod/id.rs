//! id — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_types::{ContainerId, InvalidContainerId};
#[cfg(test)]
#[allow(unused_imports)]
use std::io;
/// Validated pod identity used as a guest path segment under `/run/firkin/pods`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PodId(ContainerId);
impl PodId {
    /// Construct a pod ID with the same path-safe validation as container IDs.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when the ID is empty, too long, or
    /// contains characters outside `[a-zA-Z0-9_.-]`.
    pub fn new(id: impl Into<String>) -> std::result::Result<Self, InvalidContainerId> {
        ContainerId::new(id).map(Self)
    }
    /// Return the pod ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl std::fmt::Display for PodId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl IntoPodId for PodId {
    fn into_pod_id(self) -> std::result::Result<PodId, InvalidContainerId> {
        Ok(self)
    }
}
/// Convert supported inputs into a [`PodId`].
pub trait IntoPodId {
    /// Convert into a validated pod ID.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when string inputs fail validation.
    fn into_pod_id(self) -> std::result::Result<PodId, InvalidContainerId>;
}
impl IntoPodId for &str {
    fn into_pod_id(self) -> std::result::Result<PodId, InvalidContainerId> {
        PodId::new(self)
    }
}
impl IntoPodId for String {
    fn into_pod_id(self) -> std::result::Result<PodId, InvalidContainerId> {
        PodId::new(self)
    }
}
