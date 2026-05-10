//! rootfs — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::GuestPath;
#[allow(unused_imports)]
use crate::pod::layout::PodContainerLayout;
#[allow(unused_imports)]
use firkin_vminitd_client::ApplyOciLayer;
#[allow(unused_imports)]
use firkin_vminitd_client::RemovePath;
#[cfg(test)]
#[allow(unused_imports)]
use std::io;
/// Rootfs directory materialized inside a pod store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedRootfs {
    pub(crate) path: GuestPath,
    source_digest: Option<String>,
}
impl MaterializedRootfs {
    /// Construct a materialized rootfs report.
    #[must_use]
    pub fn new(path: GuestPath, source_digest: Option<String>) -> Self {
        Self {
            path,
            source_digest,
        }
    }
    /// Return the guest rootfs path.
    #[must_use]
    pub const fn path(&self) -> &GuestPath {
        &self.path
    }
    /// Return the source digest, if the source had one.
    #[must_use]
    pub fn source_digest(&self) -> Option<&str> {
        self.source_digest.as_deref()
    }
}
pub(crate) fn apply_oci_layer_request(
    archive_path: &GuestPath,
    destination: &GuestPath,
) -> firkin_vminitd_client::pb::ApplyOciLayerRequest {
    ApplyOciLayer::new(archive_path.as_str(), destination.as_str()).into_request()
}
pub(crate) fn container_overlay_mount_request(
    template_rootfs: &GuestPath,
    layout: &PodContainerLayout,
) -> firkin_vminitd_client::pb::MountRequest {
    firkin_vminitd_client::pb::MountRequest {
        r#type: "overlay".to_owned(),
        source: "overlay".to_owned(),
        destination: layout.merged.as_str().to_owned(),
        options: vec![
            format!("lowerdir={}", template_rootfs.as_str()),
            format!("upperdir={}", layout.upper.as_str()),
            format!("workdir={}", layout.work.as_str()),
        ],
    }
}
pub(crate) fn container_overlay_umount_request(
    layout: &PodContainerLayout,
) -> firkin_vminitd_client::pb::UmountRequest {
    firkin_vminitd_client::pb::UmountRequest {
        path: layout.merged.as_str().to_owned(),
        flags: 2,
    }
}
pub(crate) fn container_layout_remove_request(
    layout: &PodContainerLayout,
) -> firkin_vminitd_client::pb::RemovePathRequest {
    RemovePath::recursive(layout.base.as_str())
        .allow_missing(true)
        .into_request()
}
