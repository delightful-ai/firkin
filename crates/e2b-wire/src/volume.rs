//! volume — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
/// Volume mount payload used by sandbox create and info responses.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct VolumeMount {
    /// Guest mount path.
    pub path: String,
    /// Control-plane volume name.
    pub name: String,
}
/// E2B volume create request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct VolumeCreateRequest {
    /// Volume name.
    pub name: String,
}
/// E2B volume info response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct VolumeInfo {
    /// Volume id.
    #[serde(rename = "volumeID")]
    pub volume_id: String,
    /// Volume name.
    pub name: String,
}
/// E2B volume response that includes the access token.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct VolumeAndToken {
    /// Volume id.
    #[serde(rename = "volumeID")]
    pub volume_id: String,
    /// Volume name.
    pub name: String,
    /// Volume access token.
    pub token: String,
}
/// E2B volume file type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(private_interfaces)]
pub enum VolumeFileType {
    /// Unknown file type.
    Unknown,
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
}
/// E2B volume entry stat response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct VolumeEntryStat {
    /// Basename.
    pub name: String,
    /// Entry type.
    #[serde(rename = "type")]
    pub kind: VolumeFileType,
    /// Absolute volume path.
    pub path: String,
    /// Entry size in bytes.
    pub size: i64,
    /// POSIX mode.
    pub mode: u32,
    /// Owner user id.
    pub uid: u32,
    /// Owner group id.
    pub gid: u32,
    /// RFC3339 access timestamp.
    pub atime: String,
    /// RFC3339 modification timestamp.
    pub mtime: String,
    /// RFC3339 status-change timestamp.
    pub ctime: String,
    /// Symlink target.
    pub target: Option<String>,
}
/// E2B volume metadata update request.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct VolumeMetadataRequest {
    /// Owner user id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    /// Owner group id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gid: Option<u32>,
    /// POSIX mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}
/// E2B volume content write options.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct VolumeWriteOptions {
    /// Owner user id.
    pub uid: Option<u32>,
    /// Owner group id.
    pub gid: Option<u32>,
    /// POSIX mode.
    pub mode: Option<u32>,
    /// Whether to overwrite an existing path.
    pub force: Option<bool>,
}
