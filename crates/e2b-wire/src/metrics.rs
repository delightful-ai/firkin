//! metrics — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::BTreeMap;
/// Sandbox metric sample shape used by SDK routes.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct SandboxMetric {
    /// RFC3339 sample timestamp.
    pub timestamp: String,
    /// CPU used percent.
    #[serde(rename = "cpuUsedPct")]
    pub cpu_used_pct: f64,
    /// vCPU count.
    #[serde(rename = "cpuCount")]
    pub cpu_count: u32,
    /// Used memory bytes.
    #[serde(rename = "memUsed")]
    pub mem_used: u64,
    /// Total memory bytes.
    #[serde(rename = "memTotal")]
    pub mem_total: u64,
    /// Used disk bytes.
    #[serde(rename = "diskUsed")]
    pub disk_used: u64,
    /// Total disk bytes.
    #[serde(rename = "diskTotal")]
    pub disk_total: u64,
}
/// Metrics response for querying many sandboxes at once.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct SandboxesWithMetrics {
    /// Metrics keyed by sandbox id.
    pub sandboxes: BTreeMap<String, SandboxMetric>,
}
