use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;

use crate::backend::{BoxBackend, SandboxControl};
use crate::error::{Result, UnsupportedCapability};
use crate::ids::{SandboxId, SnapshotId, TemplateId};
use crate::process::{Command, CommandOutput, ProcessClient, Shell, ShellOpts, ShellPool};
use crate::snapshot::{PauseOptions, PausedSandbox, SnapshotRef};

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxSpec {
    Template {
        template_id: TemplateId,
        resources: Option<SandboxResources>,
        env: SandboxEnvironment,
        deadline: Option<SandboxDeadline>,
        metadata: SandboxMetadata,
    },
    Snapshot {
        snapshot: SnapshotRef,
        resources: Option<SandboxResources>,
        env: SandboxEnvironment,
        deadline: Option<SandboxDeadline>,
        metadata: SandboxMetadata,
    },
}

impl SandboxSpec {
    pub fn from_template(template: &crate::template::PreparedTemplate) -> Self {
        Self::Template {
            template_id: template.id().clone(),
            resources: None,
            env: SandboxEnvironment::default(),
            deadline: None,
            metadata: SandboxMetadata::default(),
        }
    }

    pub fn from_snapshot(snapshot: SnapshotRef) -> Self {
        Self::Snapshot {
            snapshot,
            resources: None,
            env: SandboxEnvironment::default(),
            deadline: None,
            metadata: SandboxMetadata::default(),
        }
    }

    pub fn resources(mut self, resources: SandboxResources) -> Self {
        match &mut self {
            Self::Template {
                resources: slot, ..
            }
            | Self::Snapshot {
                resources: slot, ..
            } => {
                *slot = Some(resources);
            }
        }
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        match &mut self {
            Self::Template { env, .. } | Self::Snapshot { env, .. } => {
                env.insert(key, value);
            }
        }
        self
    }

    pub fn timeout(self, timeout: Duration) -> Self {
        self.deadline(SandboxDeadline::timeout(timeout))
    }

    pub fn deadline(mut self, deadline: SandboxDeadline) -> Self {
        match &mut self {
            Self::Template { deadline: slot, .. } | Self::Snapshot { deadline: slot, .. } => {
                *slot = Some(deadline);
            }
        }
        self
    }

    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        match &mut self {
            Self::Template { metadata, .. } | Self::Snapshot { metadata, .. } => {
                metadata.insert(key, value);
            }
        }
        self
    }
}

#[derive(Clone)]
pub struct Sandbox {
    id: SandboxId,
    backend: BoxBackend,
}

impl Sandbox {
    pub(crate) fn new(id: SandboxId, backend: BoxBackend) -> Self {
        Self { id, backend }
    }

    pub const fn id(&self) -> &SandboxId {
        &self.id
    }

    pub async fn info(&self) -> Result<SandboxInfo> {
        self.backend.sandboxes().inspect_sandbox(&self.id).await
    }

    pub async fn stop(&self) -> Result<()> {
        self.backend
            .sandboxes()
            .stop_sandbox(&self.id, StopMode::Graceful)
            .await
    }

    pub async fn kill(&self) -> Result<()> {
        self.backend
            .sandboxes()
            .kill_sandbox(&self.id, KillSignal::Term)
            .await
    }

    pub async fn delete(self) -> Result<()> {
        self.backend
            .sandboxes()
            .delete_sandbox(&self.id, DeleteOptions::default())
            .await
    }

    pub async fn update_deadline(&self, deadline: SandboxDeadline) -> Result<()> {
        self.backend
            .sandboxes()
            .update_deadline(&self.id, deadline)
            .await
    }

    pub fn process(&self) -> ProcessClient {
        ProcessClient::new(self.backend.clone(), self.id.clone())
    }

    pub fn fs(&self) -> crate::filesystem::FilesystemClient {
        crate::filesystem::FilesystemClient::new(self.backend.clone(), self.id.clone())
    }

    pub fn ports(&self) -> crate::ports::PortClient {
        crate::ports::PortClient::new(self.backend.clone(), self.id.clone())
    }

    pub fn logs(&self) -> crate::logs::LogClient {
        crate::logs::LogClient::new(self.backend.clone(), Some(self.id.clone()))
    }

    pub fn metrics(&self) -> crate::metrics::MetricClient {
        crate::metrics::MetricClient::new(self.backend.clone(), Some(self.id.clone()))
    }

    pub async fn snapshot(&self, name: impl Into<String>) -> Result<SnapshotRef> {
        self.backend
            .snapshots()
            .capture_snapshot(&self.id, crate::snapshot::SnapshotOptions::named(name))
            .await
    }

    pub async fn pause(&self, options: PauseOptions) -> Result<PausedSandbox> {
        self.backend
            .snapshots()
            .pause_sandbox(&self.id, options)
            .await
    }

    pub async fn exec(&self, command: Command) -> Result<CommandOutput> {
        self.process().run(command).await
    }

    pub async fn shell(&self) -> Result<Shell> {
        self.process().shell().await
    }

    pub async fn shell_with(&self, opts: ShellOpts) -> Result<Shell> {
        self.process().shell_with(opts).await
    }

    pub async fn shell_pool(&self, size: usize) -> Result<ShellPool> {
        self.process().shell_pool(size).await
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxInfo {
    pub id: SandboxId,
    pub state: SandboxState,
    pub template_id: Option<TemplateId>,
    pub snapshot_id: Option<SnapshotId>,
    pub resources: Option<SandboxResources>,
    pub deadline: Option<SandboxDeadline>,
    pub created_at: OffsetDateTime,
    pub metadata: SandboxMetadata,
}

impl SandboxInfo {
    pub fn running(id: SandboxId) -> Self {
        Self {
            id,
            state: SandboxState::Running,
            template_id: None,
            snapshot_id: None,
            resources: None,
            deadline: None,
            created_at: OffsetDateTime::now_utc(),
            metadata: SandboxMetadata::default(),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxState {
    Creating,
    Running,
    Stopping,
    Stopped,
    Deleted,
    Failed,
}

#[derive(Clone)]
pub struct SandboxClient {
    backend: BoxBackend,
}

impl SandboxClient {
    pub(crate) fn new(backend: BoxBackend) -> Self {
        Self { backend }
    }

    fn control(&self) -> &dyn SandboxControl {
        self.backend.sandboxes()
    }

    pub async fn create(&self, spec: SandboxSpec) -> Result<Sandbox> {
        let info = self.control().create_sandbox(spec).await?;
        Ok(Sandbox::new(info.id, self.backend.clone()))
    }

    pub async fn restore(&self, snapshot: SnapshotRef) -> Result<Sandbox> {
        let info = self
            .control()
            .restore_sandbox(snapshot, crate::snapshot::RestoreOptions::default())
            .await?;
        Ok(Sandbox::new(info.id, self.backend.clone()))
    }

    pub async fn attach(&self, id: &SandboxId, options: AttachOptions) -> Result<Sandbox> {
        let info = self.control().attach_sandbox(id, options).await?;
        Ok(Sandbox::new(info.id, self.backend.clone()))
    }

    pub async fn inspect(&self, id: &SandboxId) -> Result<SandboxInfo> {
        self.control().inspect_sandbox(id).await
    }

    pub async fn list(&self, filter: SandboxFilter) -> Result<Vec<SandboxInfo>> {
        self.control().list_sandboxes(filter).await
    }

    pub async fn stop(&self, id: &SandboxId, mode: StopMode) -> Result<()> {
        self.control().stop_sandbox(id, mode).await
    }

    pub async fn kill(&self, id: &SandboxId, signal: KillSignal) -> Result<()> {
        self.control().kill_sandbox(id, signal).await
    }

    pub async fn delete(&self, id: &SandboxId, options: DeleteOptions) -> Result<()> {
        self.control().delete_sandbox(id, options).await
    }

    pub async fn update_deadline(&self, id: &SandboxId, deadline: SandboxDeadline) -> Result<()> {
        self.control().update_deadline(id, deadline).await
    }
}

impl From<Arc<dyn crate::backend::SandboxBackend>> for SandboxClient {
    fn from(backend: Arc<dyn crate::backend::SandboxBackend>) -> Self {
        Self::new(backend)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SandboxResources {
    pub vcpus: u32,
    pub memory_bytes: u64,
    pub disk_bytes: Option<u64>,
}

impl SandboxResources {
    pub const fn new(vcpus: u32, memory_bytes: u64) -> Self {
        Self {
            vcpus,
            memory_bytes,
            disk_bytes: None,
        }
    }

    pub const fn disk_bytes(mut self, disk_bytes: u64) -> Self {
        self.disk_bytes = Some(disk_bytes);
        self
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxDeadline {
    Timeout(Duration),
    At(OffsetDateTime),
}

impl SandboxDeadline {
    pub const fn timeout(timeout: Duration) -> Self {
        Self::Timeout(timeout)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SandboxMetadata(BTreeMap<String, String>);

impl SandboxMetadata {
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SandboxEnvironment(BTreeMap<String, String>);

impl SandboxEnvironment {
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.insert(key.into(), value.into());
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StopMode {
    #[default]
    Graceful,
    ForceAfter(Duration),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KillSignal {
    #[default]
    Term,
    Kill,
    Interrupt,
    Number(i32),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttachOptions {
    pub require_running: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeleteOptions {
    pub force: bool,
    pub delete_snapshots: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SandboxFilter {
    pub state: Option<SandboxState>,
}

pub(crate) fn unsupported(
    operation: &'static str,
    capability: crate::capability::CapabilityName,
) -> crate::error::Error {
    UnsupportedCapability::new(
        operation,
        capability,
        crate::capability::CapabilityReason::Permanent {
            detail: "backend did not provide this control surface".to_owned(),
        },
    )
    .into()
}
