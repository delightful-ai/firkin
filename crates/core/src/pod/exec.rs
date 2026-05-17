//! exec — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::pod::copy_file_to_guest_path;
#[allow(unused_imports)]
use crate::pod::id::{IntoPodId, PodId};
#[allow(unused_imports)]
use crate::pod::layout::{
    PodContainerLayout, PodTemplate, PodTemplateKey, pod_empty_dir_path, pod_template_base_path,
    pod_template_layer_path, pod_template_layers_base_path, pod_template_rootfs_path,
};
#[allow(unused_imports)]
use crate::pod::mkdir_guest_path;
#[allow(unused_imports)]
use crate::pod::mount_pod_store_with_client;
#[allow(unused_imports)]
use crate::pod::prepare_pod_directories;
#[allow(unused_imports)]
use crate::pod::remove_guest_path;
#[allow(unused_imports)]
use crate::pod::rootfs::{
    apply_oci_layer_request, container_layout_remove_request, container_overlay_mount_request,
    container_overlay_umount_request,
};
#[allow(unused_imports)]
use crate::pod::spec::{
    EmptyDirMedium, EmptyDirVolume, PodContainerSpec, PodRootfsSource,
    validate_container_empty_dir_mounts, validate_pod_spec,
};
#[allow(unused_imports)]
use crate::pod::store::{MountedPodStore, PodStoreSpec};
use crate::vm_attach::CoreContainerFactory as _;
#[allow(unused_imports)]
use crate::{
    Container, ContainerStartupTiming, Error, ExitStatus, GuestPath, IntoContainerId, Mount,
    Output, Streams, VmRootfs, VminitdClient, runtime_rpc_error,
};
#[allow(unused_imports)]
use crate::{
    Result, boot_vm_with_transient_network_retry, connect_vminitd, runtime_vmm_error,
    standard_guest_setup,
};
#[allow(unused_imports)]
use firkin_oci::ImageBundle;
#[allow(unused_imports)]
use firkin_types::ContainerId;
#[allow(unused_imports)]
use firkin_vminitd_client::{FilesystemUsage, Fstrim, pb};
#[allow(unused_imports)]
use firkin_vmm::VmConfig;
#[allow(unused_imports)]
use firkin_vmm::{Running, VirtualMachine};
#[allow(unused_imports)]
use std::collections::{HashMap, HashSet};
#[cfg(test)]
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::Duration;
use tokio::sync::Semaphore;
/// Builder for a VM-backed pod sandbox.
#[derive(Clone, Debug)]
pub struct PodBuilder {
    pub(crate) id: PodId,
    vm_config: VmConfig,
    pod_store: PodStoreSpec,
    empty_dirs: Vec<EmptyDirVolume>,
    containers: Vec<PodContainerSpec>,
}
impl PodBuilder {
    /// Construct a pod builder from a complete VM config and mounted pod-store
    /// declaration.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when `id` is not path-safe.
    pub fn new(id: impl IntoPodId, vm_config: VmConfig, pod_store: PodStoreSpec) -> Result<Self> {
        Ok(Self {
            id: id.into_pod_id()?,
            vm_config,
            pod_store,
            empty_dirs: Vec::new(),
            containers: Vec::new(),
        })
    }
    /// Return the pod ID.
    #[must_use]
    pub const fn id(&self) -> &PodId {
        &self.id
    }
    /// Return the predeclared pod-store mount.
    #[must_use]
    pub const fn pod_store(&self) -> &PodStoreSpec {
        &self.pod_store
    }
    /// Return declared pod `emptyDir` volumes.
    #[must_use]
    pub fn empty_dirs(&self) -> &[EmptyDirVolume] {
        &self.empty_dirs
    }
    /// Return initial container declarations.
    #[must_use]
    pub fn containers(&self) -> &[PodContainerSpec] {
        &self.containers
    }
    /// Add one pod-owned `emptyDir` volume.
    #[must_use]
    pub fn empty_dir(mut self, volume: EmptyDirVolume) -> Self {
        self.empty_dirs.push(volume);
        self
    }
    /// Add one initial container.
    #[must_use]
    pub fn container(mut self, container: PodContainerSpec) -> Self {
        self.containers.push(container);
        self
    }
    /// Validate pod declarations without booting a VM.
    ///
    /// # Errors
    ///
    /// Returns [`PodValidationError`] if a container or volume declaration is
    /// internally inconsistent.
    pub fn validate(&self) -> Result<()> {
        validate_pod_spec(&self.empty_dirs, &self.containers)
    }
    /// Boot the pod VM, mount the pod store, prepare emptyDirs, and start all
    /// initial containers.
    ///
    /// # Errors
    ///
    /// Returns an error if VM boot, vminitd setup, pod-store mounting,
    /// rootfs materialization, or container startup fails.
    pub async fn spawn(self) -> Result<Pod> {
        self.validate()?;
        let vm = boot_vm_with_transient_network_retry(self.vm_config)
            .await
            .map_err(runtime_vmm_error("boot pod VM"))?;
        let mut client = connect_vminitd(&vm).await?;
        standard_guest_setup(&mut client).await?;
        let store = mount_pod_store_with_client(&mut client, &self.pod_store).await?;
        prepare_pod_directories(&mut client, &store, &self.id).await?;
        prepare_empty_dirs(&mut client, &store, &self.id, &self.empty_dirs).await?;
        let mut pod = Pod {
            id: self.id,
            vm: Arc::new(vm),
            client,
            store,
            empty_dirs: self.empty_dirs,
            templates: HashMap::new(),
            container_layouts: HashMap::new(),
            containers: HashMap::new(),
            pending_containers: HashSet::new(),
            container_start_gate: Arc::new(Semaphore::new(POD_CONTAINER_START_PERMITS)),
        };
        for container in self.containers {
            if let Err(error) = pod.add_container(container).await {
                let _ = pod.stop().await;
                return Err(error);
            }
        }
        Ok(pod)
    }
}
/// Running VM-backed pod sandbox.
#[derive(Debug)]
pub struct Pod {
    pub(crate) id: PodId,
    vm: Arc<VirtualMachine<Running>>,
    client: VminitdClient,
    store: MountedPodStore,
    empty_dirs: Vec<EmptyDirVolume>,
    templates: HashMap<PodTemplateKey, PodTemplate>,
    container_layouts: HashMap<ContainerId, PodContainerLayout>,
    containers: HashMap<ContainerId, Container<Streams>>,
    pending_containers: HashSet<ContainerId>,
    container_start_gate: Arc<Semaphore>,
}
const POD_CONTAINER_START_PERMITS: usize = 1;
/// A pod container add operation reserved in pod state and ready to start.
#[derive(Debug)]
pub struct PendingPodContainerAdd {
    id: ContainerId,
    spec: PodContainerSpec,
    rootfs_path: GuestPath,
    overlay_lower: Option<GuestPath>,
    layout: Option<PodContainerLayout>,
    mounts: Vec<Mount>,
    vm: Arc<VirtualMachine<Running>>,
    cleanup_client: VminitdClient,
    container_start_gate: Arc<Semaphore>,
}
impl PendingPodContainerAdd {
    /// Return the reserved container ID.
    #[must_use]
    pub const fn id(&self) -> &ContainerId {
        &self.id
    }

    /// Prepare guest overlay state and start the container without borrowing
    /// the pod registry.
    ///
    /// # Errors
    ///
    /// Returns an error if guest overlay preparation or vminitd process start
    /// fails. Overlay state prepared by this method is cleaned before returning
    /// a failure.
    pub async fn start(self) -> Result<StartedPodContainerAdd> {
        self.prepare().await?.start().await
    }

    /// Prepare guest overlay state without starting the container process.
    ///
    /// # Errors
    ///
    /// Returns an error if guest overlay preparation fails. Partial overlay
    /// state prepared by this method is cleaned before returning a failure.
    pub async fn prepare(mut self) -> Result<PreparedPodContainerAdd> {
        if let Some(layout) = &self.layout {
            prepare_container_overlay_layout(&mut self.cleanup_client, layout).await?;
            let Some(lower) = self.overlay_lower.as_ref() else {
                return Err(Error::RuntimeOperation {
                    operation: "prepare pod container overlay",
                    reason: format!(
                        "container {} has overlay layout without lower rootfs",
                        self.id
                    ),
                });
            };
            if let Err(error) = self
                .cleanup_client
                .mount(tonic::Request::new(container_overlay_mount_request(
                    lower, layout,
                )))
                .await
                .map_err(runtime_rpc_error("mount pod container overlay rootfs"))
            {
                let _ =
                    cleanup_container_layout_with_client(&mut self.cleanup_client, layout).await;
                return Err(error);
            }
        }

        Ok(PreparedPodContainerAdd {
            id: self.id,
            spec: self.spec,
            rootfs_path: self.rootfs_path,
            layout: self.layout,
            mounts: self.mounts,
            vm: self.vm,
            cleanup_client: self.cleanup_client,
            container_start_gate: self.container_start_gate,
        })
    }
}
/// A pod container add operation with guest rootfs state prepared.
#[derive(Debug)]
pub struct PreparedPodContainerAdd {
    id: ContainerId,
    spec: PodContainerSpec,
    rootfs_path: GuestPath,
    layout: Option<PodContainerLayout>,
    mounts: Vec<Mount>,
    vm: Arc<VirtualMachine<Running>>,
    cleanup_client: VminitdClient,
    container_start_gate: Arc<Semaphore>,
}
impl PreparedPodContainerAdd {
    /// Return the reserved container ID.
    #[must_use]
    pub const fn id(&self) -> &ContainerId {
        &self.id
    }

    /// Start the prepared pod container process.
    ///
    /// # Errors
    ///
    /// Returns an error if vminitd process start fails. Prepared overlay state
    /// is cleaned before returning a failure.
    pub async fn start(mut self) -> Result<StartedPodContainerAdd> {
        let mut builder = self
            .vm
            .container_shared(self.id.clone())?
            .stdin(self.spec.stdin)
            .stdout(self.spec.stdout)
            .stderr(self.spec.stderr);
        if let PodRootfsSource::OciBundle(bundle) = &self.spec.rootfs {
            builder = builder.image_config(bundle.config());
        }
        if !self.spec.command.is_empty() {
            builder = builder.command(self.spec.command);
        }
        if !self.spec.env.is_empty() {
            builder = builder.envs(self.spec.env);
        }
        for mount in self.mounts {
            builder = builder.mount(mount);
        }
        let container = match builder
            .rootfs(VmRootfs::guest_path(self.rootfs_path.clone()))
            .spawn_prepared_guest_path_with_start_gate(Arc::clone(&self.container_start_gate))
            .await
        {
            Ok(container) => container,
            Err(error) => {
                if let Some(layout) = &self.layout {
                    let _ = cleanup_container_layout_with_client(&mut self.cleanup_client, layout)
                        .await;
                }
                return Err(error);
            }
        };

        Ok(StartedPodContainerAdd {
            id: self.id,
            container,
            layout: self.layout,
        })
    }
}
/// A started pod container waiting to be committed into pod registry state.
#[derive(Debug)]
pub struct StartedPodContainerAdd {
    id: ContainerId,
    container: Container<Streams>,
    layout: Option<PodContainerLayout>,
}
impl StartedPodContainerAdd {
    /// Return the started container ID.
    #[must_use]
    pub const fn id(&self) -> &ContainerId {
        &self.id
    }

    /// Return host-side phase timing for the started container init process.
    #[must_use]
    pub const fn startup_timing(&self) -> ContainerStartupTiming {
        self.container.startup_timing()
    }
}
/// A validated pod `emptyDir` write that can run without borrowing pod state.
#[derive(Debug)]
pub struct PendingPodEmptyDirFileWrite {
    destination: GuestPath,
    client: VminitdClient,
}
impl PendingPodEmptyDirFileWrite {
    /// Write bytes to the validated destination.
    ///
    /// # Errors
    ///
    /// Returns an error if vminitd rejects the guest write.
    pub async fn write(mut self, data: Vec<u8>) -> Result<GuestPath> {
        self.client
            .write_file(tonic::Request::new(pb::WriteFileRequest {
                path: self.destination.as_str().to_owned(),
                data,
                mode: 0o644,
                flags: Some(pb::write_file_request::WriteFileFlags {
                    create_parent_dirs: true,
                    append: false,
                    create_if_missing: true,
                }),
            }))
            .await
            .map_err(runtime_rpc_error("write pod emptyDir file"))?;
        Ok(self.destination)
    }
}
/// A pod container that has been detached from the pod registry for waiting.
#[derive(Debug)]
pub struct DetachedPodContainer {
    id: ContainerId,
    container: Container<Streams>,
    layout: Option<PodContainerLayout>,
    cleanup_client: VminitdClient,
}
impl DetachedPodContainer {
    /// Return the detached container ID.
    #[must_use]
    pub const fn id(&self) -> &ContainerId {
        &self.id
    }

    /// Wait for the detached container, collect stdout/stderr, and clean its
    /// pod-owned rootfs state.
    ///
    /// # Errors
    ///
    /// Returns an error if output collection, process wait, or guest cleanup
    /// fails.
    pub async fn wait_with_output(mut self) -> Result<Output> {
        let output = self.container.wait_with_output().await?;
        if let Some(layout) = self.layout.take() {
            cleanup_container_layout_with_client(&mut self.cleanup_client, &layout).await?;
        }
        Ok(output)
    }
}
impl Pod {
    /// Return the pod ID.
    #[must_use]
    pub const fn id(&self) -> &PodId {
        &self.id
    }
    /// Return the mounted pod store.
    #[must_use]
    pub const fn store(&self) -> &MountedPodStore {
        &self.store
    }
    /// Return the running VM backing this pod.
    #[must_use]
    pub fn vm(&self) -> &VirtualMachine<Running> {
        self.vm.as_ref()
    }
    /// Return whether a container ID is currently tracked by this pod.
    #[must_use]
    pub fn contains_container(&self, id: &ContainerId) -> bool {
        self.containers.contains_key(id)
    }
    /// Return a mutable handle to a tracked container.
    pub fn container_mut(&mut self, id: &ContainerId) -> Option<&mut Container<Streams>> {
        self.containers.get_mut(id)
    }
    /// Write one file into a pod-owned `emptyDir` volume.
    ///
    /// # Errors
    ///
    /// Returns an error if the volume is unknown, the relative path escapes the
    /// volume, or vminitd rejects the guest write.
    pub async fn write_empty_dir_file(
        &mut self,
        volume_name: impl IntoContainerId,
        relative_path: &str,
        data: Vec<u8>,
    ) -> Result<GuestPath> {
        self.begin_empty_dir_file_write(volume_name, relative_path)?
            .write(data)
            .await
    }
    /// Validate one pod-owned `emptyDir` write and detach it from pod state.
    ///
    /// # Errors
    ///
    /// Returns an error if the volume is unknown or the relative path escapes
    /// the volume.
    pub fn begin_empty_dir_file_write(
        &self,
        volume_name: impl IntoContainerId,
        relative_path: &str,
    ) -> Result<PendingPodEmptyDirFileWrite> {
        let volume_name = volume_name.into_container_id()?;
        if !self
            .empty_dirs
            .iter()
            .any(|volume| volume.name == volume_name)
        {
            return Err(Error::RuntimeOperation {
                operation: "write pod emptyDir file",
                reason: format!(
                    "emptyDir volume {volume_name} is not declared on pod {}",
                    self.id
                ),
            });
        }
        let destination = empty_dir_file_path(&self.store, &self.id, &volume_name, relative_path)?;
        Ok(PendingPodEmptyDirFileWrite {
            destination,
            client: self.client.clone(),
        })
    }
    /// Add and start one container inside this already-running pod VM.
    ///
    /// # Errors
    ///
    /// Returns an error if the container already exists, rootfs materialization
    /// fails, an emptyDir mount is unknown, or vminitd rejects process startup.
    pub async fn add_container(&mut self, spec: PodContainerSpec) -> Result<()> {
        let pending = self.begin_container_add(spec).await?;
        let id = pending.id().clone();
        match pending.start().await {
            Ok(started) => self.commit_container_add(started),
            Err(error) => {
                self.abort_container_add(&id);
                Err(error)
            }
        }
    }
    /// Reserve a container add operation so guest preparation and process start
    /// can proceed outside the pod registry borrow.
    ///
    /// # Errors
    ///
    /// Returns an error if the container already exists or is already pending,
    /// if rootfs planning fails, or if emptyDir mounts are invalid.
    pub async fn begin_container_add(
        &mut self,
        spec: PodContainerSpec,
    ) -> Result<PendingPodContainerAdd> {
        if self.containers.contains_key(&spec.id) || self.pending_containers.contains(&spec.id) {
            return Err(Error::RuntimeOperation {
                operation: "add pod container",
                reason: format!("container {} already exists", spec.id),
            });
        }
        validate_container_empty_dir_mounts(&self.empty_dirs, &spec)?;
        let mounts = self.pod_container_mounts(&spec)?;
        let id = spec.id.clone();
        let (rootfs_path, overlay_lower, layout) = match &spec.rootfs {
            PodRootfsSource::GuestPath(path) => (path.clone(), None, None),
            PodRootfsSource::Ext4Image(path) => {
                return Err(Error::RuntimeOperation {
                    operation: "materialize ext4 pod rootfs",
                    reason: format!(
                        "{} requires a guest mount-and-copy helper; use an OCI bundle or GuestPath rootfs",
                        path.display()
                    ),
                });
            }
            PodRootfsSource::OciBundle(bundle) => {
                let template = self.ensure_oci_template(bundle).await?;
                let layout = PodContainerLayout::new(&self.store, &self.id, &id)?;
                (layout.merged.clone(), Some(template.rootfs), Some(layout))
            }
        };
        self.pending_containers.insert(id.clone());
        Ok(PendingPodContainerAdd {
            id,
            spec,
            rootfs_path,
            overlay_lower,
            layout,
            mounts,
            vm: Arc::clone(&self.vm),
            cleanup_client: self.client.clone(),
            container_start_gate: Arc::clone(&self.container_start_gate),
        })
    }
    /// Commit a successfully started pending pod container into pod state.
    ///
    /// # Errors
    ///
    /// Returns an error if the started container was not reserved by this pod.
    pub fn commit_container_add(&mut self, started: StartedPodContainerAdd) -> Result<()> {
        if !self.pending_containers.remove(started.id()) {
            return Err(Error::RuntimeOperation {
                operation: "commit pod container add",
                reason: format!("container {} is not pending on pod {}", started.id, self.id),
            });
        }
        if let Some(layout) = started.layout {
            self.container_layouts.insert(started.id.clone(), layout);
        }
        self.containers.insert(started.id, started.container);
        Ok(())
    }
    /// Release a pending add reservation after a detached start failure.
    pub fn abort_container_add(&mut self, id: &ContainerId) {
        self.pending_containers.remove(id);
    }
    /// Wait for a container and collect stdout/stderr.
    ///
    /// # Errors
    ///
    /// Returns an error if the container is unknown or wait/output collection
    /// fails.
    pub async fn wait_container(&mut self, id: impl IntoContainerId) -> Result<Output> {
        let detached = self.detach_container_for_wait(id)?;
        detached.wait_with_output().await
    }
    /// Detach a tracked container from this pod so its process can be awaited
    /// without holding the pod state lock.
    ///
    /// # Errors
    ///
    /// Returns an error if the container is unknown.
    pub fn detach_container_for_wait(
        &mut self,
        id: impl IntoContainerId,
    ) -> Result<DetachedPodContainer> {
        let id = id.into_container_id()?;
        let container = self
            .containers
            .remove(&id)
            .ok_or_else(|| Error::RuntimeOperation {
                operation: "detach pod container for wait",
                reason: format!("container {id} is not tracked by pod {}", self.id),
            })?;
        let layout = self.container_layouts.remove(&id);
        Ok(DetachedPodContainer {
            id,
            container,
            layout,
            cleanup_client: self.client.clone(),
        })
    }
    /// Stop and remove one tracked container.
    ///
    /// # Errors
    ///
    /// Returns an error if the container is unknown or vminitd rejects stop.
    pub async fn remove_container(&mut self, id: impl IntoContainerId) -> Result<ExitStatus> {
        let id = id.into_container_id()?;
        let container = self
            .containers
            .get_mut(&id)
            .ok_or_else(|| Error::RuntimeOperation {
                operation: "remove pod container",
                reason: format!("container {id} is not tracked by pod {}", self.id),
            })?;
        let status = stop_container_with_grace(container, Duration::from_millis(250)).await?;
        self.cleanup_container_layout_for_id(&id).await?;
        self.containers.remove(&id);
        Ok(status)
    }
    /// Trim the mounted pod-store filesystem and return bytes reported by the
    /// guest kernel as discarded.
    ///
    /// # Errors
    ///
    /// Returns an error if vminitd rejects `fstrim` for the pod-store mount.
    pub async fn trim_store(&mut self) -> Result<u64> {
        let response = self
            .client
            .fstrim(tonic::Request::new(
                Fstrim::new(self.store.guest_mount().as_str()).into_request(),
            ))
            .await
            .map_err(runtime_rpc_error("trim pod store"))?
            .into_inner();
        Ok(response.trimmed_bytes)
    }
    /// Return bytes currently used on the mounted pod-store filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error if vminitd cannot stat the pod-store filesystem.
    pub async fn store_used_bytes(&mut self) -> Result<u64> {
        let response = self
            .client
            .filesystem_usage(tonic::Request::new(
                FilesystemUsage::new(self.store.guest_mount().as_str()).into_request(),
            ))
            .await
            .map_err(runtime_rpc_error("read pod-store filesystem usage"))?
            .into_inner();
        Ok(response
            .total_blocks
            .saturating_sub(response.free_blocks)
            .saturating_mul(response.block_size))
    }
    /// Return the number of OCI rootfs templates materialized in the pod store.
    #[must_use]
    pub fn template_cache_entries(&self) -> usize {
        self.templates.len()
    }
    /// Stop all remaining containers and then stop the pod VM.
    ///
    /// # Errors
    ///
    /// Returns an error if VM stop fails. Individual remaining container stop
    /// errors are ignored because the VM teardown is authoritative.
    pub async fn stop(mut self) -> Result<()> {
        self.shutdown().await
    }
    /// Stop all remaining containers and then stop the pod VM without consuming
    /// the pod handle until teardown succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error if VM stop fails. Individual remaining container stop
    /// errors are ignored because the VM teardown is authoritative.
    pub async fn shutdown(&mut self) -> Result<()> {
        self.stop_containers_and_cleanup().await;
        self.stop_vm().await
    }
    /// Stop all remaining containers, clean their overlay state, trim the pod
    /// store, and then stop the pod VM.
    ///
    /// # Errors
    ///
    /// Returns an error if VM stop fails. Guest fstrim is best-effort so reclaim
    /// failure cannot strand a stopped product pod in the runtime registry.
    pub async fn shutdown_trimming_store(&mut self) -> Result<()> {
        self.stop_containers_and_cleanup().await;
        let _ = self.trim_store().await;
        self.stop_vm().await
    }
    async fn stop_containers_and_cleanup(&mut self) {
        for (_, container) in self.containers.drain() {
            let _ = container.stop_with_grace(Duration::from_millis(250)).await;
        }
        let layouts: Vec<_> = self
            .container_layouts
            .drain()
            .map(|(_, layout)| layout)
            .collect();
        for layout in layouts {
            let _ = cleanup_container_layout_with_client(&mut self.client, &layout).await;
        }
    }
    async fn stop_vm(&self) -> Result<()> {
        self.vm
            .as_ref()
            .clone()
            .stop_with_grace(Duration::from_millis(250))
            .await
            .map_err(runtime_vmm_error("stop pod VM"))
    }
    fn pod_container_mounts(&self, spec: &PodContainerSpec) -> Result<Vec<Mount>> {
        let mut mounts = spec.mounts.clone();
        for empty_dir in &spec.empty_dir_mounts {
            let volume = self
                .empty_dirs
                .iter()
                .find(|volume| volume.name == empty_dir.volume_name)
                .ok_or_else(|| Error::RuntimeOperation {
                    operation: "build pod emptyDir mounts",
                    reason: format!(
                        "container {} references unknown emptyDir {}",
                        spec.id, empty_dir.volume_name
                    ),
                })?;
            let destination =
                empty_dir
                    .container_path
                    .to_str()
                    .ok_or_else(|| Error::RuntimeOperation {
                        operation: "build pod emptyDir mounts",
                        reason: format!(
                            "emptyDir destination {} is not valid UTF-8",
                            empty_dir.container_path.display()
                        ),
                    })?;
            let source = pod_empty_dir_path(&self.store, &self.id, &volume.name)?;
            let mut mount = Mount::bind(source.as_str(), destination);
            if empty_dir.read_only {
                mount = mount.read_only();
            }
            mounts.push(mount);
        }
        Ok(mounts)
    }
    async fn ensure_oci_template(&mut self, bundle: &ImageBundle) -> Result<PodTemplate> {
        let key = PodTemplateKey::from_digest(bundle.digest().as_str())?;
        if let Some(template) = self.templates.get(&key) {
            return Ok(template.clone());
        }
        let template = materialize_oci_template(
            self.vm.as_ref(),
            &mut self.client,
            &self.store,
            &self.id,
            &key,
            bundle,
        )
        .await?;
        self.templates.insert(key, template.clone());
        Ok(template)
    }
    async fn cleanup_container_layout_for_id(&mut self, id: &ContainerId) -> Result<()> {
        if let Some(layout) = self.container_layouts.get(id).cloned() {
            cleanup_container_layout_with_client(&mut self.client, &layout).await?;
            self.container_layouts.remove(id);
        }
        Ok(())
    }
}
async fn prepare_empty_dirs(
    client: &mut VminitdClient,
    store: &MountedPodStore,
    pod_id: &PodId,
    volumes: &[EmptyDirVolume],
) -> Result<()> {
    for volume in volumes {
        let path = pod_empty_dir_path(store, pod_id, &volume.name)?;
        mkdir_guest_path(client, &path, 0o777, "mkdir pod emptyDir").await?;
        if volume.medium == EmptyDirMedium::Memory {
            let mut options = vec!["rw".to_owned()];
            if let Some(size) = volume.size_limit {
                options.push(format!("size={}", size.as_bytes()));
            }
            client
                .mount(tonic::Request::new(
                    firkin_vminitd_client::pb::MountRequest {
                        r#type: "tmpfs".to_owned(),
                        source: "tmpfs".to_owned(),
                        destination: path.as_str().to_owned(),
                        options,
                    },
                ))
                .await
                .map_err(runtime_rpc_error("mount pod memory emptyDir"))?;
        }
    }
    Ok(())
}
async fn materialize_oci_template(
    vm: &VirtualMachine<Running>,
    client: &mut VminitdClient,
    store: &MountedPodStore,
    pod_id: &PodId,
    key: &PodTemplateKey,
    bundle: &ImageBundle,
) -> Result<PodTemplate> {
    let template_base = pod_template_base_path(store, pod_id, key)?;
    remove_guest_path(client, &template_base, "remove existing pod template").await?;
    let rootfs = pod_template_rootfs_path(store, pod_id, key)?;
    let layer_dir = pod_template_layers_base_path(store, pod_id, key)?;
    mkdir_guest_path(client, &rootfs, 0o755, "mkdir pod template rootfs").await?;
    mkdir_guest_path(
        client,
        &layer_dir,
        0o755,
        "mkdir pod template layer directory",
    )
    .await?;
    for layer in bundle.layers() {
        let layer_path = pod_template_layer_path(store, pod_id, key, layer)?;
        copy_file_to_guest_path(vm, client, layer.path(), &layer_path).await?;
        client
            .apply_oci_layer(tonic::Request::new(apply_oci_layer_request(
                &layer_path,
                &rootfs,
            )))
            .await
            .map_err(runtime_rpc_error("apply pod template layer"))?;
    }
    client
        .sync(tonic::Request::new(
            firkin_vminitd_client::pb::SyncRequest {},
        ))
        .await
        .map_err(runtime_rpc_error("sync pod template materialization"))?;
    Ok(PodTemplate::new(rootfs))
}
async fn prepare_container_overlay_layout(
    client: &mut VminitdClient,
    layout: &PodContainerLayout,
) -> Result<()> {
    for path in [&layout.upper, &layout.work, &layout.merged] {
        mkdir_guest_path(client, path, 0o755, "mkdir pod container overlay path").await?;
    }
    Ok(())
}
fn empty_dir_file_path(
    store: &MountedPodStore,
    pod_id: &PodId,
    volume_name: &ContainerId,
    relative_path: &str,
) -> Result<GuestPath> {
    validate_empty_dir_relative_path(relative_path)?;
    let base = pod_empty_dir_path(store, pod_id, volume_name)?;
    GuestPath::new(format!("{}/{relative_path}", base.as_str()))
}
fn validate_empty_dir_relative_path(relative_path: &str) -> Result<()> {
    let invalid_reason = if relative_path.is_empty() {
        Some("path cannot be empty")
    } else if relative_path.contains('\0') {
        Some("path cannot contain NUL")
    } else if relative_path.starts_with('/') {
        Some("path must be relative")
    } else if relative_path
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        Some("path must be normalized")
    } else {
        None
    };
    if let Some(reason) = invalid_reason {
        return Err(Error::RuntimeOperation {
            operation: "build pod emptyDir file path",
            reason: format!("relative path `{relative_path}` is invalid: {reason}"),
        });
    }
    Ok(())
}
async fn cleanup_container_layout_with_client(
    client: &mut VminitdClient,
    layout: &PodContainerLayout,
) -> Result<()> {
    let _ = client
        .umount(tonic::Request::new(container_overlay_umount_request(
            layout,
        )))
        .await
        .map_err(runtime_rpc_error("umount pod container overlay rootfs"));
    client
        .remove_path(tonic::Request::new(container_layout_remove_request(layout)))
        .await
        .map_err(runtime_rpc_error("remove pod container overlay state"))?;
    Ok(())
}
async fn stop_container_with_grace(
    container: &mut Container,
    grace: Duration,
) -> Result<ExitStatus> {
    let mut runtime = container.runtime.lock().await;
    runtime.stop_with_grace(grace).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use firkin_types::BlockDeviceId;
    use std::num::NonZeroU32;

    fn mounted_store() -> MountedPodStore {
        MountedPodStore {
            spec: PodStoreSpec::ext4(BlockDeviceId::from_slot(
                NonZeroU32::new(1).expect("nonzero slot"),
            )),
        }
    }

    #[test]
    fn empty_dir_file_path_accepts_normal_relative_paths() {
        let store = mounted_store();
        let pod_id = PodId::new("pod-1").expect("pod id");
        let volume_name = ContainerId::new("db").expect("volume name");

        let path = empty_dir_file_path(&store, &pod_id, &volume_name, "requests/slot-1")
            .expect("emptyDir file path");

        assert_eq!(
            path.as_str(),
            "/run/firkin/pod-store/pods/pod-1/emptydir/db/requests/slot-1"
        );
    }

    #[test]
    fn empty_dir_file_path_rejects_escaping_paths() {
        for relative_path in [
            "",
            "/request",
            "../request",
            "request/../slot",
            "request//slot",
        ] {
            let error = validate_empty_dir_relative_path(relative_path).expect_err("reject path");
            assert!(
                error.to_string().contains("relative path"),
                "unexpected error for {relative_path:?}: {error}"
            );
        }
    }
}
