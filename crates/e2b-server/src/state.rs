//! state — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::registry::{
    LocalPodRegistry, LocalSandboxRegistry, LocalTemplateRegistry, LocalVolumeRegistry,
};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
/// Persisted SDK-visible control-plane state.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LocalRuntimeState {
    /// Sandbox registry state.
    pub sandboxes: LocalSandboxRegistry,
    /// Product pod registry state.
    #[serde(default)]
    pub pods: LocalPodRegistry,
    /// Template registry state.
    pub templates: LocalTemplateRegistry,
    /// Volume registry state.
    pub volumes: LocalVolumeRegistry,
}
/// Errors produced when loading or saving local runtime state.
#[derive(Debug, thiserror::Error)]
pub enum LocalRuntimeStateStoreError {
    /// Filesystem error.
    #[error("state store io error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization error.
    #[error("state store json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Unsupported state schema.
    #[error("unsupported state schema {0}")]
    UnsupportedSchema(String),
    /// Unsupported state version.
    #[error("unsupported state version {0}")]
    UnsupportedVersion(u32),
}
