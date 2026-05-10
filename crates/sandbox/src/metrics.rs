use std::sync::Arc;

use time::OffsetDateTime;

use crate::backend::BoxBackend;
use crate::capability::CapabilityName;
use crate::error::Result;
use crate::ids::SandboxId;
use crate::sandbox::unsupported;

#[derive(Clone)]
pub struct MetricClient {
    backend: BoxBackend,
    sandbox_id: Option<SandboxId>,
}

impl MetricClient {
    pub(crate) fn new(backend: BoxBackend, sandbox_id: Option<SandboxId>) -> Self {
        Self {
            backend,
            sandbox_id,
        }
    }

    pub async fn snapshot(&self, filter: MetricFilter) -> Result<MetricSnapshot> {
        let Some(control) = self.backend.metrics() else {
            return Err(unsupported("metric snapshot", CapabilityName::MetricsHost));
        };
        control
            .metric_snapshot(self.sandbox_id.as_ref(), filter)
            .await
    }
}

impl From<(Arc<dyn crate::backend::SandboxBackend>, Option<SandboxId>)> for MetricClient {
    fn from(
        (backend, sandbox_id): (Arc<dyn crate::backend::SandboxBackend>, Option<SandboxId>),
    ) -> Self {
        Self::new(backend, sandbox_id)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    pub name: MetricName,
    pub value: MetricValue,
    pub unit: MetricUnit,
    pub scope: MetricScope,
    pub observed_at: OffsetDateTime,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetricName(String);

impl MetricName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MetricValue {
    Count(u64),
    Gauge(f64),
    Text(&'static str),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricUnit {
    Count,
    Bytes,
    Milliseconds,
    Percent,
    Ratio,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MetricScope {
    Runtime,
    Sandbox(SandboxId),
    Guest(SandboxId),
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricSnapshot {
    pub metrics: Vec<Metric>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetricFilter {
    pub prefix: Option<String>,
}
