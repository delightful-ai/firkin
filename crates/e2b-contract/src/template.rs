//! template — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_e2b_wire::TemplateBuildStart;
#[allow(unused_imports)]
use serde::Deserialize;
#[allow(unused_imports)]
use serde::Serialize;
#[allow(unused_imports)]
use std::collections::BTreeMap;
/// Prepared template returned by a runtime adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct PreparedTemplate {
    /// Template id.
    pub template_id: String,
    /// Build id.
    pub build_id: String,
    /// Rootfs or runtime artifact reference.
    pub artifact: String,
    /// Whether envd is expected inside the template.
    pub has_envd: bool,
    /// Expected integrity for snapshot artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_integrity: Option<PreparedTemplateArtifactIntegrity>,
}
/// Expected integrity for a prepared template artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct PreparedTemplateArtifactIntegrity {
    /// Expected artifact byte length.
    pub size_bytes: u64,
    /// Expected SHA-256 hex digest.
    pub sha256_hex: String,
}
/// Runtime template build request.
#[derive(Clone, Debug, PartialEq)]
#[allow(private_interfaces)]
pub struct RuntimeTemplateBuild {
    /// Template id.
    pub template_id: String,
    /// Build id.
    pub build_id: String,
    /// Build start inputs.
    pub start: TemplateBuildStart,
    /// Uploaded COPY archives keyed by hash.
    pub uploaded_files: BTreeMap<String, Vec<u8>>,
}
