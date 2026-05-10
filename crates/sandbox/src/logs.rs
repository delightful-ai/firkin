use std::pin::Pin;
use std::sync::Arc;

use futures_core::Stream;
use time::OffsetDateTime;

use crate::backend::BoxBackend;
use crate::capability::CapabilityName;
use crate::error::Result;
use crate::ids::SandboxId;
use crate::sandbox::unsupported;

pub type LogStream = Pin<Box<dyn Stream<Item = Result<LogEntry>> + Send + 'static>>;

#[derive(Clone)]
pub struct LogClient {
    backend: BoxBackend,
    sandbox_id: Option<SandboxId>,
}

impl LogClient {
    pub(crate) fn new(backend: BoxBackend, sandbox_id: Option<SandboxId>) -> Self {
        Self {
            backend,
            sandbox_id,
        }
    }

    pub async fn list(&self, filter: LogFilter) -> Result<Vec<LogEntry>> {
        let Some(control) = self.backend.logs() else {
            return Err(unsupported("list logs", CapabilityName::EventsSubscribe));
        };
        control.list_logs(self.sandbox_id.as_ref(), filter).await
    }

    pub async fn stream(&self, filter: LogFilter) -> Result<LogStream> {
        let Some(control) = self.backend.logs() else {
            return Err(unsupported("stream logs", CapabilityName::EventsSubscribe));
        };
        control.stream_logs(self.sandbox_id.as_ref(), filter).await
    }
}

impl From<(Arc<dyn crate::backend::SandboxBackend>, Option<SandboxId>)> for LogClient {
    fn from(
        (backend, sandbox_id): (Arc<dyn crate::backend::SandboxBackend>, Option<SandboxId>),
    ) -> Self {
        Self::new(backend, sandbox_id)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub sandbox_id: Option<SandboxId>,
    pub source: LogSource,
    pub level: LogLevel,
    pub message: String,
    pub observed_at: OffsetDateTime,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogFilter {
    pub source: Option<LogSource>,
    pub level: Option<LogLevel>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogSource {
    Runtime,
    Boot,
    Process,
    Filesystem,
    Backend(String),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
