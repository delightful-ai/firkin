use std::sync::Arc;

use crate::backend::BoxBackend;
use crate::capability::CapabilityName;
use crate::error::Result;
use crate::ids::WarmPoolKey;
use crate::sandbox::unsupported;
use crate::template::PreparedTemplate;

#[derive(Clone)]
pub struct WarmPoolClient {
    backend: BoxBackend,
}

impl WarmPoolClient {
    pub(crate) fn new(backend: BoxBackend) -> Self {
        Self { backend }
    }

    pub async fn prewarm(
        &self,
        template: &PreparedTemplate,
        spec: WarmPoolSpec,
    ) -> Result<WarmMaintainReport> {
        let Some(control) = self.backend.warm_pool() else {
            return Err(unsupported(
                "prewarm sandbox",
                CapabilityName::WarmPoolPrewarm,
            ));
        };
        control.prewarm(template, spec).await
    }

    pub async fn maintain(&self, targets: Vec<WarmPoolTarget>) -> Result<WarmMaintainReport> {
        let Some(control) = self.backend.warm_pool() else {
            return Err(unsupported(
                "maintain warm pool",
                CapabilityName::WarmPoolPrewarm,
            ));
        };
        control.maintain(targets).await
    }

    pub async fn status(&self) -> Result<WarmPoolStatus> {
        let Some(control) = self.backend.warm_pool() else {
            return Err(unsupported(
                "warm-pool status",
                CapabilityName::WarmPoolPrewarm,
            ));
        };
        control.status().await
    }

    pub async fn checkout(
        &self,
        template: &PreparedTemplate,
        policy: WarmLeasePolicy,
    ) -> Result<WarmLease> {
        let Some(control) = self.backend.warm_pool() else {
            return Err(unsupported(
                "checkout warm sandbox",
                CapabilityName::WarmPoolCheckout,
            ));
        };
        control.checkout(template, policy).await
    }

    pub async fn evict(&self, key: WarmPoolKey, count: usize) -> Result<WarmMaintainReport> {
        let Some(control) = self.backend.warm_pool() else {
            return Err(unsupported(
                "evict warm sandboxes",
                CapabilityName::WarmPoolPrewarm,
            ));
        };
        control.evict(key, count).await
    }
}

impl From<Arc<dyn crate::backend::SandboxBackend>> for WarmPoolClient {
    fn from(backend: Arc<dyn crate::backend::SandboxBackend>) -> Self {
        Self::new(backend)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarmPoolSpec {
    pub depth: usize,
    pub min_ready: usize,
    pub eviction: WarmEvictionPolicy,
}

impl WarmPoolSpec {
    pub const fn depth(depth: usize) -> Self {
        Self {
            depth,
            min_ready: depth,
            eviction: WarmEvictionPolicy::Oldest,
        }
    }

    pub const fn min_ready(mut self, min_ready: usize) -> Self {
        self.min_ready = min_ready;
        self
    }

    pub const fn eviction(mut self, policy: WarmEvictionPolicy) -> Self {
        self.eviction = policy;
        self
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarmPoolStatus {
    pub entries: Vec<WarmPoolEntry>,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmPoolEntry {
    pub key: WarmPoolKey,
    pub ready: usize,
    pub total: usize,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmLease {
    pub key: WarmPoolKey,
    pub sandbox_id: crate::ids::SandboxId,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarmLeasePolicy {
    RequireReady,
    CreateIfEmpty,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarmEvictionPolicy {
    Oldest,
    Newest,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WarmMaintainReport {
    pub created: usize,
    pub evicted: usize,
    pub ready: usize,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmPoolTarget {
    pub template_id: crate::ids::TemplateId,
    pub spec: WarmPoolSpec,
}
