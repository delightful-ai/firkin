//! logs — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::BTreeMap;
/// Log level used by E2B sandbox log routes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(private_interfaces)]
pub enum LogLevel {
    /// Debug log level.
    Debug,
    /// Info log level.
    Info,
    /// Warning log level.
    Warn,
    /// Error log level.
    Error,
}
/// Log pagination direction used by E2B sandbox log routes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogsDirection {
    /// Forward log pagination.
    Forward,
    /// Backward log pagination.
    Backward,
}
/// Structured sandbox log entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct SandboxLogEntry {
    /// RFC3339 log timestamp.
    pub timestamp: String,
    /// Log level.
    pub level: LogLevel,
    /// Log message.
    pub message: String,
    /// Structured fields.
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}
/// E2B sandbox logs response.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct SandboxLogs {
    /// Log entries.
    pub logs: Vec<SandboxLogEntry>,
}
