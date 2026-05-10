//! layout — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::Error;
#[allow(unused_imports)]
use crate::GuestPath;
#[allow(unused_imports)]
use crate::Result;
#[allow(unused_imports)]
use crate::pod::id::PodId;
#[allow(unused_imports)]
use crate::pod::store::MountedPodStore;
#[allow(unused_imports)]
use firkin_oci::Layer;
#[allow(unused_imports)]
use firkin_types::ContainerId;
#[cfg(test)]
#[allow(unused_imports)]
use std::io;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PodTemplateKey(String);
impl PodTemplateKey {
    pub(crate) fn from_digest(digest: &str) -> Result<Self> {
        let key = path_safe_digest_key(digest)?;
        Ok(Self(key))
    }
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PodTemplate {
    pub(crate) rootfs: GuestPath,
}
impl PodTemplate {
    pub(crate) const fn new(rootfs: GuestPath) -> Self {
        Self { rootfs }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PodContainerLayout {
    pub(crate) base: GuestPath,
    pub(crate) upper: GuestPath,
    pub(crate) work: GuestPath,
    pub(crate) merged: GuestPath,
}
impl PodContainerLayout {
    pub(crate) fn new(
        store: &MountedPodStore,
        pod_id: &PodId,
        container_id: &ContainerId,
    ) -> Result<Self> {
        let base = pod_container_base_path(store, pod_id, container_id)?;
        Ok(Self {
            upper: pod_container_upper_path(store, pod_id, container_id)?,
            work: pod_container_work_path(store, pod_id, container_id)?,
            merged: pod_container_merged_path(store, pod_id, container_id)?,
            base,
        })
    }
}
pub(crate) fn pod_base_path(store: &MountedPodStore, pod_id: &PodId) -> Result<GuestPath> {
    pod_path(store, pod_id, [])
}
pub(crate) fn pod_rootfs_base_path(store: &MountedPodStore, pod_id: &PodId) -> Result<GuestPath> {
    pod_path(store, pod_id, ["rootfs"])
}
pub(crate) fn pod_rootfs_layers_base_path(
    store: &MountedPodStore,
    pod_id: &PodId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["layers"])
}
pub(crate) fn pod_templates_base_path(
    store: &MountedPodStore,
    pod_id: &PodId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["templates"])
}
pub(crate) fn pod_containers_base_path(
    store: &MountedPodStore,
    pod_id: &PodId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["containers"])
}
pub(crate) fn pod_empty_dir_base_path(
    store: &MountedPodStore,
    pod_id: &PodId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["emptydir"])
}
pub(crate) fn pod_rootfs_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    container_id: &ContainerId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["rootfs", container_id.as_str()])
}
pub(crate) fn pod_rootfs_layer_base_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    container_id: &ContainerId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["layers", container_id.as_str()])
}
pub(crate) fn pod_rootfs_layer_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    container_id: &ContainerId,
    layer: &Layer,
) -> Result<GuestPath> {
    let layer_key = path_safe_digest_key(layer.digest().as_str())?;
    pod_path(store, pod_id, ["layers", container_id.as_str(), &layer_key])
}
pub(crate) fn pod_template_base_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    key: &PodTemplateKey,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["templates", key.as_str()])
}
pub(crate) fn pod_template_rootfs_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    key: &PodTemplateKey,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["templates", key.as_str(), "rootfs"])
}
pub(crate) fn pod_template_layers_base_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    key: &PodTemplateKey,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["templates", key.as_str(), "layers"])
}
pub(crate) fn pod_template_layer_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    key: &PodTemplateKey,
    layer: &Layer,
) -> Result<GuestPath> {
    let layer_key = path_safe_digest_key(layer.digest().as_str())?;
    pod_path(
        store,
        pod_id,
        ["templates", key.as_str(), "layers", &layer_key],
    )
}
fn pod_container_base_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    container_id: &ContainerId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["containers", container_id.as_str()])
}
fn pod_container_upper_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    container_id: &ContainerId,
) -> Result<GuestPath> {
    pod_path(
        store,
        pod_id,
        ["containers", container_id.as_str(), "upper"],
    )
}
fn pod_container_work_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    container_id: &ContainerId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["containers", container_id.as_str(), "work"])
}
fn pod_container_merged_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    container_id: &ContainerId,
) -> Result<GuestPath> {
    pod_path(
        store,
        pod_id,
        ["containers", container_id.as_str(), "merged"],
    )
}
pub(crate) fn pod_empty_dir_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    volume_name: &ContainerId,
) -> Result<GuestPath> {
    pod_path(store, pod_id, ["emptydir", volume_name.as_str()])
}
fn pod_path<'a>(
    store: &MountedPodStore,
    pod_id: &PodId,
    components: impl IntoIterator<Item = &'a str>,
) -> Result<GuestPath> {
    let mut path = format!("{}/pods/{}", store.guest_mount().as_str(), pod_id.as_str());
    for component in components {
        path.push('/');
        path.push_str(component);
    }
    GuestPath::new(path)
}
fn path_safe_digest_key(digest: &str) -> Result<String> {
    let mut key = String::with_capacity(digest.len());
    for byte in digest.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
            key.push(char::from(byte));
        } else {
            key.push('-');
        }
    }
    if key.is_empty() || key == "." || key == ".." {
        return Err(Error::RuntimeOperation {
            operation: "build pod digest path key",
            reason: format!("digest `{digest}` does not produce a safe path segment"),
        });
    }
    Ok(key)
}
