//! sandbox — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_e2b_contract::BackendError;
#[allow(unused_imports)]
use firkin_e2b_contract::PreparedTemplateArtifactIntegrity;
#[allow(unused_imports)]
use firkin_e2b_contract::{FollowupSnapshot, PreparedTemplate, SandboxRuntimeConfig, SnapshotRef};
#[allow(unused_imports)]
use firkin_e2b_wire::SnapshotInfo;
#[allow(unused_imports)]
use firkin_e2b_wire::{
    ConnectRequest, CreateSnapshotRequest, RefreshRequest, SandboxLogEntry, SandboxState,
    SandboxesWithMetrics, TimeoutRequest,
};
#[allow(unused_imports)]
use firkin_e2b_wire::{
    ConnectedSandbox, SandboxCreateRequest, SandboxInfo, SandboxLogs, SandboxMetric,
};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::BTreeMap;
/// Registry record for one E2B sandbox.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SandboxRecord {
    /// Original create request.
    pub create_request: SandboxCreateRequest,
    /// SDK create/connect response.
    pub connected: ConnectedSandbox,
    /// SDK info response.
    pub info: SandboxInfo,
    /// Captured log entries.
    pub logs: SandboxLogs,
    /// Latest metric sample, when available.
    pub metric: Option<SandboxMetric>,
}
/// Stored snapshot metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SnapshotRecord {
    /// Public snapshot info.
    pub info: SnapshotInfo,
    /// Source sandbox id.
    pub sandbox_id: String,
    /// Optional user-visible snapshot name.
    pub name: Option<String>,
    /// Runtime-local snapshot path or URI.
    #[serde(default)]
    pub location: Option<String>,
    /// Expected integrity for the runtime snapshot artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_integrity: Option<PreparedTemplateArtifactIntegrity>,
}
/// In-memory E2B control-plane registry.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LocalSandboxRegistry {
    #[allow(missing_docs)]
    pub sandboxes: BTreeMap<String, SandboxRecord>,
    #[allow(missing_docs)]
    pub snapshots: BTreeMap<String, SnapshotRecord>,
}
impl LocalSandboxRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Insert a newly started sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::AlreadyExists`] when `runtime.sandbox_id`
    /// already exists.
    pub fn create(
        &mut self,
        request: SandboxCreateRequest,
        runtime: SandboxRuntimeConfig,
    ) -> Result<ConnectedSandbox, BackendError> {
        if self.sandboxes.contains_key(&runtime.sandbox_id) {
            return Err(BackendError::AlreadyExists(runtime.sandbox_id));
        }
        let connected = ConnectedSandbox {
            sandbox_id: runtime.sandbox_id.clone(),
            envd_version: runtime.envd_version.clone(),
            envd_access_token: runtime.envd_access_token,
            traffic_access_token: runtime.traffic_access_token,
            domain: Some(runtime.domain),
        };
        let info = SandboxInfo {
            sandbox_id: runtime.sandbox_id.clone(),
            template_id: request.template_id.clone(),
            alias: None,
            metadata: request.metadata.clone(),
            started_at: runtime.started_at,
            end_at: runtime.end_at,
            state: SandboxState::Running,
            cpu_count: runtime.cpu_count,
            memory_mb: runtime.memory_mb,
            envd_version: runtime.envd_version,
            allow_internet_access: request.allow_internet_access,
            network: request.network.clone(),
            volume_mounts: request.volume_mounts.clone(),
        };
        self.sandboxes.insert(
            runtime.sandbox_id,
            SandboxRecord {
                create_request: request,
                connected: connected.clone(),
                info,
                logs: SandboxLogs::default(),
                metric: None,
            },
        );
        Ok(connected)
    }
    /// Return sandbox info.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn get(&self, sandbox_id: &str) -> Result<&SandboxInfo, BackendError> {
        Ok(&self.record(sandbox_id)?.info)
    }
    /// Return all registered sandbox infos.
    #[must_use]
    pub fn list(&self) -> Vec<SandboxInfo> {
        self.sandboxes
            .values()
            .map(|record| record.info.clone())
            .collect()
    }
    /// Delete a sandbox, returning whether it existed.
    pub fn delete(&mut self, sandbox_id: &str) -> bool {
        self.sandboxes.remove(sandbox_id).is_some()
    }
    /// Pause a running sandbox, returning whether the state changed.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn pause(&mut self, sandbox_id: &str) -> Result<bool, BackendError> {
        let record = self.record_mut(sandbox_id)?;
        if record.info.state == SandboxState::Paused {
            return Ok(false);
        }
        record.info.state = SandboxState::Paused;
        Ok(true)
    }
    /// Connect or resume a sandbox, returning the SDK connect response.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn connect(
        &mut self,
        sandbox_id: &str,
        _request: ConnectRequest,
    ) -> Result<ConnectedSandbox, BackendError> {
        let record = self.record_mut(sandbox_id)?;
        record.info.state = SandboxState::Running;
        Ok(record.connected.clone())
    }
    /// Set a new timeout deadline.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn set_timeout(
        &mut self,
        sandbox_id: &str,
        _request: TimeoutRequest,
        end_at: impl Into<String>,
    ) -> Result<(), BackendError> {
        self.record_mut(sandbox_id)?.info.end_at = end_at.into();
        Ok(())
    }
    /// Refresh a sandbox timeout deadline.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn refresh(
        &mut self,
        sandbox_id: &str,
        _request: RefreshRequest,
        end_at: impl Into<String>,
    ) -> Result<(), BackendError> {
        self.record_mut(sandbox_id)?.info.end_at = end_at.into();
        Ok(())
    }
    /// Append a log entry for a sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn push_log(&mut self, sandbox_id: &str, log: SandboxLogEntry) -> Result<(), BackendError> {
        self.record_mut(sandbox_id)?.logs.logs.push(log);
        Ok(())
    }
    /// Return sandbox logs.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn logs(&self, sandbox_id: &str) -> Result<SandboxLogs, BackendError> {
        Ok(self.record(sandbox_id)?.logs.clone())
    }
    /// Store the latest metric sample for a sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn set_metric(
        &mut self,
        sandbox_id: &str,
        metric: SandboxMetric,
    ) -> Result<(), BackendError> {
        self.record_mut(sandbox_id)?.metric = Some(metric);
        Ok(())
    }
    /// Return sandbox-local metrics.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn metrics(&self, sandbox_id: &str) -> Result<Vec<SandboxMetric>, BackendError> {
        Ok(self
            .record(sandbox_id)?
            .metric
            .clone()
            .into_iter()
            .collect())
    }
    /// Return latest metric samples for many sandboxes.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when any requested sandbox is not
    /// registered.
    pub fn metrics_many(
        &self,
        sandbox_ids: &[String],
    ) -> Result<SandboxesWithMetrics, BackendError> {
        let mut sandboxes = BTreeMap::new();
        for sandbox_id in sandbox_ids {
            let record = self.record(sandbox_id)?;
            if let Some(metric) = &record.metric {
                sandboxes.insert(sandbox_id.clone(), metric.clone());
            }
        }
        Ok(SandboxesWithMetrics { sandboxes })
    }
    /// Return running sandbox ids whose deadline is at or before `now`.
    #[must_use]
    pub fn due_running_sandboxes(&self, now: &str) -> Vec<String> {
        self.sandboxes
            .iter()
            .filter(|(_, record)| record.info.state == SandboxState::Running)
            .filter(|(_, record)| record.info.end_at.as_str() <= now)
            .map(|(sandbox_id, _)| sandbox_id.clone())
            .collect()
    }
    /// Return whether timeout expiration should pause this sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn auto_pause_enabled(&self, sandbox_id: &str) -> Result<bool, BackendError> {
        Ok(self
            .record(sandbox_id)?
            .create_request
            .auto_pause
            .unwrap_or(false))
    }
    /// Create snapshot metadata for a sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not registered.
    pub fn create_snapshot(
        &mut self,
        sandbox_id: &str,
        request: CreateSnapshotRequest,
        snapshot: SnapshotRef,
    ) -> Result<SnapshotInfo, BackendError> {
        self.record(sandbox_id)?;
        let snapshot_id = snapshot.snapshot_id;
        if self.snapshots.contains_key(&snapshot_id) {
            return Err(BackendError::AlreadyExists(snapshot_id));
        }
        let info = SnapshotInfo {
            snapshot_id: snapshot_id.clone(),
        };
        self.snapshots.insert(
            snapshot_id.clone(),
            SnapshotRecord {
                info: info.clone(),
                sandbox_id: sandbox_id.to_owned(),
                name: request.name,
                location: snapshot.location,
                artifact_integrity: snapshot.artifact_integrity,
            },
        );
        Ok(info)
    }
    /// List snapshots, optionally filtering by source sandbox.
    #[must_use]
    pub fn list_snapshots(&self, sandbox_id: Option<&str>) -> Vec<SnapshotInfo> {
        self.snapshots
            .values()
            .filter(|record| sandbox_id.is_none_or(|id| id == record.sandbox_id))
            .map(|record| record.info.clone())
            .collect()
    }
    /// Delete snapshot metadata, returning whether it existed.
    pub fn delete_snapshot(&mut self, snapshot_id: &str) -> bool {
        self.snapshots.remove(snapshot_id).is_some()
    }
    /// Return a runtime snapshot as a prepared template source.
    #[must_use]
    pub fn snapshot_prepared_template(&self, snapshot_id: &str) -> Option<PreparedTemplate> {
        let record = self.snapshots.get(snapshot_id)?;
        Some(PreparedTemplate {
            template_id: record.info.snapshot_id.clone(),
            build_id: record.info.snapshot_id.clone(),
            artifact: record.location.clone().unwrap_or_default(),
            has_envd: true,
            artifact_integrity: record.artifact_integrity.clone(),
        })
    }
    /// Return a runtime snapshot as a follow-up source.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the snapshot is absent, or
    /// [`BackendError::Runtime`] when the snapshot has no runtime location.
    pub fn followup_snapshot(&self, snapshot_id: &str) -> Result<FollowupSnapshot, BackendError> {
        let record = self
            .snapshots
            .get(snapshot_id)
            .ok_or_else(|| BackendError::NotFound(snapshot_id.to_owned()))?;
        let location = record
            .location
            .clone()
            .ok_or_else(|| {
                BackendError::Runtime(
                    format!(
                        "snapshot `{snapshot_id}` cannot start a follow-up sandbox without a runtime location"
                    ),
                )
            })?;
        Ok(FollowupSnapshot {
            snapshot_id: record.info.snapshot_id.clone(),
            location,
            artifact_integrity: record.artifact_integrity.clone(),
        })
    }
    pub(crate) fn record(&self, sandbox_id: &str) -> Result<&SandboxRecord, BackendError> {
        self.sandboxes
            .get(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))
    }
    pub(crate) fn record_mut(
        &mut self,
        sandbox_id: &str,
    ) -> Result<&mut SandboxRecord, BackendError> {
        self.sandboxes
            .get_mut(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))
    }
}
