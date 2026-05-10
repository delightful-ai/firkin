use std::pin::Pin;
use std::sync::Arc;

use futures_core::Stream;
use time::OffsetDateTime;

use crate::backend::BoxBackend;
use crate::capability::CapabilityName;
use crate::error::Result;
use crate::ids::{ProcessId, SandboxId, SnapshotId, WarmPoolKey};
use crate::sandbox::unsupported;

pub type EventStream = Pin<Box<dyn Stream<Item = Result<Event>> + Send + 'static>>;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: EventKind,
    pub observed_at: OffsetDateTime,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventKind {
    Lifecycle(LifecycleEvent),
    Process(ProcessEvent),
    Filesystem(FilesystemEvent),
    Snapshot(SnapshotEvent),
    WarmPool(WarmPoolEvent),
    Port(PortEvent),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventFilter {
    pub sandbox_id: Option<SandboxId>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub sandbox_id: SandboxId,
    pub state: crate::sandbox::SandboxState,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessEvent {
    pub sandbox_id: SandboxId,
    pub process_id: ProcessId,
    pub status: crate::process::ProcessStatus,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesystemEvent {
    pub sandbox_id: SandboxId,
    pub path: crate::filesystem::SandboxPath,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEvent {
    pub sandbox_id: Option<SandboxId>,
    pub snapshot_id: SnapshotId,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmPoolEvent {
    pub key: WarmPoolKey,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortEvent {
    pub sandbox_id: SandboxId,
    pub port: crate::ports::GuestPort,
}

#[derive(Clone)]
pub struct EventClient {
    backend: BoxBackend,
}

impl EventClient {
    pub(crate) fn new(backend: BoxBackend) -> Self {
        Self { backend }
    }

    pub async fn subscribe(&self, filter: EventFilter) -> Result<EventStream> {
        let Some(control) = self.backend.events() else {
            return Err(unsupported(
                "subscribe events",
                CapabilityName::EventsSubscribe,
            ));
        };
        control.subscribe_events(filter).await
    }
}

impl From<Arc<dyn crate::backend::SandboxBackend>> for EventClient {
    fn from(backend: Arc<dyn crate::backend::SandboxBackend>) -> Self {
        Self::new(backend)
    }
}
