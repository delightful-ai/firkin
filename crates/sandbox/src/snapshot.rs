use std::collections::BTreeMap;
use std::sync::Arc;

use time::OffsetDateTime;

use crate::backend::BoxBackend;
use crate::capability::CapabilityName;
use crate::error::Result;
use crate::ids::{SandboxId, SnapshotId, TemplateId};
use crate::sandbox::{Sandbox, unsupported};

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRef {
    id: SnapshotId,
    kind: SnapshotKind,
}

impl SnapshotRef {
    pub const fn new(id: SnapshotId, kind: SnapshotKind) -> Self {
        Self { id, kind }
    }

    pub const fn id(&self) -> &SnapshotId {
        &self.id
    }

    pub const fn kind(&self) -> SnapshotKind {
        self.kind
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub id: SnapshotId,
    pub kind: SnapshotKind,
    pub source_sandbox_id: Option<SandboxId>,
    pub source_template_id: Option<TemplateId>,
    pub created_at: OffsetDateTime,
    pub integrity: Option<SnapshotIntegrity>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotKind {
    Template,
    Continuation,
    Pause,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapshotOptions {
    pub name: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl SnapshotOptions {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            metadata: BTreeMap::new(),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotExport {
    pub id: SnapshotId,
    pub media_type: String,
    pub bytes: bytes::Bytes,
    pub integrity: SnapshotIntegrity,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotImport {
    pub id: Option<SnapshotId>,
    pub media_type: String,
    pub bytes: bytes::Bytes,
    pub integrity: SnapshotIntegrity,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotIntegrity {
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct PausedSandbox {
    pub sandbox_id: SandboxId,
    pub snapshot: SnapshotRef,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PauseOptions {
    pub preserve_identity: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResumeOptions {
    pub preserve_identity: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreOptions {
    pub preserve_identity: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapshotFilter {
    pub kind: Option<SnapshotKind>,
}

#[derive(Clone)]
pub struct SnapshotClient {
    backend: BoxBackend,
}

impl SnapshotClient {
    pub(crate) fn new(backend: BoxBackend) -> Self {
        Self { backend }
    }

    pub async fn capture(
        &self,
        sandbox: &SandboxId,
        options: SnapshotOptions,
    ) -> Result<SnapshotRef> {
        self.backend
            .snapshots()
            .capture_snapshot(sandbox, options)
            .await
    }

    pub async fn restore(&self, snapshot: SnapshotRef, options: RestoreOptions) -> Result<Sandbox> {
        let info = self
            .backend
            .sandboxes()
            .restore_sandbox(snapshot, options)
            .await?;
        Ok(Sandbox::new(info.id, self.backend.clone()))
    }

    pub async fn get(&self, id: &SnapshotId) -> Result<SnapshotInfo> {
        self.backend.snapshots().get_snapshot(id).await
    }

    pub async fn list(&self, filter: SnapshotFilter) -> Result<Vec<SnapshotInfo>> {
        self.backend.snapshots().list_snapshots(filter).await
    }

    pub async fn delete(&self, id: &SnapshotId) -> Result<()> {
        self.backend.snapshots().delete_snapshot(id).await
    }

    pub async fn export(&self, id: &SnapshotId) -> Result<SnapshotExport> {
        self.backend.snapshots().export_snapshot(id).await
    }

    pub async fn import(&self, import: SnapshotImport) -> Result<SnapshotRef> {
        self.backend.snapshots().import_snapshot(import).await
    }

    pub async fn resume(&self, paused: PausedSandbox, options: ResumeOptions) -> Result<Sandbox> {
        let Some(control) = self.backend.pause() else {
            return Err(unsupported(
                "resume paused sandbox",
                CapabilityName::PauseResume,
            ));
        };
        let info = control.resume_paused(paused, options).await?;
        Ok(Sandbox::new(info.id, self.backend.clone()))
    }
}

impl From<Arc<dyn crate::backend::SandboxBackend>> for SnapshotClient {
    fn from(backend: Arc<dyn crate::backend::SandboxBackend>) -> Self {
        Self::new(backend)
    }
}
