//! pod — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_e2b_contract::{BackendError, RuntimePod};
#[allow(unused_imports)]
use firkin_e2b_wire::{PodContainerInfo, PodState};
#[allow(unused_imports)]
use firkin_e2b_wire::{PodCreateRequest, PodInfo};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::collections::BTreeSet;
/// Registry record for one product pod.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodRecord {
    /// Original create request.
    pub create_request: PodCreateRequest,
    /// Public pod info.
    pub info: PodInfo,
}
/// In-memory product pod registry.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalPodRegistry {
    pub(crate) pods: BTreeMap<String, PodRecord>,
}
impl LocalPodRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Insert a newly started pod.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::AlreadyExists`] for duplicate pod IDs.
    pub fn create(
        &mut self,
        request: PodCreateRequest,
        runtime: RuntimePod,
    ) -> Result<PodInfo, BackendError> {
        if self.pods.contains_key(&runtime.config.pod_id) {
            return Err(BackendError::AlreadyExists(runtime.config.pod_id));
        }
        let mut seen = BTreeSet::new();
        for container in &runtime.containers {
            if !seen.insert(container.name.as_str()) {
                return Err(BackendError::AlreadyExists(container.name.clone()));
            }
        }
        let info = PodInfo {
            pod_id: runtime.config.pod_id.clone(),
            metadata: request.metadata.clone(),
            started_at: runtime.config.started_at,
            end_at: runtime.config.end_at,
            state: PodState::Running,
            empty_dirs: request.empty_dirs.clone(),
            containers: runtime.containers,
        };
        self.pods.insert(
            runtime.config.pod_id,
            PodRecord {
                create_request: request,
                info: info.clone(),
            },
        );
        Ok(info)
    }
    /// Return pod info.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the pod is absent.
    pub fn get(&self, pod_id: &str) -> Result<&PodInfo, BackendError> {
        Ok(&self.record(pod_id)?.info)
    }
    /// Return all pods.
    #[must_use]
    pub fn list(&self) -> Vec<PodInfo> {
        self.pods
            .values()
            .map(|record| record.info.clone())
            .collect()
    }
    /// Delete a pod, returning whether it existed.
    pub fn delete(&mut self, pod_id: &str) -> bool {
        self.pods.remove(pod_id).is_some()
    }
    /// Register a newly started container in an existing pod.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the pod is absent and
    /// [`BackendError::AlreadyExists`] when the container already exists.
    pub fn add_container(
        &mut self,
        pod_id: &str,
        container: PodContainerInfo,
    ) -> Result<PodContainerInfo, BackendError> {
        let record = self.record_mut(pod_id)?;
        if record
            .info
            .containers
            .iter()
            .any(|existing| existing.name == container.name)
        {
            return Err(BackendError::AlreadyExists(container.name));
        }
        record.info.containers.push(container.clone());
        Ok(container)
    }
    /// Remove a container from a pod.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the pod or container is absent.
    pub fn delete_container(
        &mut self,
        pod_id: &str,
        container_name: &str,
    ) -> Result<(), BackendError> {
        let record = self.record_mut(pod_id)?;
        let original_len = record.info.containers.len();
        record
            .info
            .containers
            .retain(|container| container.name != container_name);
        if record.info.containers.len() == original_len {
            return Err(BackendError::NotFound(container_name.to_owned()));
        }
        Ok(())
    }
    pub(crate) fn record(&self, pod_id: &str) -> Result<&PodRecord, BackendError> {
        self.pods
            .get(pod_id)
            .ok_or_else(|| BackendError::NotFound(pod_id.to_owned()))
    }
    pub(crate) fn record_mut(&mut self, pod_id: &str) -> Result<&mut PodRecord, BackendError> {
        self.pods
            .get_mut(pod_id)
            .ok_or_else(|| BackendError::NotFound(pod_id.to_owned()))
    }
}
