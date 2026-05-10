//! VM-backed pod orchestration surface.
#[allow(unused_imports)]
use crate::{
    GuestPath, Result, VminitdClient, allocate_vm_stdio_port, await_copy_control,
    block_device_guest_path, io_runtime_error, run_copy_control, runtime_rpc_error,
    runtime_vmm_error, vsock_runtime_error,
};
#[cfg(test)]
#[allow(unused_imports)]
use firkin_types::{BlockDeviceId, ContainerId};
#[allow(unused_imports)]
use firkin_vminitd_client::{CopyTransfer, RemovePath};
#[allow(unused_imports)]
use firkin_vmm::{Running, VirtualMachine};
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use tokio::io::AsyncWriteExt;
pub(crate) async fn mount_pod_store_with_client(
    client: &mut VminitdClient,
    spec: &PodStoreSpec,
) -> Result<MountedPodStore> {
    client
        .mkdir(tonic::Request::new(
            firkin_vminitd_client::pb::MkdirRequest {
                path: spec.guest_mount().as_str().to_owned(),
                all: true,
                perms: 0o755,
            },
        ))
        .await
        .map_err(runtime_rpc_error("mkdir pod store"))?;
    client
        .mount(tonic::Request::new(
            firkin_vminitd_client::pb::MountRequest {
                r#type: spec.filesystem().mount_type().to_owned(),
                source: block_device_guest_path(spec.block_device())?,
                destination: spec.guest_mount().as_str().to_owned(),
                options: vec!["rw".to_owned()],
            },
        ))
        .await
        .map_err(runtime_rpc_error("mount pod store"))?;
    Ok(MountedPodStore { spec: spec.clone() })
}
pub(crate) async fn prepare_pod_directories(
    client: &mut VminitdClient,
    store: &MountedPodStore,
    pod_id: &PodId,
) -> Result<()> {
    for path in [
        pod_base_path(store, pod_id)?,
        pod_rootfs_base_path(store, pod_id)?,
        pod_rootfs_layers_base_path(store, pod_id)?,
        pod_templates_base_path(store, pod_id)?,
        pod_containers_base_path(store, pod_id)?,
        pod_empty_dir_base_path(store, pod_id)?,
    ] {
        mkdir_guest_path(client, &path, 0o755, "mkdir pod directory").await?;
    }
    Ok(())
}
pub(crate) async fn mkdir_guest_path(
    client: &mut VminitdClient,
    path: &GuestPath,
    perms: u32,
    operation: &'static str,
) -> Result<()> {
    client
        .mkdir(tonic::Request::new(
            firkin_vminitd_client::pb::MkdirRequest {
                path: path.as_str().to_owned(),
                all: true,
                perms,
            },
        ))
        .await
        .map_err(runtime_rpc_error(operation))?;
    Ok(())
}
pub(crate) async fn remove_guest_path(
    client: &mut VminitdClient,
    path: &GuestPath,
    operation: &'static str,
) -> Result<()> {
    client
        .remove_path(tonic::Request::new(
            RemovePath::recursive(path.as_str())
                .allow_missing(true)
                .into_request(),
        ))
        .await
        .map_err(runtime_rpc_error(operation))?;
    Ok(())
}
pub(crate) async fn copy_file_to_guest_path(
    vm: &VirtualMachine<Running>,
    client: &mut VminitdClient,
    source_path: &Path,
    destination: &GuestPath,
) -> Result<()> {
    let port = allocate_vm_stdio_port(vm.id(), "allocate pod rootfs copy port")?;
    let listener = vm
        .listen_reserved_port(port)
        .map_err(runtime_vmm_error("listen for pod rootfs copy"))?;
    let request = CopyTransfer::copy_in(destination.as_str(), port)
        .create_parents(true)
        .archive(false)
        .into_request();
    let copy_task = tokio::spawn(run_copy_control(client.clone(), request, None));
    let (mut stream, _peer) = listener
        .accept()
        .await
        .map_err(vsock_runtime_error("accept pod rootfs copy connection"))?;
    let mut file = tokio::fs::File::open(source_path)
        .await
        .map_err(io_runtime_error("open pod rootfs copy source"))?;
    tokio::io::copy(&mut file, &mut stream)
        .await
        .map_err(io_runtime_error("stream pod rootfs copy source"))?;
    stream
        .shutdown()
        .await
        .map_err(io_runtime_error("finish pod rootfs copy stream"))?;
    await_copy_control(copy_task, "copy pod rootfs layer").await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;
    fn mounted_store() -> MountedPodStore {
        MountedPodStore {
            spec: PodStoreSpec::ext4(BlockDeviceId::from_slot(NonZeroU32::new(2).unwrap())),
        }
    }
    #[test]
    fn pod_template_key_is_stable_and_path_safe() {
        let key = PodTemplateKey::from_digest("sha256:abc/def").unwrap();
        let full_digest = PodTemplateKey::from_digest(
            "sha256:2f3adcef67f09f6a99c349bfbb6ba429e4acd925d86f69bcc51322c096581c17",
        )
        .unwrap();
        assert_eq!(key.as_str(), "sha256-abc-def");
        assert_eq!(
            full_digest.as_str(),
            "sha256-2f3adcef67f09f6a99c349bfbb6ba429e4acd925d86f69bcc51322c096581c17"
        );
        assert_eq!(
            PodTemplateKey::from_digest("sha256:abc/def")
                .unwrap()
                .as_str(),
            key.as_str()
        );
    }
    #[test]
    fn pod_container_layout_uses_template_lower_and_container_overlay() {
        let store = mounted_store();
        let pod_id = PodId::new("pod-a").unwrap();
        let container_id = ContainerId::new("worker").unwrap();
        let template_key = PodTemplateKey::from_digest("sha256:manifest").unwrap();
        let template_rootfs = pod_template_rootfs_path(&store, &pod_id, &template_key).unwrap();
        let layout = PodContainerLayout::new(&store, &pod_id, &container_id).unwrap();
        let request = container_overlay_mount_request(&template_rootfs, &layout);
        assert_eq!(
            template_rootfs.as_str(),
            "/run/firkin/pod-store/pods/pod-a/templates/sha256-manifest/rootfs"
        );
        assert_eq!(
            layout.base.as_str(),
            "/run/firkin/pod-store/pods/pod-a/containers/worker"
        );
        assert_eq!(
            layout.upper.as_str(),
            "/run/firkin/pod-store/pods/pod-a/containers/worker/upper"
        );
        assert_eq!(
            layout.work.as_str(),
            "/run/firkin/pod-store/pods/pod-a/containers/worker/work"
        );
        assert_eq!(
            layout.merged.as_str(),
            "/run/firkin/pod-store/pods/pod-a/containers/worker/merged"
        );
        assert_eq!(request.r#type, "overlay");
        assert_eq!(request.source, "overlay");
        assert_eq!(request.destination, layout.merged.as_str());
        assert_eq!(
            request.options,
            [
                "lowerdir=/run/firkin/pod-store/pods/pod-a/templates/sha256-manifest/rootfs",
                "upperdir=/run/firkin/pod-store/pods/pod-a/containers/worker/upper",
                "workdir=/run/firkin/pod-store/pods/pod-a/containers/worker/work",
            ]
        );
    }
    #[test]
    fn pod_container_cleanup_requests_detached_unmount_and_recursive_remove() {
        let store = mounted_store();
        let pod_id = PodId::new("pod-a").unwrap();
        let container_id = ContainerId::new("worker").unwrap();
        let layout = PodContainerLayout::new(&store, &pod_id, &container_id).unwrap();
        let umount = container_overlay_umount_request(&layout);
        let remove = container_layout_remove_request(&layout);
        assert_eq!(
            umount.path,
            "/run/firkin/pod-store/pods/pod-a/containers/worker/merged"
        );
        assert_eq!(umount.flags, 2);
        assert_eq!(
            remove.path,
            "/run/firkin/pod-store/pods/pod-a/containers/worker"
        );
        assert!(remove.recursive);
        assert!(remove.allow_missing);
    }
    #[test]
    fn pod_apply_layer_request_targets_guest_archive_and_template_rootfs() {
        let archive =
            GuestPath::new("/run/firkin/pod-store/pods/pod-a/templates/t/layers/sha256-layer")
                .unwrap();
        let rootfs = GuestPath::new("/run/firkin/pod-store/pods/pod-a/templates/t/rootfs").unwrap();
        let request = apply_oci_layer_request(&archive, &rootfs);
        assert_eq!(
            request.archive_path,
            "/run/firkin/pod-store/pods/pod-a/templates/t/layers/sha256-layer"
        );
        assert_eq!(
            request.destination,
            "/run/firkin/pod-store/pods/pod-a/templates/t/rootfs"
        );
    }
}
pub mod exec;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use exec::*;
pub mod id;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use id::*;
pub mod layout;
#[allow(unused_imports)]
pub(crate) use layout::*;
pub mod materialize;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use materialize::*;
pub mod rootfs;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use rootfs::*;
pub mod spec;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use spec::*;
pub mod store;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use store::*;
pub mod tar_import;
#[allow(unused_imports)]
pub(crate) use tar_import::*;
