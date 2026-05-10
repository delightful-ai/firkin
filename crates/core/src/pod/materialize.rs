//! materialize — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::pod::copy_file_to_guest_path;
#[allow(unused_imports)]
use crate::pod::id::PodId;
#[allow(unused_imports)]
use crate::pod::layout::{pod_rootfs_layer_base_path, pod_rootfs_layer_path, pod_rootfs_path};
#[allow(unused_imports)]
use crate::pod::mkdir_guest_path;
#[allow(unused_imports)]
use crate::pod::prepare_pod_directories;
#[allow(unused_imports)]
use crate::pod::remove_guest_path;
#[allow(unused_imports)]
use crate::pod::rootfs::{MaterializedRootfs, apply_oci_layer_request};
#[allow(unused_imports)]
use crate::pod::spec::PodRootfsSource;
#[allow(unused_imports)]
use crate::pod::store::MountedPodStore;
#[allow(unused_imports)]
use crate::{Error, VminitdClient, runtime_rpc_error};
#[allow(unused_imports)]
use crate::{Result, connect_vminitd};
#[allow(unused_imports)]
use firkin_types::ContainerId;
#[allow(unused_imports)]
use firkin_vmm::{Running, VirtualMachine};
/// Materialize one pod container rootfs into a mounted pod store.
///
/// This public helper uses a default pod path segment for direct smoke tests
/// and low-level callers. First-class pods call the same implementation with
/// their own pod ID.
///
/// # Errors
///
/// Returns an error if the guest path cannot be prepared, if an OCI layer uses
/// unsupported rootfs-delta semantics, or if vminitd rejects the copy.
pub async fn materialize_rootfs_in_pod_store(
    vm: &VirtualMachine<Running>,
    pod_store: &MountedPodStore,
    container_id: &ContainerId,
    source: &PodRootfsSource,
) -> Result<MaterializedRootfs> {
    let mut client = connect_vminitd(vm).await?;
    let pod_id = PodId::new("default")?;
    prepare_pod_directories(&mut client, pod_store, &pod_id).await?;
    materialize_rootfs_in_pod_store_for_pod(
        vm,
        &mut client,
        pod_store,
        &pod_id,
        container_id,
        source,
    )
    .await
}
async fn materialize_rootfs_in_pod_store_for_pod(
    vm: &VirtualMachine<Running>,
    client: &mut VminitdClient,
    pod_store: &MountedPodStore,
    pod_id: &PodId,
    container_id: &ContainerId,
    source: &PodRootfsSource,
) -> Result<MaterializedRootfs> {
    prepare_pod_directories(client, pod_store, pod_id).await?;
    match source {
        PodRootfsSource::GuestPath(path) => Ok(MaterializedRootfs::new(path.clone(), None)),
        PodRootfsSource::Ext4Image(path) => Err(Error::RuntimeOperation {
            operation: "materialize ext4 pod rootfs",
            reason: format!(
                "{} requires a guest mount-and-copy helper; use an OCI bundle or GuestPath rootfs",
                path.display()
            ),
        }),
        PodRootfsSource::OciBundle(bundle) => {
            let rootfs_path = pod_rootfs_path(pod_store, pod_id, container_id)?;
            remove_guest_path(client, &rootfs_path, "remove existing pod container rootfs").await?;
            mkdir_guest_path(client, &rootfs_path, 0o755, "mkdir pod container rootfs").await?;
            let layer_dir = pod_rootfs_layer_base_path(pod_store, pod_id, container_id)?;
            remove_guest_path(client, &layer_dir, "remove existing pod rootfs layers").await?;
            mkdir_guest_path(
                client,
                &layer_dir,
                0o755,
                "mkdir pod rootfs layer directory",
            )
            .await?;
            for layer in bundle.layers() {
                let layer_path = pod_rootfs_layer_path(pod_store, pod_id, container_id, layer)?;
                copy_file_to_guest_path(vm, client, layer.path(), &layer_path).await?;
                client
                    .apply_oci_layer(tonic::Request::new(apply_oci_layer_request(
                        &layer_path,
                        &rootfs_path,
                    )))
                    .await
                    .map_err(runtime_rpc_error("apply pod rootfs layer"))?;
            }
            client
                .sync(tonic::Request::new(
                    firkin_vminitd_client::pb::SyncRequest {},
                ))
                .await
                .map_err(runtime_rpc_error("sync pod rootfs materialization"))?;
            Ok(MaterializedRootfs::new(
                rootfs_path,
                Some(bundle.digest().to_string()),
            ))
        }
    }
}
