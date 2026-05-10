use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use time::OffsetDateTime;

use crate::backend::{BoxBackend, SandboxBackend};
use crate::capability::Capabilities;
use crate::error::{InvalidSpec, InvalidSpecReason, Result};
use crate::event::{EventFilter, EventStream};
use crate::ids::RuntimeId;

#[derive(Clone)]
pub struct Runtime {
    backend: BoxBackend,
    config: RuntimeConfig,
}

impl Runtime {
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::default()
    }

    pub async fn build<B>(backend: B) -> Result<Self>
    where
        B: SandboxBackend + 'static,
    {
        RuntimeBuilder::default().backend(backend).build().await
    }

    pub async fn capabilities(&self) -> Result<Capabilities> {
        self.backend.capabilities().await
    }

    pub async fn preflight(&self) -> Result<RuntimePreflight> {
        self.backend.preflight().await
    }

    pub async fn info(&self) -> Result<RuntimeInfo> {
        self.backend.info().await.map(|backend| RuntimeInfo {
            id: self.config.id.clone(),
            backend,
            created_at: self.config.created_at,
        })
    }

    pub fn templates(&self) -> crate::template::TemplateClient {
        crate::template::TemplateClient::new(self.backend.clone())
    }

    pub fn sandboxes(&self) -> crate::sandbox::SandboxClient {
        crate::sandbox::SandboxClient::new(self.backend.clone())
    }

    pub fn warm_pool(&self) -> crate::warm_pool::WarmPoolClient {
        crate::warm_pool::WarmPoolClient::new(self.backend.clone())
    }

    pub fn snapshots(&self) -> crate::snapshot::SnapshotClient {
        crate::snapshot::SnapshotClient::new(self.backend.clone())
    }

    pub fn logs(&self) -> crate::logs::LogClient {
        crate::logs::LogClient::new(self.backend.clone(), None)
    }

    pub fn metrics(&self) -> crate::metrics::MetricClient {
        crate::metrics::MetricClient::new(self.backend.clone(), None)
    }

    pub async fn subscribe(&self, filter: EventFilter) -> Result<EventStream> {
        crate::event::EventClient::new(self.backend.clone())
            .subscribe(filter)
            .await
    }
}

#[derive(Default)]
pub struct RuntimeBuilder {
    backend: Option<BoxBackend>,
    capacity: Option<Capacity>,
    deadline_policy: Option<DeadlinePolicy>,
    hygiene_policy: Option<HygienePolicy>,
}

impl RuntimeBuilder {
    pub fn backend<B>(mut self, backend: B) -> Self
    where
        B: SandboxBackend + 'static,
    {
        self.backend = Some(Arc::new(backend));
        self
    }

    pub fn backend_arc(mut self, backend: BoxBackend) -> Self {
        self.backend = Some(backend);
        self
    }

    pub fn capacity(mut self, capacity: Capacity) -> Self {
        self.capacity = Some(capacity);
        self
    }

    pub fn deadline_policy(mut self, policy: DeadlinePolicy) -> Self {
        self.deadline_policy = Some(policy);
        self
    }

    pub fn hygiene_policy(mut self, policy: HygienePolicy) -> Self {
        self.hygiene_policy = Some(policy);
        self
    }

    pub async fn build(self) -> Result<Runtime> {
        let backend = self
            .backend
            .ok_or_else(|| InvalidSpec::new("build runtime", InvalidSpecReason::MissingBackend))?;
        backend.preflight().await?;
        Ok(Runtime {
            backend,
            config: RuntimeConfig {
                id: RuntimeId::new("default-runtime")?,
                capacity: self.capacity,
                deadline_policy: self.deadline_policy,
                hygiene_policy: self.hygiene_policy,
                created_at: OffsetDateTime::now_utc(),
            },
        })
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub id: RuntimeId,
    pub capacity: Option<Capacity>,
    pub deadline_policy: Option<DeadlinePolicy>,
    pub hygiene_policy: Option<HygienePolicy>,
    pub created_at: OffsetDateTime,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub id: RuntimeId,
    pub backend: crate::backend::BackendInfo,
    pub created_at: OffsetDateTime,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeState {
    Starting,
    Ready,
    Degraded,
    Stopped,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeRoot(PathBuf);

impl RuntimeRoot {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn as_path(&self) -> &std::path::Path {
        &self.0
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub max_sandboxes: Option<usize>,
    pub max_vcpus: Option<u32>,
    pub max_memory_bytes: Option<u64>,
}

impl Capacity {
    pub const fn new() -> Self {
        Self {
            max_sandboxes: None,
            max_vcpus: None,
            max_memory_bytes: None,
        }
    }

    pub const fn max_sandboxes(mut self, max: usize) -> Self {
        self.max_sandboxes = Some(max);
        self
    }
}

impl Default for Capacity {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadlinePolicy {
    pub default_timeout: Option<Duration>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HygienePolicy {
    pub delete_on_drop: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePreflight {
    pub ready: bool,
    pub details: Vec<String>,
}

impl RuntimePreflight {
    pub fn ready() -> Self {
        Self {
            ready: true,
            details: Vec::new(),
        }
    }
}
