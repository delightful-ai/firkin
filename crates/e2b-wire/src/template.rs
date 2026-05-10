//! template — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::logs::LogLevel;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
/// Template build lifecycle status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(private_interfaces)]
pub enum TemplateBuildStatus {
    /// Build is running.
    Building,
    /// Build is waiting for capacity or input.
    Waiting,
    /// Build completed successfully.
    Ready,
    /// Build failed.
    Error,
}
/// E2B template build request body.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(private_interfaces)]
pub struct TemplateBuildRequest {
    /// Optional template name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Requested tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// vCPU count.
    #[serde(rename = "cpuCount", skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<u32>,
    /// Memory size in MiB.
    #[serde(rename = "memoryMB", skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
}
/// E2B template build start body.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(private_interfaces)]
pub struct TemplateBuildStart {
    /// Base image reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_image: Option<String>,
    /// Base template reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_template: Option<String>,
    /// Registry auth/config payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_image_registry: Option<serde_json::Value>,
    /// Force rebuild.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
    /// Template build steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<TemplateStep>,
    /// Optional start command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_cmd: Option<String>,
    /// Optional readiness command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_cmd: Option<String>,
}
/// E2B template build step.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateStep {
    /// Instruction kind.
    #[serde(rename = "type")]
    pub kind: TemplateInstructionKind,
    /// Instruction arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Hash of uploaded COPY archive bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_hash: Option<String>,
    /// Force COPY upload or rebuild step.
    #[serde(default, skip_serializing_if = "is_false")]
    pub force: bool,
}
/// E2B template instruction kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
#[allow(private_interfaces)]
pub enum TemplateInstructionKind {
    /// COPY instruction.
    Copy,
    /// ENV instruction.
    Env,
    /// RUN instruction.
    Run,
    /// WORKDIR instruction.
    Workdir,
    /// USER instruction.
    User,
}
/// E2B template build request response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(private_interfaces)]
pub struct TemplateBuildRequestInfo {
    /// Template id.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Build id.
    #[serde(rename = "buildID")]
    pub build_id: String,
    /// Whether the template is public.
    pub public: bool,
    /// Template names.
    #[serde(default)]
    pub names: Vec<String>,
    /// Template tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Template aliases.
    #[serde(default)]
    pub aliases: Vec<String>,
}
/// Template summary returned by `GET /templates`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(private_interfaces)]
pub struct TemplateInfo {
    /// Template id.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Latest build id.
    #[serde(rename = "buildID")]
    pub build_id: Option<String>,
    /// Whether the template is public.
    pub public: bool,
    /// Template aliases.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Template names.
    #[serde(default)]
    pub names: Vec<String>,
    /// RFC3339 created timestamp.
    pub created_at: String,
    /// RFC3339 updated timestamp.
    pub updated_at: String,
    /// RFC3339 last spawned timestamp.
    pub last_spawned_at: Option<String>,
    /// Spawn count.
    pub spawn_count: u64,
    /// Build count.
    pub build_count: u32,
    /// envd version.
    pub envd_version: Option<String>,
    /// Latest build status.
    pub build_status: Option<TemplateBuildStatus>,
}
/// Template build entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct TemplateBuild {
    /// Build id.
    #[serde(rename = "buildID")]
    pub build_id: String,
    /// Build status.
    pub status: TemplateBuildStatus,
    /// RFC3339 created timestamp.
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// RFC3339 updated timestamp.
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    /// RFC3339 finished timestamp.
    #[serde(rename = "finishedAt")]
    pub finished_at: Option<String>,
    /// vCPU count.
    #[serde(rename = "cpuCount")]
    pub cpu_count: u32,
    /// Memory size in MiB.
    #[serde(rename = "memoryMB")]
    pub memory_mb: u32,
    /// Disk size in MiB.
    #[serde(rename = "diskSizeMB", skip_serializing_if = "Option::is_none")]
    pub disk_size_mb: Option<u32>,
    /// envd version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envd_version: Option<String>,
}
/// Template-with-builds response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(private_interfaces)]
pub struct TemplateWithBuilds {
    /// Template id.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Whether the template is public.
    pub public: bool,
    /// Template aliases.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Template names.
    #[serde(default)]
    pub names: Vec<String>,
    /// RFC3339 created timestamp.
    pub created_at: String,
    /// RFC3339 updated timestamp.
    pub updated_at: String,
    /// RFC3339 last spawned timestamp.
    pub last_spawned_at: Option<String>,
    /// Spawn count.
    pub spawn_count: u64,
    /// Builds.
    #[serde(default)]
    pub builds: Vec<TemplateBuild>,
}
/// Template build info response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct TemplateBuildInfo {
    /// Template id.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Build id.
    #[serde(rename = "buildID")]
    pub build_id: String,
    /// Build status.
    pub status: TemplateBuildStatus,
    /// String log lines.
    #[serde(default)]
    pub logs: Vec<String>,
    /// Structured log entries.
    #[serde(rename = "logEntries", default)]
    pub log_entries: Vec<BuildLogEntry>,
    /// Optional failure reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<BuildStatusReason>,
}
/// Template build log entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct BuildLogEntry {
    /// RFC3339 timestamp.
    pub timestamp: String,
    /// Log level.
    pub level: LogLevel,
    /// Message.
    pub message: String,
    /// Optional build step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}
/// Template build failure reason.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BuildStatusReason {
    /// Failure message.
    pub message: String,
    /// Optional failing step.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// Failure log entries.
    #[serde(rename = "logEntries", default)]
    pub log_entries: Vec<BuildLogEntry>,
}
/// Template build logs response.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct TemplateBuildLogs {
    /// Build logs.
    #[serde(default)]
    pub logs: Vec<BuildLogEntry>,
}
/// Template public update request.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct TemplateUpdateRequest {
    /// Public visibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
}
/// Template update response.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct TemplateUpdateInfo {
    /// Template names.
    #[serde(default)]
    pub names: Vec<String>,
}
/// Template file upload response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct TemplateFileUpload {
    /// Whether the file already exists server side.
    pub present: bool,
    /// Upload URL when upload is required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}
/// Template tag assignment request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct AssignTemplateTags {
    /// Target template/name/build expression.
    pub target: String,
    /// Tags to assign.
    pub tags: Vec<String>,
}
/// Template tag removal request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct RemoveTemplateTags {
    /// Template name.
    pub name: String,
    /// Tags to remove.
    pub tags: Vec<String>,
}
/// Template tag assignment response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct AssignedTemplateTags {
    /// Assigned tags.
    pub tags: Vec<String>,
    /// Build id.
    #[serde(rename = "buildID")]
    pub build_id: String,
}
/// Template tag response item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(private_interfaces)]
pub struct TemplateTag {
    /// Tag.
    pub tag: String,
    /// Build id.
    #[serde(rename = "buildID")]
    pub build_id: String,
    /// RFC3339 created timestamp.
    pub created_at: String,
}
/// Template alias response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct TemplateAliasInfo {
    /// Template id.
    #[serde(rename = "templateID")]
    pub template_id: String,
    /// Whether the template is public.
    pub public: bool,
}
/// Logical paginated response used with `x-next-token` transport headers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Page<T> {
    /// Items in this page.
    pub items: Vec<T>,
    /// Next page token from the `x-next-token` header.
    pub next_token: Option<String>,
}
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_false(value: &bool) -> bool {
    !*value
}
