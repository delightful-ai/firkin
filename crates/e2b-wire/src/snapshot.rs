//! snapshot — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
/// E2B create-snapshot request body.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct CreateSnapshotRequest {
    /// Optional snapshot name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
/// E2B snapshot info response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct SnapshotInfo {
    /// Snapshot id.
    #[serde(rename = "snapshotID")]
    pub snapshot_id: String,
}
