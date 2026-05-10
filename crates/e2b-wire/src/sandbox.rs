//! sandbox — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::volume::VolumeMount;
#[allow(unused_imports)]
use firkin_types::SandboxNetworkPolicy;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use serde_json::Value as JsonValue;
#[allow(unused_imports)]
use std::collections::BTreeMap;
/// Sandbox lifecycle state exposed by the E2B API.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(private_interfaces)]
pub enum SandboxState {
    /// Sandbox is running and envd should be reachable.
    Running,
    /// Sandbox is paused and must be connected/resumed before envd traffic.
    Paused,
}
/// Auto-resume payload nested under `autoResume`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoResume {
    /// Whether the sandbox should automatically resume after timeout pause.
    pub enabled: bool,
}
/// E2B sandbox create request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct SandboxCreateRequest {
    /// Template id or alias.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Timeout in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Caller-supplied metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    /// Environment variables.
    #[serde(
        rename = "envVars",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub env_vars: BTreeMap<String, String>,
    /// MCP gateway configuration requested by the SDK.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<JsonValue>,
    /// Whether secure sandbox behavior is requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,
    /// Top-level E2B internet access switch.
    #[serde(
        rename = "allow_internet_access",
        skip_serializing_if = "Option::is_none"
    )]
    pub allow_internet_access: Option<bool>,
    /// Nested E2B network policy object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SandboxNetworkPolicy>,
    /// Whether timeout should pause instead of kill.
    #[serde(rename = "autoPause", skip_serializing_if = "Option::is_none")]
    pub auto_pause: Option<bool>,
    /// Auto-resume behavior.
    #[serde(rename = "autoResume", skip_serializing_if = "Option::is_none")]
    pub auto_resume: Option<AutoResume>,
    /// Volume mounts requested by the SDK.
    #[serde(
        rename = "volumeMounts",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub volume_mounts: Vec<VolumeMount>,
}
impl Default for SandboxCreateRequest {
    fn default() -> Self {
        Self {
            template_id: "base".to_owned(),
            timeout: Some(300),
            metadata: BTreeMap::new(),
            env_vars: BTreeMap::new(),
            mcp: None,
            secure: Some(true),
            allow_internet_access: Some(true),
            network: None,
            auto_pause: Some(false),
            auto_resume: None,
            volume_mounts: Vec::new(),
        }
    }
}
/// E2B/Cube follow-up create request backed by a continuation snapshot.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct FollowupSandboxCreateRequest {
    /// Snapshot id to restore as the new follow-up sandbox.
    #[serde(rename = "snapshotID")]
    pub snapshot_id: String,
    /// Create options for the follow-up sandbox.
    #[serde(rename = "createRequest")]
    pub create_request: SandboxCreateRequest,
}
/// E2B connect request body.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct ConnectRequest {
    /// Timeout in seconds.
    pub timeout: u64,
}
/// E2B set-timeout request body.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct TimeoutRequest {
    /// Timeout in seconds.
    pub timeout: u64,
}
/// E2B refresh request body.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct RefreshRequest {
    /// Optional refresh duration in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<u64>,
}
/// E2B create/connect response consumed by the SDK to construct envd URLs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct ConnectedSandbox {
    /// Sandbox id.
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    /// envd semantic version string.
    #[serde(rename = "envdVersion")]
    pub envd_version: String,
    /// Optional envd access token.
    #[serde(rename = "envdAccessToken", skip_serializing_if = "Option::is_none")]
    pub envd_access_token: Option<String>,
    /// Optional proxy traffic token.
    #[serde(rename = "trafficAccessToken", skip_serializing_if = "Option::is_none")]
    pub traffic_access_token: Option<String>,
    /// Sandbox domain used by `get_host(port)`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}
/// E2B sandbox info response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct SandboxInfo {
    /// Sandbox id.
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    /// Template id.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Optional alias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Caller metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    /// RFC3339 start timestamp.
    #[serde(rename = "startedAt")]
    pub started_at: String,
    /// RFC3339 timeout/deadline timestamp.
    #[serde(rename = "endAt")]
    pub end_at: String,
    /// Current sandbox state.
    pub state: SandboxState,
    /// vCPU count.
    #[serde(rename = "cpuCount")]
    pub cpu_count: u32,
    /// Memory size in MiB.
    #[serde(rename = "memoryMB")]
    pub memory_mb: u32,
    /// envd semantic version string.
    #[serde(rename = "envdVersion")]
    pub envd_version: String,
    /// SDK-visible internet-access setting.
    #[serde(rename = "allowInternetAccess")]
    pub allow_internet_access: Option<bool>,
    /// SDK-visible nested network settings.
    pub network: Option<SandboxNetworkPolicy>,
    /// Volume mounts attached to the sandbox.
    #[serde(rename = "volumeMounts", default)]
    pub volume_mounts: Vec<VolumeMount>,
}
