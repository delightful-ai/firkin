use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

use async_trait::async_trait;
use firkin_core::{
    Container, EmptyDirVolume, ExecConfig, Pod, PodBuilder, PodContainerSpec, PodRootfsSource,
    PodStoreSpec, Process, ProcessKillHandle, PtyConfig, Rootfs, Signal, Stdio, User,
};
use firkin_ext4::Writer;
use firkin_types::{BlockDeviceId, SandboxNetworkPolicy, Size};
use firkin_vmm::{DiskImageConversion, KernelImage, VmConfig, VmConfigBuilder, convert_disk_image};
use time::format_description::well_known::Rfc3339;
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use {
    firkin_e2b_contract::{
        FollowupSnapshot, PausedSandbox, PodRuntimeConfig, PortTarget, RuntimeAdapter,
        RuntimeCapabilitySet, RuntimePod, RuntimeSandbox, SandboxRuntimeConfig, SnapshotRef,
        StartPodRequest, StartSandboxRequest,
    },
    firkin_e2b_server::EnvdProcessHttpServer,
    firkin_e2b_wire::{
        PodContainerCreateRequest, PodContainerInfo, PodContainerOutput, PodStoreImageFormat,
        PodStoreOptions, PodTrimPolicy, SandboxLogs, SandboxMetric, TemplateBuildRequest,
    },
    firkin_envd::{
        EnvdFilesystemAdapter, EnvdFilesystemEntry, EnvdFilesystemEvent, EnvdFilesystemFileType,
        EnvdFilesystemWriteInfo, EnvdProcessAdapter, EnvdProcessEventStream, EnvdProcessInfo,
        EnvdProcessInput, EnvdProcessOutput, EnvdProcessSelector, EnvdProcessSignal,
        EnvdProcessStartRequest, EnvdProcessStreamEvent, EnvdPtySize,
    },
};

const E2B_ENVD_PORT: u16 = 49983;
type E2bBackendError = firkin_e2b_contract::BackendError;
type E2bResult<T> = std::result::Result<T, E2bBackendError>;

use super::{
    CommandOutput, CommandRequest, Error, LogStore, PortRegistry, Result, RuntimeCreatedSandbox,
    RuntimeDriver, RuntimeSnapshotRef, SandboxResources, SingleNodeCreateRequest,
    SingleNodeRuntimeMode, SnapshotRecord,
};

/// Apple/VZ-backed local runtime driver for [`super::SingleNodeBackend`].
#[derive(Clone)]
pub struct AppleVzLocalRuntimeDriver {
    template_image: String,
    ports: PortRegistry,
    logs: LogStore,
    snapshot_dir: Arc<PathBuf>,
    sandboxes: Arc<Mutex<HashMap<String, AppleVzRuntimeSandbox>>>,
    pods: Arc<Mutex<HashMap<String, Arc<Mutex<AppleVzRuntimePod>>>>>,
    next_adapter_sandbox: Arc<AtomicU64>,
    next_pod: Arc<AtomicU64>,
}

struct AppleVzRuntimeSandbox {
    container: Arc<Mutex<Container>>,
    process_adapter: AppleVzEnvdAdapter,
    envd_task: Option<JoinHandle<()>>,
    start_processes: Vec<Process>,
}

struct AppleVzRuntimePod {
    pod: Pod,
    staging_dir: PathBuf,
    pod_store_path: PathBuf,
    trim_policy: PodTrimPolicy,
    templates: HashMap<String, firkin_oci::ImageBundle>,
}

#[cfg_attr(any(not(test), not(feature = "snapshot")), allow(dead_code))]
struct AppleVzRestoreRootfsPlan {
    source_rootfs: PathBuf,
    staging_dir: PathBuf,
    restored_rootfs: PathBuf,
}

impl AppleVzLocalRuntimeDriver {
    /// Construct a local Apple/VZ driver with default in-memory proxy/log state.
    #[must_use]
    pub fn new(template_image: impl Into<String>) -> Self {
        Self::with_port_registry(template_image, PortRegistry::default())
    }

    /// Construct a local Apple/VZ driver with an injected port registry.
    #[must_use]
    pub fn with_port_registry(template_image: impl Into<String>, ports: PortRegistry) -> Self {
        Self::with_port_registry_and_log_store(template_image, ports, LogStore::default())
    }

    /// Construct a local Apple/VZ driver with injected port and log stores.
    #[must_use]
    pub fn with_port_registry_and_log_store(
        template_image: impl Into<String>,
        ports: PortRegistry,
        logs: LogStore,
    ) -> Self {
        Self::with_snapshot_dir(
            template_image,
            ports,
            logs,
            firkin_runtime::default_single_node_snapshot_root(),
        )
    }

    /// Construct a local Apple/VZ driver with an explicit snapshot directory.
    #[must_use]
    pub fn with_snapshot_dir(
        template_image: impl Into<String>,
        ports: PortRegistry,
        logs: LogStore,
        snapshot_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            template_image: template_image.into(),
            ports,
            logs,
            snapshot_dir: Arc::new(snapshot_dir.into()),
            sandboxes: Arc::new(Mutex::new(HashMap::new())),
            pods: Arc::new(Mutex::new(HashMap::new())),
            next_adapter_sandbox: Arc::new(AtomicU64::new(0)),
            next_pod: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn pod_handle(&self, pod_id: &str) -> Result<Arc<Mutex<AppleVzRuntimePod>>> {
        self.pods
            .lock()
            .await
            .get(pod_id)
            .cloned()
            .ok_or_else(|| Error::SandboxNotFound(format!("pod {pod_id} not found")))
    }

    async fn pod_handle_for_adapter(
        &self,
        pod_id: &str,
    ) -> std::result::Result<Arc<Mutex<AppleVzRuntimePod>>, E2bBackendError> {
        self.pods
            .lock()
            .await
            .get(pod_id)
            .cloned()
            .ok_or_else(|| E2bBackendError::NotFound(pod_id.to_owned()))
    }

    /// Return bytes used inside a running product pod's guest pod-store
    /// filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error if the pod is absent or the guest rejects the
    /// filesystem usage request.
    pub async fn pod_store_used_bytes(&self, pod_id: &str) -> Result<u64> {
        let pod = self.pod_handle(pod_id).await?;
        let mut runtime_pod = pod.lock().await;
        runtime_pod
            .pod
            .store_used_bytes()
            .await
            .map_err(|error| runtime_error("read pod-store used bytes", error))
    }

    /// Return host-allocated bytes for a running product pod's raw pod-store
    /// disk image.
    ///
    /// # Errors
    ///
    /// Returns an error if the pod is absent or host metadata cannot be read.
    pub async fn pod_store_host_allocated_bytes(&self, pod_id: &str) -> Result<u64> {
        let pod = self.pod_handle(pod_id).await?;
        let runtime_pod = pod.lock().await;
        host_allocated_bytes(&runtime_pod.pod_store_path)
            .map_err(|error| runtime_error("read pod-store host allocation", error))
    }

    /// Run guest fstrim for a running product pod's mounted pod store and
    /// return bytes reported by the guest kernel as discarded.
    ///
    /// # Errors
    ///
    /// Returns an error if the pod is absent or the guest rejects fstrim.
    pub async fn trim_pod_store(&self, pod_id: &str) -> Result<u64> {
        let pod = self.pod_handle(pod_id).await?;
        let mut runtime_pod = pod.lock().await;
        runtime_pod
            .pod
            .trim_store()
            .await
            .map_err(|error| runtime_error("trim pod store", error))
    }

    /// Write a small control file into a running product pod's declared
    /// `emptyDir` volume.
    ///
    /// This is an evidence hook for prestarted agent-slot benchmarks. Product
    /// semantics stay in the caller; the driver only locates the pod and asks
    /// core to write bytes into pod-owned storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the pod is absent, the volume or relative path is
    /// invalid, or the guest rejects the file write.
    pub async fn write_pod_empty_dir_file(
        &self,
        pod_id: &str,
        volume_name: &str,
        relative_path: &str,
        data: Vec<u8>,
    ) -> Result<String> {
        let pod = self.pod_handle(pod_id).await?;
        let pending_write = {
            let runtime_pod = pod.lock().await;
            runtime_pod
                .pod
                .begin_empty_dir_file_write(volume_name, relative_path)
                .map_err(|error| runtime_error("prepare pod emptyDir file write", error))?
        };
        pending_write
            .write(data)
            .await
            .map(|path| path.to_string())
            .map_err(|error| runtime_error("write pod emptyDir file", error))
    }

    /// Return the number of product pod OCI rootfs templates materialized in
    /// the guest pod store.
    ///
    /// This is an evidence hook for the signed autoscale benchmark; it proves
    /// repeated containers used one shared rootfs template.
    ///
    /// # Errors
    ///
    /// Returns an error if the pod is absent.
    pub async fn pod_template_cache_entries(&self, pod_id: &str) -> Result<usize> {
        let pod = self.pod_handle(pod_id).await?;
        let runtime_pod = pod.lock().await;
        Ok(runtime_pod.pod.template_cache_entries())
    }

    fn template_ref(&self, template_id: &str) -> Result<String> {
        if template_id.contains('/') || template_id.contains(':') {
            Ok(template_id.to_owned())
        } else if template_id == "base" || template_id == "mcp-gateway" {
            Ok(self.template_image.clone())
        } else {
            Err(Error::SnapshotNotFound(format!(
                "firkin template {template_id} not found"
            )))
        }
    }

    fn snapshot_path(&self, snapshot_id: &str) -> Result<PathBuf> {
        if snapshot_id.is_empty()
            || snapshot_id == "."
            || snapshot_id == ".."
            || snapshot_id.contains('/')
            || snapshot_id.contains('\\')
        {
            return Err(Error::InvalidRequest(format!(
                "invalid firkin snapshot id `{snapshot_id}`"
            )));
        }
        Ok(self.snapshot_dir.join(format!("{snapshot_id}.vzstate")))
    }

    fn runtime_staging_dir(&self, sandbox_id: &str) -> PathBuf {
        self.snapshot_dir
            .parent()
            .unwrap_or_else(|| self.snapshot_dir.as_ref())
            .join("runtime")
            .join(sandbox_id)
    }

    fn next_adapter_sandbox_id(&self) -> String {
        let next = self
            .next_adapter_sandbox
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        format!("sbx_avz_{next}")
    }

    fn next_pod_id(&self) -> String {
        let next = self
            .next_pod
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        format!("pod_avz_{next}")
    }

    fn timestamps(timeout_seconds: Option<u64>) -> Result<(String, String)> {
        let started_at = OffsetDateTime::now_utc();
        let timeout = i64::try_from(timeout_seconds.unwrap_or(300))
            .map_err(|_| Error::InvalidRequest("timeout is too large".to_owned()))?;
        let end_at = started_at + TimeDuration::seconds(timeout);
        Ok((
            started_at
                .format(&Rfc3339)
                .expect("RFC3339 formatting current UTC time is infallible"),
            end_at
                .format(&Rfc3339)
                .expect("RFC3339 formatting current UTC time is infallible"),
        ))
    }

    fn write_empty_pod_store(path: &PathBuf, size: Size) -> Result<()> {
        Writer::new(path, size)
            .map_err(|error| runtime_error("create pod-store ext4 writer", error))?
            .write_dir("/run", 0o755)
            .map_err(|error| runtime_error("write pod-store /run", error))?
            .write_dir("/run/firkin", 0o755)
            .map_err(|error| runtime_error("write pod-store /run/firkin", error))?
            .finalize()
            .map(|_| ())
            .map_err(|error| runtime_error("finalize pod-store ext4", error))
    }

    fn prepare_product_pod_store(options: &PodStoreOptions, staging_dir: &Path) -> Result<PathBuf> {
        if !options.shared_rootfs {
            return Err(Error::UnsupportedCapability(
                "Apple/VZ product pods require sharedRootfs=true".to_owned(),
            ));
        }
        let size = Size::bytes(options.size_bytes);
        match options.image_format {
            PodStoreImageFormat::Raw => {
                let path = staging_dir.join("pod-store.ext4");
                Self::write_empty_pod_store(&path, size)?;
                Ok(path)
            }
            PodStoreImageFormat::Asif => {
                let raw_path = staging_dir.join("pod-store.raw.ext4");
                let asif_path = staging_dir.join("pod-store.asif");
                Self::write_empty_pod_store(&raw_path, size)?;
                convert_disk_image(&DiskImageConversion::asif(&raw_path, &asif_path))
                    .map_err(|error| runtime_error("convert pod-store ext4 to ASIF", error))?;
                fs::remove_file(&raw_path)
                    .map_err(|error| runtime_error("remove intermediate raw pod-store", error))?;
                Ok(asif_path)
            }
        }
    }

    fn add_product_pod_store_device(
        vm_builder: VmConfigBuilder,
        options: &PodStoreOptions,
        pod_store_path: &Path,
    ) -> (VmConfigBuilder, BlockDeviceId) {
        match options.image_format {
            PodStoreImageFormat::Raw => vm_builder.block_device(pod_store_path),
            PodStoreImageFormat::Asif => vm_builder.asif_disk_image(pod_store_path),
        }
    }

    fn ensure_supported_runtime_mode(request: &SingleNodeCreateRequest) {
        match request.runtime_mode() {
            SingleNodeRuntimeMode::SingleVmBackedContainer => {}
        }
    }

    async fn pull_pod_template(&self, template_id: &str) -> Result<firkin_oci::ImageBundle> {
        let image_ref = self.template_ref(template_id)?;
        let reference = firkin_oci::Reference::parse(&image_ref)
            .map_err(|error| runtime_error("parse pod OCI template reference", error))?;
        firkin_oci::Client::default()
            .pull(&reference)
            .await
            .map_err(|error| runtime_error("pull pod OCI template image", error))
    }

    fn pod_container_spec(
        request: &PodContainerCreateRequest,
        bundle: firkin_oci::ImageBundle,
    ) -> Result<PodContainerSpec> {
        let mut spec =
            PodContainerSpec::new(request.name.clone(), PodRootfsSource::oci_bundle(bundle))
                .map_err(|error| runtime_error("build pod container spec", error))?
                .envs(
                    request
                        .env_vars
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_str())),
                );
        if !request.command.is_empty() {
            spec = spec.command(request.command.clone());
        }
        if request.capture_output {
            spec = spec.stdout(Stdio::piped()).stderr(Stdio::piped());
        }
        for mount in &request.empty_dir_mounts {
            spec = if mount.read_only {
                spec.empty_dir_mount_read_only(mount.name.clone(), mount.path.clone())
            } else {
                spec.empty_dir_mount(mount.name.clone(), mount.path.clone())
            }
            .map_err(|error| runtime_error("build pod emptyDir mount", error))?;
        }
        Ok(spec)
    }

    async fn start_product_pod(&self, request: StartPodRequest) -> Result<RuntimePod> {
        for (template_id, prepared) in &request.prepared_templates {
            if prepared.is_some() {
                return Err(Error::UnsupportedCapability(format!(
                    "Apple/VZ product pods use OCI template ids directly; prepared template `{template_id}` is not supported"
                )));
            }
        }
        let pod_id = request
            .create_request
            .pod_id
            .clone()
            .unwrap_or_else(|| self.next_pod_id());
        if self.pods.lock().await.contains_key(&pod_id) {
            return Err(Error::Conflict(format!("pod {pod_id} already exists")));
        }
        let staging_dir = self.runtime_staging_dir(&format!("pod-{pod_id}"));
        tokio::fs::create_dir_all(&staging_dir)
            .await
            .map_err(|error| runtime_error("create pod staging dir", error))?;
        let pod_store_options = request.create_request.pod_store.clone();
        let pod_store_path = Self::prepare_product_pod_store(&pod_store_options, &staging_dir)?;
        let vm_builder = VmConfig::builder()
            .memory(Size::mib(512))
            .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
            .init_block(resolve_init_block()?);
        let (vm_builder, pod_store_id) =
            Self::add_product_pod_store_device(vm_builder, &pod_store_options, &pod_store_path);
        let mut builder = PodBuilder::new(
            pod_id.clone(),
            vm_builder
                .build()
                .map_err(|error| runtime_error("build pod VM config", error))?,
            PodStoreSpec::ext4(pod_store_id),
        )
        .map_err(|error| runtime_error("build pod", error))?;
        for volume in &request.create_request.empty_dirs {
            builder = builder.empty_dir(
                EmptyDirVolume::disk(volume.name.clone())
                    .map_err(|error| runtime_error("build pod emptyDir", error))?,
            );
        }
        let mut template_cache: HashMap<String, firkin_oci::ImageBundle> = HashMap::new();
        let mut container_infos = Vec::with_capacity(request.create_request.containers.len());
        for container in &request.create_request.containers {
            let bundle = if let Some(bundle) = template_cache.get(&container.template_id) {
                bundle.clone()
            } else {
                let bundle = self.pull_pod_template(&container.template_id).await?;
                template_cache.insert(container.template_id.clone(), bundle.clone());
                bundle
            };
            builder = builder.container(Self::pod_container_spec(container, bundle)?);
            container_infos.push(PodContainerInfo::running(container));
        }
        let pod = match builder.spawn().await {
            Ok(pod) => pod,
            Err(error) => {
                let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                return Err(runtime_error("spawn Apple/VZ pod", error));
            }
        };
        let (started_at, end_at) = Self::timestamps(request.create_request.timeout)?;
        self.pods.lock().await.insert(
            pod_id.clone(),
            Arc::new(Mutex::new(AppleVzRuntimePod {
                pod,
                staging_dir,
                pod_store_path,
                trim_policy: pod_store_options.trim_policy,
                templates: template_cache,
            })),
        );
        Ok(RuntimePod {
            config: PodRuntimeConfig {
                pod_id,
                started_at,
                end_at,
            },
            containers: container_infos,
        })
    }

    async fn attached_container(&self, sandbox_id: &str) -> Result<Arc<Mutex<Container>>> {
        let sandboxes = self.sandboxes.lock().await;
        sandboxes
            .get(sandbox_id)
            .map(|sandbox| Arc::clone(&sandbox.container))
            .ok_or_else(|| Error::SandboxNotFound(format!("sandbox {sandbox_id} not found")))
    }

    async fn process_adapter(&self, sandbox_id: &str) -> Result<AppleVzEnvdAdapter> {
        let sandboxes = self.sandboxes.lock().await;
        sandboxes
            .get(sandbox_id)
            .map(|sandbox| sandbox.process_adapter.clone())
            .ok_or_else(|| Error::SandboxNotFound(format!("sandbox {sandbox_id} not found")))
    }

    #[cfg(feature = "snapshot")]
    fn restore_rootfs_plan(
        &self,
        sandbox_id: &str,
        snapshot: &SnapshotRecord,
    ) -> Result<AppleVzRestoreRootfsPlan> {
        let staging_dir = self.runtime_staging_dir(sandbox_id);
        let source_staging_dir = snapshot.staging_dir.clone().ok_or_else(|| {
            Error::RuntimeLaunchFailed(format!(
                "firkin snapshot {} has no staging metadata",
                snapshot.snapshot_id
            ))
        })?;
        Ok(AppleVzRestoreRootfsPlan {
            source_rootfs: PathBuf::from(source_staging_dir)
                .join(format!("{}-rootfs.ext4", snapshot.source_sandbox_id)),
            restored_rootfs: staging_dir.join(format!("{sandbox_id}-rootfs.ext4")),
            staging_dir,
        })
    }

    async fn attach_runtime(
        &self,
        request: SingleNodeCreateRequest,
        container: Container,
    ) -> Result<RuntimeCreatedSandbox> {
        let container = Arc::new(Mutex::new(container));
        let envd_access_token = format!("envd-{}", uuid::Uuid::new_v4().simple());
        let envd_adapter = AppleVzEnvdAdapter::new(
            request.sandbox_id().to_owned(),
            Arc::clone(&container),
            request.env().clone(),
            self.logs.clone(),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| runtime_error("bind firkin envd", error))?;
        let envd_port = listener
            .local_addr()
            .map_err(|error| runtime_error("read firkin envd listener", error))?;
        self.ports
            .route_tcp(request.sandbox_id(), E2B_ENVD_PORT, envd_port)
            .await;
        let server = EnvdProcessHttpServer::new(envd_adapter.clone())
            .with_access_token(envd_access_token.clone());
        let envd_task = tokio::spawn(async move {
            let _ = server.serve(listener).await;
        });
        self.sandboxes.lock().await.insert(
            request.sandbox_id().to_owned(),
            AppleVzRuntimeSandbox {
                container,
                process_adapter: envd_adapter.clone(),
                envd_task: Some(envd_task),
                start_processes: Vec::new(),
            },
        );
        Ok(RuntimeCreatedSandbox {
            sandbox_id: request.sandbox_id().to_owned(),
            client_id: "firkin-local".to_owned(),
            envd_access_token: Some(envd_access_token),
            traffic_access_token: None,
        })
    }
}

#[async_trait]
impl RuntimeDriver for AppleVzLocalRuntimeDriver {
    fn runtime_sandbox_exists(&self, sandbox_id: &str) -> Result<bool> {
        match self.sandboxes.try_lock() {
            Ok(sandboxes) => Ok(sandboxes.contains_key(sandbox_id)),
            Err(_) => Ok(true),
        }
    }

    async fn create(&self, request: SingleNodeCreateRequest) -> Result<RuntimeCreatedSandbox> {
        Self::ensure_supported_runtime_mode(&request);
        let image_ref = self.template_ref(request.template_id())?;
        let reference = firkin_oci::Reference::parse(&image_ref)
            .map_err(|error| runtime_error("parse OCI template reference", error))?;
        let bundle = firkin_oci::Client::default()
            .pull(&reference)
            .await
            .map_err(|error| runtime_error("pull OCI template image", error))?;
        let resources = *request.resources();
        let cpu_count =
            NonZeroU32::new(resources.vcpus.max(1)).expect("vCPU count is clamped to at least one");
        let container = Container::builder(request.sandbox_id().to_owned())
            .map_err(|error| runtime_error("build container", error))?
            .image_config(bundle.config())
            .rootfs(Rootfs::oci_bundle(bundle))
            .cpus(cpu_count)
            .memory(Size::bytes(resources.memory_bytes))
            .command(["/bin/sh", "-lc", "sleep 2147483647"])
            .spawn_with_staging_dir(self.runtime_staging_dir(request.sandbox_id()))
            .await
            .map_err(|error| runtime_error("boot Apple/VZ sandbox container", error))?;
        self.attach_runtime(request, container).await
    }

    async fn delete(&self, sandbox_id: &str) -> Result<()> {
        let sandbox = self.sandboxes.lock().await.remove(sandbox_id);
        let Some(sandbox) = sandbox else {
            return Err(Error::SandboxNotFound(format!(
                "sandbox {sandbox_id} not found"
            )));
        };
        self.ports.remove_sandbox(sandbox_id).await;
        self.logs.remove_sandbox(sandbox_id)?;
        if let Some(envd_task) = sandbox.envd_task {
            envd_task.abort();
            let _ = envd_task.await;
        }
        for mut process in sandbox.start_processes {
            let _ = process.kill(Signal::new(15)).await;
        }
        let container = Arc::try_unwrap(sandbox.container)
            .map_err(|_| Error::RuntimeLaunchFailed("firkin container still in use".to_owned()))?;
        if let Err(error) = container
            .into_inner()
            .stop_with_grace(std::time::Duration::from_millis(0))
            .await
            && !vz_stop_error_means_already_stopped(&error.to_string())
        {
            return Err(runtime_error("stop Apple/VZ sandbox container", error));
        }
        Ok(())
    }

    async fn restore(
        &self,
        request: SingleNodeCreateRequest,
        snapshot: SnapshotRecord,
    ) -> Result<RuntimeCreatedSandbox> {
        Self::ensure_supported_runtime_mode(&request);
        #[cfg(not(feature = "snapshot"))]
        {
            let _ = snapshot;
            return Err(Error::UnsupportedCapability(
                "single-node Apple/VZ snapshot restore requires the firkin-runtime snapshot feature"
                    .to_owned(),
            ));
        }
        #[cfg(feature = "snapshot")]
        {
            let location = snapshot.location.clone().ok_or_else(|| {
                Error::RuntimeLaunchFailed(format!(
                    "firkin snapshot {} has no runtime location",
                    snapshot.snapshot_id
                ))
            })?;
            let machine_identifier = snapshot.machine_identifier.clone().ok_or_else(|| {
                Error::RuntimeLaunchFailed(format!(
                    "firkin snapshot {} has no machine identifier metadata",
                    snapshot.snapshot_id
                ))
            })?;
            let network_macs = snapshot.network_macs.clone().unwrap_or_default();
            let restore_plan = self.restore_rootfs_plan(request.sandbox_id(), &snapshot)?;
            let resources = *request.resources();
            let container = Container::builder(snapshot.source_sandbox_id.clone())
                .map_err(|error| runtime_error("build restored container", error))?
                .rootfs(Rootfs::ext4_image(restore_plan.source_rootfs))
                .cpus(
                    NonZeroU32::new(resources.vcpus.max(1))
                        .expect("vCPU count is clamped to at least one"),
                )
                .memory(Size::bytes(resources.memory_bytes))
                .command(["/bin/sh", "-lc", "sleep 2147483647"])
                .restore_from_snapshot(
                    location,
                    restore_plan.staging_dir,
                    machine_identifier,
                    network_macs,
                )
                .await
                .map_err(|error| runtime_error("restore Apple/VZ sandbox snapshot", error))?;
            self.attach_runtime(request, container).await
        }
    }

    async fn snapshot(&self, sandbox_id: &str, name: Option<String>) -> Result<RuntimeSnapshotRef> {
        #[cfg(not(feature = "snapshot"))]
        {
            let _ = (sandbox_id, name);
            return Err(Error::UnsupportedCapability(
                "single-node Apple/VZ snapshot requires the firkin-runtime snapshot feature"
                    .to_owned(),
            ));
        }
        #[cfg(feature = "snapshot")]
        {
            let sandbox = self.attached_container(sandbox_id).await?;
            let snapshot_id = name.unwrap_or_else(|| format!("{sandbox_id}-snapshot"));
            let path = self.snapshot_path(&snapshot_id)?;
            if path.exists() {
                return Err(Error::Conflict(format!(
                    "firkin snapshot {snapshot_id} already exists"
                )));
            }
            fs::create_dir_all(self.snapshot_dir.as_ref()).map_err(|error| {
                Error::StatePersistenceFailed(format!(
                    "create firkin snapshot dir {}: {error}",
                    self.snapshot_dir.display()
                ))
            })?;
            let container = sandbox.lock().await;
            let source_sandbox_id = container.id().to_string();
            container
                .save_snapshot(&path)
                .await
                .map_err(|error| runtime_error("snapshot Apple/VZ sandbox container", error))?;
            let state = container
                .snapshot_state()
                .await
                .map_err(|error| runtime_error("read Apple/VZ snapshot state", error))?;
            Ok(RuntimeSnapshotRef {
                snapshot_id,
                source_sandbox_id: Some(source_sandbox_id),
                location: Some(path.display().to_string()),
                staging_dir: Some(state.staging_dir().display().to_string()),
                machine_identifier: Some(state.machine_identifier().to_vec()),
                network_macs: Some(state.network_macs().to_vec()),
            })
        }
    }

    async fn delete_snapshot(&self, snapshot_id: &str) -> Result<()> {
        let path = self.snapshot_path(snapshot_id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::StatePersistenceFailed(format!(
                "delete firkin snapshot {}: {error}",
                path.display()
            ))),
        }
    }

    async fn run_command(
        &self,
        sandbox_id: &str,
        request: CommandRequest,
    ) -> Result<CommandOutput> {
        let container = self.attached_container(sandbox_id).await?;
        let mut config = ExecConfig::builder()
            .command(["/bin/sh".to_owned(), "-lc".to_owned(), request.command])
            .envs(
                request
                    .envs
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .working_dir(request.cwd.unwrap_or_else(|| "/".to_owned()))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(user) = request.user.as_deref() {
            config = config.user(exec_user_from_template_user(user));
        }
        if !request.stdin.is_empty() {
            config = config.stdin(Stdio::piped());
        }
        let mut process = container
            .lock()
            .await
            .exec(
                format!("cmd_{}", uuid::Uuid::new_v4().simple()),
                config.build(),
            )
            .await
            .map_err(|error| runtime_error("exec command", error))?;
        if !request.stdin.is_empty() {
            let mut stdin = process
                .take_stdin()
                .await
                .map_err(|error| runtime_error("open command stdin", error))?
                .ok_or_else(|| {
                    Error::RuntimeCommandFailed(
                        "single-node Apple/VZ command stdin pipe was not opened".to_owned(),
                    )
                })?;
            tokio::io::AsyncWriteExt::write_all(&mut stdin, &request.stdin)
                .await
                .map_err(|error| runtime_error("write command stdin", error))?;
        }
        let output = process
            .wait_with_output()
            .await
            .map_err(|error| runtime_error("wait command", error))?;
        let exit_code = output.status.code().unwrap_or(128);
        if !output.stdout.is_empty() {
            let _ = self.logs.record(
                sandbox_id,
                format!("stdout: {}", String::from_utf8_lossy(&output.stdout)),
            );
        }
        if !output.stderr.is_empty() {
            let _ = self.logs.record(
                sandbox_id,
                format!("stderr: {}", String::from_utf8_lossy(&output.stderr)),
            );
        }
        Ok(CommandOutput {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code,
        })
    }

    async fn start_process_stream(
        &self,
        sandbox_id: &str,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<E2bBackendError>> {
        self.process_adapter(sandbox_id)
            .await?
            .start_process_stream(request)
            .await
            .map_err(|error| Error::RuntimeCommandFailed(format!("start process stream: {error}")))
    }

    async fn list_processes(&self, sandbox_id: &str) -> Result<Vec<EnvdProcessInfo>> {
        self.process_adapter(sandbox_id)
            .await?
            .list_processes()
            .await
            .map_err(|error| Error::RuntimeCommandFailed(format!("list processes: {error}")))
    }

    async fn connect_process(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
    ) -> Result<EnvdProcessOutput> {
        self.process_adapter(sandbox_id)
            .await?
            .connect_process(selector)
            .await
            .map_err(|error| Error::RuntimeCommandFailed(format!("connect process: {error}")))
    }

    async fn send_process_input(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
        input: EnvdProcessInput,
    ) -> Result<()> {
        self.process_adapter(sandbox_id)
            .await?
            .send_process_input(selector, input)
            .await
            .map_err(|error| Error::RuntimeCommandFailed(format!("send process input: {error}")))
    }

    async fn close_process_stdin(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
    ) -> Result<()> {
        self.process_adapter(sandbox_id)
            .await?
            .close_process_stdin(selector)
            .await
            .map_err(|error| Error::RuntimeCommandFailed(format!("close process stdin: {error}")))
    }

    async fn signal_process(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
        signal: EnvdProcessSignal,
    ) -> Result<()> {
        self.process_adapter(sandbox_id)
            .await?
            .signal_process(selector, signal)
            .await
            .map_err(|error| Error::RuntimeCommandFailed(format!("signal process: {error}")))
    }

    async fn resize_process_pty(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
        pty: Option<EnvdPtySize>,
    ) -> Result<()> {
        self.process_adapter(sandbox_id)
            .await?
            .update_process_pty(selector, pty)
            .await
            .map_err(|error| Error::RuntimeCommandFailed(format!("resize process pty: {error}")))
    }

    async fn start_template_command(
        &self,
        sandbox_id: &str,
        command: String,
        envs: HashMap<String, String>,
    ) -> Result<()> {
        let container = self.attached_container(sandbox_id).await?;
        let process = container
            .lock()
            .await
            .exec(
                format!("start_{}", uuid::Uuid::new_v4().simple()),
                ExecConfig::builder()
                    .command(["/bin/sh".to_owned(), "-lc".to_owned(), command])
                    .envs(
                        envs.iter()
                            .map(|(key, value)| (key.as_str(), value.as_str())),
                    )
                    .working_dir("/")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .build(),
            )
            .await
            .map_err(|error| runtime_error("start template command", error))?;
        let pid = process.pid();
        let mut sandboxes = self.sandboxes.lock().await;
        let Some(sandbox) = sandboxes.get_mut(sandbox_id) else {
            let mut process = process;
            let _ = process.kill(Signal::new(15)).await;
            return Err(Error::SandboxNotFound(format!(
                "sandbox {sandbox_id} not found"
            )));
        };
        sandbox.start_processes.push(process);
        if let Some(pid) = pid {
            let _ = self.logs.record(
                sandbox_id,
                format!("template start command pid {pid} started"),
            );
        }
        Ok(())
    }
}

#[async_trait]
impl RuntimeAdapter for AppleVzLocalRuntimeDriver {
    async fn preflight(&self) -> std::result::Result<RuntimeCapabilitySet, E2bBackendError> {
        Ok(RuntimeCapabilitySet {
            backend: "apple-vz-single-node".to_owned(),
            supported: vec![
                "single-vm-backed-container".to_owned(),
                "product-pods".to_owned(),
                "pod-emptydir".to_owned(),
                "pod-guest-path-rootfs".to_owned(),
                "pod-shared-rootfs-template".to_owned(),
                "pod-store-asif".to_owned(),
                "pod-store-raw-size".to_owned(),
                "pod-store-trim-on-remove".to_owned(),
            ],
            unsupported: vec![
                (
                    "prepared-template-pods".to_owned(),
                    "Apple/VZ product pods currently pull OCI template ids directly".to_owned(),
                ),
                (
                    "pod-snapshot-restore".to_owned(),
                    "pod-aware snapshot restore is not wired in the single-node driver".to_owned(),
                ),
            ],
        })
    }

    async fn prepare_template(
        &self,
        request: TemplateBuildRequest,
    ) -> std::result::Result<firkin_e2b_contract::PreparedTemplate, E2bBackendError> {
        Err(E2bBackendError::Runtime(format!(
            "Apple/VZ single-node driver cannot prepare template `{}` through RuntimeAdapter",
            request.name.unwrap_or_else(|| "unnamed".to_owned())
        )))
    }

    async fn start(
        &self,
        request: StartSandboxRequest,
    ) -> std::result::Result<RuntimeSandbox, E2bBackendError> {
        let sandbox_id = self.next_adapter_sandbox_id();
        let resources = SandboxResources::new(1, Size::mib(512));
        let mut create = SingleNodeCreateRequest::new(
            sandbox_id.clone(),
            request.create_request.template_id.clone(),
            resources,
        );
        if let Some(timeout) = request.create_request.timeout {
            create = create.with_timeout(std::time::Duration::from_secs(timeout));
        }
        create = create.with_envs(
            request
                .create_request
                .env_vars
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
        let created = <Self as RuntimeDriver>::create(self, create)
            .await
            .map_err(single_node_to_backend_error)?;
        let (started_at, end_at) = Self::timestamps(request.create_request.timeout)
            .map_err(single_node_to_backend_error)?;
        Ok(RuntimeSandbox {
            config: SandboxRuntimeConfig {
                sandbox_id: created.sandbox_id,
                domain: "localhost".to_owned(),
                envd_version: "apple-vz".to_owned(),
                envd_access_token: created.envd_access_token,
                traffic_access_token: created.traffic_access_token,
                started_at,
                end_at,
                cpu_count: resources.cpu_count(),
                memory_mb: u32::try_from(resources.memory().as_bytes() / (1024 * 1024))
                    .unwrap_or(u32::MAX),
            },
            exposed_ports: vec![E2B_ENVD_PORT],
        })
    }

    async fn start_followup(
        &self,
        _request: StartSandboxRequest,
        snapshot: FollowupSnapshot,
    ) -> std::result::Result<RuntimeSandbox, E2bBackendError> {
        Err(E2bBackendError::Runtime(format!(
            "Apple/VZ single-node follow-up restore is not exposed through RuntimeAdapter for `{}`",
            snapshot.snapshot_id
        )))
    }

    async fn start_pod(
        &self,
        request: StartPodRequest,
    ) -> std::result::Result<RuntimePod, E2bBackendError> {
        self.start_product_pod(request)
            .await
            .map_err(single_node_to_backend_error)
    }

    async fn stop_pod(&self, pod_id: &str) -> std::result::Result<(), E2bBackendError> {
        let pod = {
            let pods = self.pods.lock().await;
            pods.get(pod_id)
                .cloned()
                .ok_or_else(|| E2bBackendError::NotFound(pod_id.to_owned()))?
        };
        let staging_dir = {
            let mut runtime_pod = pod.lock().await;
            if runtime_pod.trim_policy == PodTrimPolicy::OnStop {
                runtime_pod
                    .pod
                    .shutdown_trimming_store()
                    .await
                    .map_err(|error| {
                        E2bBackendError::Runtime(format!("stop Apple/VZ pod: {error}"))
                    })?;
            } else {
                runtime_pod.pod.shutdown().await.map_err(|error| {
                    E2bBackendError::Runtime(format!("stop Apple/VZ pod: {error}"))
                })?;
            }
            runtime_pod.staging_dir.clone()
        };
        self.pods.lock().await.remove(pod_id);
        let _ = tokio::fs::remove_dir_all(staging_dir).await;
        Ok(())
    }

    async fn add_pod_container(
        &self,
        pod_id: &str,
        container: PodContainerCreateRequest,
    ) -> std::result::Result<PodContainerInfo, E2bBackendError> {
        let existing_bundle = {
            let pod = self.pod_handle_for_adapter(pod_id).await?;
            let runtime_pod = pod.lock().await;
            runtime_pod.templates.get(&container.template_id).cloned()
        };
        let bundle = match existing_bundle {
            Some(bundle) => bundle,
            None => self
                .pull_pod_template(&container.template_id)
                .await
                .map_err(single_node_to_backend_error)?,
        };
        let spec = Self::pod_container_spec(&container, bundle.clone())
            .map_err(single_node_to_backend_error)?;
        let pod = self.pod_handle_for_adapter(pod_id).await?;
        let pending = {
            let mut runtime_pod = pod.lock().await;
            runtime_pod
                .pod
                .begin_container_add(spec)
                .await
                .map_err(|error| {
                    E2bBackendError::Runtime(format!("add Apple/VZ pod container: {error}"))
                })?
        };
        let pending_id = pending.id().clone();
        let prepared = match pending.prepare().await {
            Ok(prepared) => prepared,
            Err(error) => {
                let mut runtime_pod = pod.lock().await;
                runtime_pod.pod.abort_container_add(&pending_id);
                return Err(E2bBackendError::Runtime(format!(
                    "add Apple/VZ pod container: {error}"
                )));
            }
        };
        let started = match prepared.start().await {
            Ok(started) => started,
            Err(error) => {
                let mut runtime_pod = pod.lock().await;
                runtime_pod.pod.abort_container_add(&pending_id);
                return Err(E2bBackendError::Runtime(format!(
                    "add Apple/VZ pod container: {error}"
                )));
            }
        };
        {
            let mut runtime_pod = pod.lock().await;
            runtime_pod
                .pod
                .commit_container_add(started)
                .map_err(|error| {
                    E2bBackendError::Runtime(format!("add Apple/VZ pod container: {error}"))
                })?;
            runtime_pod
                .templates
                .entry(container.template_id.clone())
                .or_insert(bundle);
        }
        Ok(PodContainerInfo::running(&container))
    }

    async fn remove_pod_container(
        &self,
        pod_id: &str,
        container_name: &str,
    ) -> std::result::Result<(), E2bBackendError> {
        let pod = self.pod_handle_for_adapter(pod_id).await?;
        let mut runtime_pod = pod.lock().await;
        runtime_pod
            .pod
            .remove_container(container_name)
            .await
            .map(|_| ())
            .map_err(|error| {
                E2bBackendError::Runtime(format!("remove Apple/VZ pod container: {error}"))
            })?;
        if runtime_pod.trim_policy == PodTrimPolicy::OnRemove {
            let _ = runtime_pod.pod.trim_store().await;
        }
        Ok(())
    }

    async fn wait_pod_container(
        &self,
        pod_id: &str,
        container_name: &str,
    ) -> std::result::Result<PodContainerOutput, E2bBackendError> {
        let pod = self.pod_handle_for_adapter(pod_id).await?;
        let (detached, trim_policy) = {
            let mut runtime_pod = pod.lock().await;
            let detached = runtime_pod
                .pod
                .detach_container_for_wait(container_name)
                .map_err(|error| {
                    E2bBackendError::Runtime(format!("wait Apple/VZ pod container: {error}"))
                })?;
            (detached, runtime_pod.trim_policy)
        };
        let output = detached.wait_with_output().await.map_err(|error| {
            E2bBackendError::Runtime(format!("wait Apple/VZ pod container: {error}"))
        })?;
        if trim_policy == PodTrimPolicy::OnRemove
            && let Ok(pod) = self.pod_handle_for_adapter(pod_id).await
        {
            let mut runtime_pod = pod.lock().await;
            let _ = runtime_pod.pod.trim_store().await;
        }
        Ok(PodContainerOutput::new(
            output.stdout,
            output.stderr,
            output.status.code().unwrap_or(128),
        ))
    }

    async fn stop(&self, sandbox_id: &str) -> std::result::Result<(), E2bBackendError> {
        <Self as RuntimeDriver>::delete(self, sandbox_id)
            .await
            .map_err(single_node_to_backend_error)
    }

    async fn pause(&self, sandbox_id: &str) -> std::result::Result<PausedSandbox, E2bBackendError> {
        Err(E2bBackendError::Runtime(format!(
            "Apple/VZ single-node RuntimeAdapter does not pause `{sandbox_id}`"
        )))
    }

    async fn resume(
        &self,
        paused: PausedSandbox,
    ) -> std::result::Result<RuntimeSandbox, E2bBackendError> {
        Err(E2bBackendError::Runtime(format!(
            "Apple/VZ single-node RuntimeAdapter does not resume `{}`",
            paused.sandbox_id
        )))
    }

    async fn snapshot(
        &self,
        sandbox_id: &str,
        name: Option<String>,
    ) -> std::result::Result<SnapshotRef, E2bBackendError> {
        let snapshot = <Self as RuntimeDriver>::snapshot(self, sandbox_id, name)
            .await
            .map_err(single_node_to_backend_error)?;
        Ok(SnapshotRef {
            snapshot_id: snapshot.snapshot_id,
            location: snapshot.location,
            artifact_integrity: None,
        })
    }

    async fn metrics(
        &self,
        _sandbox_id: &str,
    ) -> std::result::Result<Vec<SandboxMetric>, E2bBackendError> {
        Ok(Vec::new())
    }

    async fn logs(&self, sandbox_id: &str) -> std::result::Result<SandboxLogs, E2bBackendError> {
        let logs = self
            .logs
            .entries(sandbox_id, None, 0)
            .map_err(single_node_to_backend_error)?;
        Ok(SandboxLogs {
            logs: logs
                .into_iter()
                .map(|event| firkin_e2b_wire::SandboxLogEntry {
                    timestamp: event.timestamp_unix_seconds.to_string(),
                    level: firkin_e2b_wire::LogLevel::Info,
                    message: event.message,
                    fields: std::collections::BTreeMap::new(),
                })
                .collect(),
        })
    }

    async fn apply_network(
        &self,
        sandbox_id: &str,
        _policy: SandboxNetworkPolicy,
    ) -> std::result::Result<(), E2bBackendError> {
        Err(E2bBackendError::Runtime(format!(
            "Apple/VZ single-node RuntimeAdapter does not enforce network policy for `{sandbox_id}`"
        )))
    }

    async fn port_target(
        &self,
        sandbox_id: &str,
        port: u16,
    ) -> std::result::Result<PortTarget, E2bBackendError> {
        self.ports.target(sandbox_id, port).await
    }
}

#[derive(Clone)]
struct AppleVzEnvdAdapter {
    sandbox_id: String,
    container: Arc<Mutex<Container>>,
    base_envs: Arc<HashMap<String, String>>,
    logs: LogStore,
    processes: Arc<Mutex<HashMap<u32, AppleVzEnvdProcessRecord>>>,
    next_process_id: Arc<AtomicU32>,
}

struct AppleVzEnvdProcessRecord {
    tag: Option<String>,
    cmd: String,
    args: Vec<String>,
    envs: BTreeMap<String, String>,
    cwd: Option<String>,
    output: EnvdProcessOutput,
    stdin: Option<mpsc::Sender<Vec<u8>>>,
    kill_handle: Option<ProcessKillHandle>,
    pty_control: Option<Arc<Mutex<firkin_core::PtyControl>>>,
}

#[derive(Clone, Copy)]
enum AppleVzProcessOutputKind {
    Stdout,
    Stderr,
    Pty,
}

impl AppleVzEnvdAdapter {
    fn new(
        sandbox_id: String,
        container: Arc<Mutex<Container>>,
        base_envs: HashMap<String, String>,
        logs: LogStore,
    ) -> Self {
        Self {
            sandbox_id,
            container,
            base_envs: Arc::new(base_envs),
            logs,
            processes: Arc::new(Mutex::new(HashMap::new())),
            next_process_id: Arc::new(AtomicU32::new(1)),
        }
    }

    fn process_command(request: &EnvdProcessStartRequest) -> E2bResult<Vec<String>> {
        if request.cmd.is_empty() {
            return Err(E2bBackendError::Runtime(
                "process command is required".to_owned(),
            ));
        }
        Ok(std::iter::once(request.cmd.clone())
            .chain(request.args.clone())
            .collect())
    }

    fn effective_process_envs(
        &self,
        request: &EnvdProcessStartRequest,
    ) -> BTreeMap<String, String> {
        let mut envs = self
            .base_envs
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        envs.extend(request.envs.clone());
        envs
    }

    fn process_exec_config(
        &self,
        request: &EnvdProcessStartRequest,
        stdin: Stdio,
        stdout: Stdio,
        stderr: Stdio,
    ) -> E2bResult<ExecConfig> {
        let envs = self.effective_process_envs(request);
        Ok(ExecConfig::builder()
            .command(Self::process_command(request)?)
            .envs(
                envs.iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .working_dir(request.cwd.clone().unwrap_or_else(|| "/".to_owned()))
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr)
            .build())
    }

    fn process_pty_exec_config(
        &self,
        request: &EnvdProcessStartRequest,
    ) -> E2bResult<ExecConfig<firkin_core::Pty>> {
        let pty = request
            .pty
            .ok_or_else(|| E2bBackendError::Runtime("process PTY size is required".to_owned()))?;
        let envs = self.effective_process_envs(request);
        Ok(ExecConfig::builder()
            .command(Self::process_command(request)?)
            .envs(
                envs.iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .working_dir(request.cwd.clone().unwrap_or_else(|| "/".to_owned()))
            .stdin(Stdio::piped())
            .pty(Self::envd_pty_size_to_core(pty)?)
            .build())
    }

    fn envd_pty_size_to_core(size: EnvdPtySize) -> E2bResult<PtyConfig> {
        let cols = u16::try_from(size.cols).map_err(|error| {
            E2bBackendError::Runtime(format!("convert PTY cols {}: {error}", size.cols))
        })?;
        let rows = u16::try_from(size.rows).map_err(|error| {
            E2bBackendError::Runtime(format!("convert PTY rows {}: {error}", size.rows))
        })?;
        Ok(PtyConfig::new(cols, rows))
    }

    async fn require_process(&self, selector: EnvdProcessSelector) -> E2bResult<u32> {
        let processes = self.processes.lock().await;
        match selector {
            EnvdProcessSelector::Pid(pid) if processes.contains_key(&pid) => Ok(pid),
            EnvdProcessSelector::Pid(pid) => Err(E2bBackendError::NotFound(pid.to_string())),
            EnvdProcessSelector::Tag(tag) => processes
                .iter()
                .find_map(|(pid, record)| (record.tag.as_deref() == Some(&tag)).then_some(*pid))
                .ok_or(E2bBackendError::NotFound(tag)),
        }
    }

    async fn allocate_process(
        &self,
        request: &EnvdProcessStartRequest,
        output: EnvdProcessOutput,
        stdin: Option<mpsc::Sender<Vec<u8>>>,
        kill_handle: Option<ProcessKillHandle>,
        pty_control: Option<Arc<Mutex<firkin_core::PtyControl>>>,
    ) -> u32 {
        let pid = self.next_process_id.fetch_add(1, Ordering::Relaxed).max(1);
        let mut output = output;
        output.pid = pid;
        self.processes.lock().await.insert(
            pid,
            AppleVzEnvdProcessRecord {
                tag: request.tag.clone(),
                cmd: request.cmd.clone(),
                args: request.args.clone(),
                envs: self.effective_process_envs(request),
                cwd: request.cwd.clone(),
                output,
                stdin,
                kill_handle,
                pty_control,
            },
        );
        pid
    }

    async fn append_process_output(&self, pid: u32, kind: AppleVzProcessOutputKind, bytes: &[u8]) {
        {
            let mut processes = self.processes.lock().await;
            let Some(record) = processes.get_mut(&pid) else {
                return;
            };
            match kind {
                AppleVzProcessOutputKind::Stdout => record.output.stdout.extend_from_slice(bytes),
                AppleVzProcessOutputKind::Stderr => record.output.stderr.extend_from_slice(bytes),
                AppleVzProcessOutputKind::Pty => record.output.pty.extend_from_slice(bytes),
            }
        }
        let text = String::from_utf8_lossy(bytes);
        if !text.is_empty() {
            let stream = match kind {
                AppleVzProcessOutputKind::Stdout => "stdout",
                AppleVzProcessOutputKind::Stderr => "stderr",
                AppleVzProcessOutputKind::Pty => "pty",
            };
            let _ = self
                .logs
                .record(&self.sandbox_id, format!("{stream}: {text}"));
        }
    }

    async fn finish_process(
        &self,
        pid: u32,
        exit_code: i32,
        status: String,
        error: Option<String>,
    ) {
        let mut processes = self.processes.lock().await;
        let Some(record) = processes.get_mut(&pid) else {
            return;
        };
        record.output.exit_code = exit_code;
        record.output.exited = true;
        record.output.status = status;
        record.output.error = error;
        record.stdin = None;
        record.kill_handle = None;
        record.pty_control = None;
    }

    fn spawn_process_output_reader<R>(
        &self,
        pid: u32,
        mut reader: R,
        kind: AppleVzProcessOutputKind,
        sender: mpsc::Sender<E2bResult<EnvdProcessStreamEvent>>,
    ) -> JoinHandle<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let adapter = self.clone();
        tokio::spawn(async move {
            let mut buffer = [0_u8; 8192];
            loop {
                let bytes_read = match reader.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(bytes_read) => bytes_read,
                    Err(error) => {
                        let _ = sender
                            .send(Err(E2bBackendError::Runtime(format!(
                                "read process output: {error}"
                            ))))
                            .await;
                        break;
                    }
                };
                let bytes = buffer[..bytes_read].to_vec();
                adapter.append_process_output(pid, kind, &bytes).await;
                let event = match kind {
                    AppleVzProcessOutputKind::Stdout => EnvdProcessStreamEvent::Stdout(bytes),
                    AppleVzProcessOutputKind::Stderr => EnvdProcessStreamEvent::Stderr(bytes),
                    AppleVzProcessOutputKind::Pty => EnvdProcessStreamEvent::Pty(bytes),
                };
                if sender.send(Ok(event)).await.is_err() {
                    break;
                }
            }
        })
    }

    async fn start_pty_process_stream(
        &self,
        request: EnvdProcessStartRequest,
    ) -> E2bResult<EnvdProcessEventStream<E2bBackendError>> {
        let config = self.process_pty_exec_config(&request)?;
        let mut process = self
            .container
            .lock()
            .await
            .exec(format!("proc_{}", uuid::Uuid::new_v4().simple()), config)
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("exec PTY process: {error}")))?;
        let kill_handle = process.kill_handle();
        let pty = process
            .take_pty()
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("open process PTY: {error}")))?
            .ok_or_else(|| {
                E2bBackendError::Runtime("firkin process did not return a PTY handle".to_owned())
            })?;
        let (mut pty_input, pty_output, pty_control) = pty.split();
        let pty_control = Arc::new(Mutex::new(pty_control));
        let (input_sender, mut input_receiver) = mpsc::channel::<Vec<u8>>(16);
        let pid = self
            .allocate_process(
                &request,
                EnvdProcessOutput {
                    pid: 0,
                    status: "running".to_owned(),
                    ..EnvdProcessOutput::default()
                },
                Some(input_sender),
                Some(kill_handle),
                Some(Arc::clone(&pty_control)),
            )
            .await;
        let (sender, receiver) = mpsc::channel(32);
        sender
            .try_send(Ok(EnvdProcessStreamEvent::Start { pid }))
            .expect("fresh process event stream channel has capacity");

        tokio::spawn(async move {
            while let Some(bytes) = input_receiver.recv().await {
                if pty_input.write_all(&bytes).await.is_err() {
                    break;
                }
                if pty_input.flush().await.is_err() {
                    break;
                }
            }
        });

        let adapter = self.clone();
        tokio::spawn(async move {
            let output_task = adapter.spawn_process_output_reader(
                pid,
                pty_output,
                AppleVzProcessOutputKind::Pty,
                sender.clone(),
            );
            let _ = output_task.await;
            let status = process.wait().await;
            let (exit_code, status_text, error) = match status {
                Ok(status) => {
                    let exit_code = status.code().unwrap_or(128);
                    let status_text = if status.success() {
                        "exited"
                    } else {
                        "errored"
                    };
                    (
                        exit_code,
                        status_text.to_owned(),
                        (!status.success()).then(|| format!("process exited {exit_code}")),
                    )
                }
                Err(error) => (
                    128,
                    "errored".to_owned(),
                    Some(format!("wait failed: {error}")),
                ),
            };
            adapter
                .finish_process(pid, exit_code, status_text.clone(), error.clone())
                .await;
            let _ = sender
                .send(Ok(EnvdProcessStreamEvent::End {
                    exit_code,
                    exited: true,
                    status: status_text,
                    error,
                }))
                .await;
        });

        Ok(EnvdProcessEventStream::from_receiver(receiver))
    }

    async fn run_filesystem_shell(
        &self,
        command: &str,
        envs: HashMap<&str, String>,
    ) -> E2bResult<Vec<u8>> {
        let config = ExecConfig::builder()
            .command(["/bin/sh".to_owned(), "-lc".to_owned(), command.to_owned()])
            .envs(envs.iter().map(|(key, value)| (*key, value.as_str())))
            .working_dir("/")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .build();
        let output = self
            .container
            .lock()
            .await
            .exec(format!("fs_{}", uuid::Uuid::new_v4().simple()), config)
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("exec filesystem command: {error}")))?
            .wait_with_output()
            .await
            .map_err(|error| {
                E2bBackendError::Runtime(format!("wait filesystem command: {error}"))
            })?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        if output.status.code() == Some(44) {
            return Err(E2bBackendError::NotFound(
                envs.get("FIRKIN_FS_PATH")
                    .or_else(|| envs.get("FIRKIN_FS_SOURCE"))
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_owned()),
            ));
        }
        Err(E2bBackendError::Runtime(format!(
            "filesystem command exited {}: {}",
            output.status.code().unwrap_or(128),
            String::from_utf8_lossy(&output.stderr)
        )))
    }

    async fn stat_filesystem_entry(&self, path: &str) -> E2bResult<EnvdFilesystemEntry> {
        let output = self
            .run_filesystem_shell(
                r#"p="$FIRKIN_FS_PATH"
if [ ! -e "$p" ]; then exit 44; fi
kind=file
if [ -d "$p" ]; then kind=directory; fi
printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$p" "$kind" "$(stat -c '%s' -- "$p")" "$(stat -c '%f' -- "$p")" "$(stat -c '%A' -- "$p")" "$(stat -c '%U' -- "$p")" "$(stat -c '%G' -- "$p")""#,
                HashMap::from([("FIRKIN_FS_PATH", path.to_owned())]),
            )
            .await?;
        parse_filesystem_entry(&String::from_utf8_lossy(&output))
    }
}

#[allow(clippy::too_many_lines)]
#[async_trait]
impl EnvdProcessAdapter for AppleVzEnvdAdapter {
    type Error = E2bBackendError;

    async fn list_processes(&self) -> E2bResult<Vec<EnvdProcessInfo>> {
        let processes = self.processes.lock().await;
        Ok(processes
            .iter()
            .map(|(pid, record)| EnvdProcessInfo {
                pid: *pid,
                tag: record.tag.clone(),
                cmd: record.cmd.clone(),
                args: record.args.clone(),
                envs: record.envs.clone(),
                cwd: record.cwd.clone(),
            })
            .collect())
    }

    async fn send_process_input(
        &self,
        selector: EnvdProcessSelector,
        input: EnvdProcessInput,
    ) -> E2bResult<()> {
        let pid = self.require_process(selector).await?;
        let sender = {
            let processes = self.processes.lock().await;
            processes.get(&pid).and_then(|record| record.stdin.clone())
        }
        .ok_or_else(|| E2bBackendError::Runtime(format!("process {pid} stdin is closed")))?;
        let bytes = match input {
            EnvdProcessInput::Stdin(bytes) | EnvdProcessInput::Pty(bytes) => bytes,
        };
        sender
            .send(bytes)
            .await
            .map_err(|_| E2bBackendError::Runtime(format!("process {pid} stdin is closed")))
    }

    async fn close_process_stdin(&self, selector: EnvdProcessSelector) -> E2bResult<()> {
        let pid = self.require_process(selector).await?;
        let mut processes = self.processes.lock().await;
        let record = processes
            .get_mut(&pid)
            .ok_or_else(|| E2bBackendError::NotFound(pid.to_string()))?;
        record.stdin = None;
        Ok(())
    }

    async fn signal_process(
        &self,
        selector: EnvdProcessSelector,
        signal: EnvdProcessSignal,
    ) -> E2bResult<()> {
        let pid = self.require_process(selector).await?;
        let kill_handle = {
            let processes = self.processes.lock().await;
            processes
                .get(&pid)
                .and_then(|record| record.kill_handle.clone())
        };
        let Some(kill_handle) = kill_handle else {
            return Ok(());
        };
        let signal = match signal {
            EnvdProcessSignal::Unspecified => return Ok(()),
            EnvdProcessSignal::Sigterm => Signal::new(15),
            EnvdProcessSignal::Sigkill => Signal::new(9),
            EnvdProcessSignal::Unknown(raw) => Signal::new(raw),
        };
        kill_handle
            .kill(signal)
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("kill process {pid}: {error}")))
    }

    async fn connect_process(&self, selector: EnvdProcessSelector) -> E2bResult<EnvdProcessOutput> {
        let pid = self.require_process(selector).await?;
        let processes = self.processes.lock().await;
        processes
            .get(&pid)
            .map(|record| record.output.clone())
            .ok_or_else(|| E2bBackendError::NotFound(pid.to_string()))
    }

    async fn update_process_pty(
        &self,
        selector: EnvdProcessSelector,
        pty: Option<EnvdPtySize>,
    ) -> E2bResult<()> {
        let pid = self.require_process(selector).await?;
        let Some(size) = pty else {
            return Ok(());
        };
        let control = {
            let processes = self.processes.lock().await;
            processes
                .get(&pid)
                .and_then(|record| record.pty_control.clone())
        }
        .ok_or_else(|| {
            E2bBackendError::Runtime(format!("process {pid} was not started with a PTY"))
        })?;
        control
            .lock()
            .await
            .resize(Self::envd_pty_size_to_core(size)?)
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("resize process {pid} PTY: {error}")))
    }

    async fn start_process(
        &self,
        request: EnvdProcessStartRequest,
    ) -> E2bResult<EnvdProcessOutput> {
        let config =
            self.process_exec_config(&request, Stdio::Null, Stdio::piped(), Stdio::piped())?;
        let output = self
            .container
            .lock()
            .await
            .exec(format!("proc_{}", uuid::Uuid::new_v4().simple()), config)
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("exec process: {error}")))?
            .wait_with_output()
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("wait process: {error}")))?;
        let exit_code = output.status.code().unwrap_or(128);
        let stdout_text = String::from_utf8_lossy(&output.stdout);
        if !stdout_text.is_empty() {
            let _ = self
                .logs
                .record(&self.sandbox_id, format!("stdout: {stdout_text}"));
        }
        let stderr_text = String::from_utf8_lossy(&output.stderr);
        if !stderr_text.is_empty() {
            let _ = self
                .logs
                .record(&self.sandbox_id, format!("stderr: {stderr_text}"));
        }
        let mut process_output = EnvdProcessOutput {
            pid: 0,
            stdout: output.stdout,
            stderr: output.stderr,
            pty: Vec::new(),
            exit_code,
            exited: true,
            status: if output.status.success() {
                "exited".to_owned()
            } else {
                "errored".to_owned()
            },
            error: (!output.status.success()).then(|| format!("process exited {exit_code}")),
        };
        let pid = self
            .allocate_process(&request, process_output.clone(), None, None, None)
            .await;
        process_output.pid = pid;
        Ok(process_output)
    }

    async fn start_process_stream(
        &self,
        request: EnvdProcessStartRequest,
    ) -> E2bResult<EnvdProcessEventStream<E2bBackendError>> {
        if request.pty.is_some() {
            return self.start_pty_process_stream(request).await;
        }
        let config =
            self.process_exec_config(&request, Stdio::piped(), Stdio::piped(), Stdio::piped())?;
        let mut process = self
            .container
            .lock()
            .await
            .exec(format!("proc_{}", uuid::Uuid::new_v4().simple()), config)
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("exec process: {error}")))?;
        let kill_handle = process.kill_handle();
        let (input_sender, mut input_receiver) = mpsc::channel::<Vec<u8>>(16);
        let pid = self
            .allocate_process(
                &request,
                EnvdProcessOutput {
                    pid: 0,
                    status: "running".to_owned(),
                    ..EnvdProcessOutput::default()
                },
                Some(input_sender),
                Some(kill_handle),
                None,
            )
            .await;
        let (sender, receiver) = mpsc::channel(32);
        sender
            .try_send(Ok(EnvdProcessStreamEvent::Start { pid }))
            .expect("fresh process event stream channel has capacity");
        let adapter = self.clone();
        tokio::spawn(async move {
            let stdin = process.take_stdin().await;
            if let Ok(Some(mut stdin)) = stdin {
                tokio::spawn(async move {
                    while let Some(bytes) = input_receiver.recv().await {
                        if stdin.write_all(&bytes).await.is_err() {
                            break;
                        }
                        if stdin.flush().await.is_err() {
                            break;
                        }
                    }
                });
            }
            let mut output_tasks = Vec::new();
            match process.take_stdout().await {
                Ok(Some(stdout)) => output_tasks.push(adapter.spawn_process_output_reader(
                    pid,
                    stdout,
                    AppleVzProcessOutputKind::Stdout,
                    sender.clone(),
                )),
                Ok(None) => {}
                Err(error) => {
                    let _ = sender
                        .send(Err(E2bBackendError::Runtime(format!(
                            "take stdout: {error}"
                        ))))
                        .await;
                }
            }
            match process.take_stderr().await {
                Ok(Some(stderr)) => output_tasks.push(adapter.spawn_process_output_reader(
                    pid,
                    stderr,
                    AppleVzProcessOutputKind::Stderr,
                    sender.clone(),
                )),
                Ok(None) => {}
                Err(error) => {
                    let _ = sender
                        .send(Err(E2bBackendError::Runtime(format!(
                            "take stderr: {error}"
                        ))))
                        .await;
                }
            }
            for task in output_tasks {
                let _ = task.await;
            }
            let status = process.wait().await;
            let (exit_code, status_text, error) = match status {
                Ok(status) => {
                    let exit_code = status.code().unwrap_or(128);
                    let status_text = if status.success() {
                        "exited"
                    } else {
                        "errored"
                    };
                    (
                        exit_code,
                        status_text.to_owned(),
                        (!status.success()).then(|| format!("process exited {exit_code}")),
                    )
                }
                Err(error) => (
                    128,
                    "errored".to_owned(),
                    Some(format!("wait failed: {error}")),
                ),
            };
            adapter
                .finish_process(pid, exit_code, status_text.clone(), error.clone())
                .await;
            let _ = sender
                .send(Ok(EnvdProcessStreamEvent::End {
                    exit_code,
                    exited: true,
                    status: status_text,
                    error,
                }))
                .await;
        });
        Ok(EnvdProcessEventStream::from_receiver(receiver))
    }
}

#[async_trait]
impl EnvdFilesystemAdapter for AppleVzEnvdAdapter {
    type Error = E2bBackendError;

    async fn read_file(&self, path: String) -> E2bResult<Vec<u8>> {
        let temp_path =
            std::env::temp_dir().join(format!("firkin-read-{}", uuid::Uuid::new_v4().simple()));
        self.container
            .lock()
            .await
            .copy_out(&path, &temp_path)
            .await
            .map_err(|error| E2bBackendError::Runtime(format!("copy out `{path}`: {error}")))?;
        let bytes = tokio::fs::read(&temp_path).await.map_err(|error| {
            E2bBackendError::Runtime(format!("read copied file {}: {error}", temp_path.display()))
        })?;
        let _ = tokio::fs::remove_file(&temp_path).await;
        Ok(bytes)
    }

    async fn write_file(&self, path: String, data: Vec<u8>) -> E2bResult<EnvdFilesystemWriteInfo> {
        let temp_path =
            std::env::temp_dir().join(format!("firkin-write-{}", uuid::Uuid::new_v4().simple()));
        tokio::fs::write(&temp_path, data).await.map_err(|error| {
            E2bBackendError::Runtime(format!("stage write file {}: {error}", temp_path.display()))
        })?;
        let result = self.container.lock().await.copy_in(&temp_path, &path).await;
        let _ = tokio::fs::remove_file(&temp_path).await;
        result.map_err(|error| E2bBackendError::Runtime(format!("copy in `{path}`: {error}")))?;
        Ok(EnvdFilesystemWriteInfo {
            name: envd_path_name(&path),
            file_type: "file".to_owned(),
            path: envd_normalize_path(&path),
        })
    }

    async fn list_dir(&self, path: String, depth: u32) -> E2bResult<Vec<EnvdFilesystemEntry>> {
        let output = self
            .run_filesystem_shell(
                r#"p="$FIRKIN_FS_PATH"
if [ ! -d "$p" ]; then exit 44; fi
if [ "$FIRKIN_FS_DEPTH" = "0" ]; then
  find "$p" -mindepth 1 -print
else
  find "$p" -mindepth 1 -maxdepth "$FIRKIN_FS_DEPTH" -print
fi"#,
                HashMap::from([
                    ("FIRKIN_FS_PATH", path.clone()),
                    ("FIRKIN_FS_DEPTH", depth.to_string()),
                ]),
            )
            .await?;
        let mut entries = Vec::new();
        for entry_path in String::from_utf8_lossy(&output)
            .lines()
            .filter(|line| !line.is_empty())
        {
            entries.push(self.stat_filesystem_entry(entry_path).await?);
        }
        Ok(entries)
    }

    async fn make_dir(&self, path: String) -> E2bResult<EnvdFilesystemEntry> {
        self.run_filesystem_shell(
            r#"mkdir -p -- "$FIRKIN_FS_PATH""#,
            HashMap::from([("FIRKIN_FS_PATH", path.clone())]),
        )
        .await?;
        self.stat_filesystem_entry(&path).await
    }

    async fn move_entry(
        &self,
        source: String,
        destination: String,
    ) -> E2bResult<EnvdFilesystemEntry> {
        self.run_filesystem_shell(
            r#"if [ ! -e "$FIRKIN_FS_SOURCE" ]; then exit 44; fi
mkdir -p -- "$(dirname -- "$FIRKIN_FS_DESTINATION")"
mv -- "$FIRKIN_FS_SOURCE" "$FIRKIN_FS_DESTINATION""#,
            HashMap::from([
                ("FIRKIN_FS_SOURCE", source),
                ("FIRKIN_FS_DESTINATION", destination.clone()),
            ]),
        )
        .await?;
        self.stat_filesystem_entry(&destination).await
    }

    async fn remove_entry(&self, path: String) -> E2bResult<()> {
        self.run_filesystem_shell(
            r#"if [ ! -e "$FIRKIN_FS_PATH" ]; then exit 44; fi
rm -rf -- "$FIRKIN_FS_PATH""#,
            HashMap::from([("FIRKIN_FS_PATH", path)]),
        )
        .await?;
        Ok(())
    }

    async fn stat_entry(&self, path: String) -> E2bResult<EnvdFilesystemEntry> {
        self.stat_filesystem_entry(&path).await
    }

    async fn watch_dir(
        &self,
        _path: String,
        _recursive: bool,
    ) -> E2bResult<Vec<EnvdFilesystemEvent>> {
        Ok(Vec::new())
    }
}

fn envd_normalize_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_owned()
    } else if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

fn envd_path_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("/")
        .to_owned()
}

fn parse_filesystem_entry(line: &str) -> E2bResult<EnvdFilesystemEntry> {
    let line = line.trim_end();
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != 7 {
        return Err(E2bBackendError::Runtime(format!(
            "invalid filesystem stat output `{line}`"
        )));
    }
    let path = envd_normalize_path(fields[0]);
    let file_type = match fields[1] {
        "directory" => EnvdFilesystemFileType::Directory,
        "file" => EnvdFilesystemFileType::File,
        other => {
            return Err(E2bBackendError::Runtime(format!(
                "invalid filesystem entry type `{other}`"
            )));
        }
    };
    let size = fields[2].parse::<i64>().map_err(|error| {
        E2bBackendError::Runtime(format!("invalid filesystem size `{}`: {error}", fields[2]))
    })?;
    let mode = u32::from_str_radix(fields[3], 16).map_err(|error| {
        E2bBackendError::Runtime(format!("invalid filesystem mode `{}`: {error}", fields[3]))
    })?;
    Ok(EnvdFilesystemEntry {
        name: envd_path_name(&path),
        path,
        file_type,
        size,
        mode,
        permissions: fields[4].to_owned(),
        owner: fields[5].to_owned(),
        group: fields[6].to_owned(),
        symlink_target: None,
    })
}

fn runtime_error(operation: &'static str, error: impl std::fmt::Display) -> Error {
    Error::RuntimeLaunchFailed(format!("firkin-apple-vz {operation}: {error}"))
}

fn host_allocated_bytes(path: &Path) -> std::io::Result<u64> {
    let metadata = std::fs::metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(metadata.blocks().saturating_mul(512))
    }
    #[cfg(not(unix))]
    {
        Ok(metadata.len())
    }
}

fn resolve_init_block() -> Result<PathBuf> {
    firkin_ext4::init_block::synthesize(
        firkin_vminitd_bytes::VMINITD_AARCH64,
        firkin_vminitd_bytes::VMEXEC_AARCH64,
    )
    .map_err(|error| runtime_error("synthesize init.block", error))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn single_node_to_backend_error(error: Error) -> E2bBackendError {
    match error {
        Error::SandboxNotFound(message) | Error::SnapshotNotFound(message) => {
            E2bBackendError::NotFound(message)
        }
        Error::Conflict(message) => E2bBackendError::AlreadyExists(message),
        other => E2bBackendError::Runtime(other.to_string()),
    }
}

fn vz_stop_error_means_already_stopped(message: &str) -> bool {
    message.contains("The virtual machine stopped unexpectedly")
}

fn exec_user_from_template_user(user: &str) -> User {
    if let Some((uid, gid)) = user.split_once(':') {
        if let (Ok(uid), Ok(gid)) = (uid.parse::<u32>(), gid.parse::<u32>()) {
            return User::numeric(uid, gid);
        }
    } else if let Ok(uid) = user.parse::<u32>() {
        return User::from(uid);
    }
    User::named(user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use firkin_vmm::DiskImageFormat;

    #[test]
    fn raw_product_pod_store_uses_requested_size() {
        let temp = tempfile::tempdir().unwrap();
        let options = PodStoreOptions {
            size_bytes: 256 * 1024 * 1024,
            image_format: PodStoreImageFormat::Raw,
            trim_policy: PodTrimPolicy::OnRemove,
            shared_rootfs: true,
        };

        let path =
            AppleVzLocalRuntimeDriver::prepare_product_pod_store(&options, temp.path()).unwrap();

        assert_eq!(path.file_name().unwrap(), "pod-store.ext4");
        assert_eq!(std::fs::metadata(path).unwrap().len(), options.size_bytes);
    }

    #[test]
    fn raw_product_pod_store_keeps_empty_sparse_allocation_small() {
        let temp = tempfile::tempdir().unwrap();
        let options = PodStoreOptions {
            size_bytes: 512 * 1024 * 1024,
            image_format: PodStoreImageFormat::Raw,
            trim_policy: PodTrimPolicy::OnRemove,
            shared_rootfs: true,
        };

        let path =
            AppleVzLocalRuntimeDriver::prepare_product_pod_store(&options, temp.path()).unwrap();
        let allocated = host_allocated_bytes(&path).unwrap();

        assert!(
            allocated < 8 * 1024 * 1024,
            "empty raw pod store allocated {allocated} bytes"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn asif_product_pod_store_converts_raw_ext4_and_removes_source() {
        let temp = tempfile::tempdir().unwrap();
        let options = PodStoreOptions {
            image_format: PodStoreImageFormat::Asif,
            size_bytes: 64 * 1024 * 1024,
            ..PodStoreOptions::default()
        };

        let path =
            AppleVzLocalRuntimeDriver::prepare_product_pod_store(&options, temp.path()).unwrap();

        assert_eq!(path.file_name().unwrap(), "pod-store.asif");
        assert!(path.exists());
        assert!(!temp.path().join("pod-store.raw.ext4").exists());
        assert!(std::fs::metadata(path).unwrap().len() > 0);
    }

    #[test]
    fn asif_product_pod_store_vm_config_records_asif_format() {
        let image = tempfile::Builder::new().suffix(".asif").tempfile().unwrap();
        let options = PodStoreOptions {
            image_format: PodStoreImageFormat::Asif,
            ..PodStoreOptions::default()
        };

        let (builder, pod_store_id) = AppleVzLocalRuntimeDriver::add_product_pod_store_device(
            VmConfig::builder(),
            &options,
            image.path(),
        );
        let config = builder.build().unwrap();
        let device = config
            .block_devices()
            .iter()
            .find(|device| device.id() == pod_store_id)
            .unwrap();

        assert_eq!(device.disk_image_format(), DiskImageFormat::Asif);
    }

    #[test]
    fn product_pods_require_shared_rootfs_templates() {
        let temp = tempfile::tempdir().unwrap();
        let options = PodStoreOptions {
            shared_rootfs: false,
            ..PodStoreOptions::default()
        };

        let error = AppleVzLocalRuntimeDriver::prepare_product_pod_store(&options, temp.path())
            .unwrap_err();

        assert!(
            matches!(error, Error::UnsupportedCapability(message) if message.contains("sharedRootfs=true"))
        );
    }

    #[tokio::test]
    async fn preflight_reports_asif_pod_store_support() {
        let capabilities = AppleVzLocalRuntimeDriver::new("busybox")
            .preflight()
            .await
            .unwrap();

        assert!(
            capabilities
                .supported
                .contains(&"pod-store-asif".to_owned())
        );
        assert!(
            !capabilities
                .unsupported
                .iter()
                .any(|(name, _reason)| name == "pod-store-asif")
        );
    }
}
