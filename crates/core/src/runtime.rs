//! runtime — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::await_pty;
#[allow(unused_imports)]
use crate::await_stderr;
#[allow(unused_imports)]
use crate::await_stdin;
#[allow(unused_imports)]
use crate::await_stdout;
#[allow(unused_imports)]
use crate::builder::{
    BuilderState, ContainerBuilder, ContainerStdio, ImplicitStartContext, ImplicitVm, Init, Ready,
    ReadyPty, TRANSIENT_VMNET_BOOT_ATTEMPTS, TRANSIENT_VMNET_BOOT_RETRY_DELAY, VmContext,
    ensure_writable_layer_is_block, guest_socket_staging_path,
};
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use crate::ids::{IntoContainerId, IntoProcessId};
#[allow(unused_imports)]
use crate::io::{
    ChildStderr, ChildStdin, ChildStdout, DnsConfig, EXEC_STDIO_PORT_START, FileMount, HostsConfig,
    Pty, PtyConfig, STDERR_PORT, STDIN_PORT, STDOUT_PORT, SocketDirection, Stdio, Streams,
    prepare_file_mounts,
};
#[allow(unused_imports)]
use crate::prepare_fixed_stderr;
#[allow(unused_imports)]
use crate::prepare_fixed_stdin;
#[allow(unused_imports)]
use crate::prepare_fixed_stdout;
#[allow(unused_imports)]
use crate::process::{
    ExecConfig, ExecStartupTiming, ExitStatus, Output, Process, SIGKILL, SIGTERM, Signal,
};
#[allow(unused_imports)]
use crate::read_pty_optional;
#[allow(unused_imports)]
use crate::read_stderr_optional;
#[allow(unused_imports)]
use crate::read_stdout_optional;
#[allow(unused_imports)]
use crate::rootfs::CopyInPayload;
#[allow(unused_imports)]
use crate::runtime_rpc_error;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::snapshot::ContainerSnapshotState;
#[allow(unused_imports)]
pub use firkin_oci::{
    LinuxSeccompAction as SeccompAction, LinuxSeccompArch as SeccompArch,
    LinuxSeccompArg as SeccompArgRule, LinuxSeccompFlag as SeccompFlag,
    LinuxSeccompOperator as SeccompOp, LinuxSeccompProfile as Seccomp,
    LinuxSyscall as SeccompSyscallRule, Mount,
};
#[allow(unused_imports)]
use firkin_types::BlockDeviceId;
#[allow(unused_imports)]
use firkin_types::VmId;
#[allow(unused_imports)]
use firkin_types::VsockPort;
#[allow(unused_imports)]
use firkin_types::{ContainerId, ProcessId};
#[allow(unused_imports)]
use firkin_vminitd_client::CopyTransfer;
#[allow(unused_imports)]
use firkin_vminitd_client::NetworkConfig;
#[allow(unused_imports)]
use firkin_vminitd_client::ProcessStdio;
#[allow(unused_imports)]
use firkin_vminitd_client::SocketProxy;
#[allow(unused_imports)]
pub use firkin_vminitd_client::{
    BlockIoDevice, BlockIoStatistics, ContainerStatistics, ContainerStatisticsQuery, CpuStatistics,
    MemoryEventStatistics, MemoryStatistics, NetworkStatistics, ProcessStatistics, StatCategory,
};
#[allow(unused_imports)]
use firkin_vminitd_client::{ContainerBundle, RosettaSetup};
#[allow(unused_imports)]
use firkin_vminitd_client::{CopyMetadata, CopyResponseEvent};
#[allow(unused_imports)]
use firkin_vminitd_client::{ProcessCreate, stop_socket_proxy_request};
#[allow(unused_imports)]
use firkin_vmm::NetworkInterface;
#[allow(unused_imports)]
use firkin_vmm::VmConfig;
#[allow(unused_imports)]
use firkin_vmm::{Running, VirtualMachine};
#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use std::collections::HashSet;
#[allow(unused_imports)]
use std::marker::PhantomData;
use std::os::unix::fs::PermissionsExt as _;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::{LazyLock, Mutex as StdMutex};
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use tokio::io::AsyncReadExt;
#[allow(unused_imports)]
use tokio::io::AsyncWriteExt;
#[allow(unused_imports)]
use tokio::io::{AsyncRead, AsyncWrite};
#[allow(unused_imports)]
use tokio::net::UnixListener;
#[allow(unused_imports)]
use tokio::net::UnixStream;
#[allow(unused_imports)]
use tokio::sync::Mutex;
#[allow(unused_imports)]
use tokio::sync::oneshot;
#[allow(unused_imports)]
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use tonic::transport::Channel;
const VMINITD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const VMINITD_CONNECT_RETRY: Duration = Duration::from_millis(100);
static ON_VM_STDIO_PORTS: LazyLock<StdMutex<HashMap<VmId, u32>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));
#[derive(Debug)]
pub(crate) enum RuntimeStaging {
    Temp(tempfile::TempDir),
    Persistent(PathBuf),
}
impl RuntimeStaging {
    pub(crate) fn temp() -> Result<Self> {
        tempfile::tempdir()
            .map(Self::Temp)
            .map_err(|error| Error::RuntimeArtifact {
                operation: "create implicit runtime staging directory",
                reason: error.to_string(),
            })
    }
    pub(crate) fn persistent(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        std::fs::create_dir_all(&path).map_err(|error| Error::RuntimeArtifact {
            operation: "create persistent runtime staging directory",
            reason: error.to_string(),
        })?;
        Ok(Self::Persistent(path))
    }
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Temp(dir) => dir.path(),
            Self::Persistent(path) => path,
        }
    }
    #[cfg(feature = "snapshot")]
    fn path_buf(&self) -> PathBuf {
        self.path().to_path_buf()
    }
}
pub(crate) type VminitdClient =
    firkin_vminitd_client::pb::sandbox_context_client::SandboxContextClient<Channel>;
pub(crate) type FixedStreamTask<T> = (bool, Option<JoinHandle<Result<T>>>);
type OptionalStreamTask<T> = (Option<VsockPort>, Option<JoinHandle<Result<T>>>);
#[derive(Debug)]
pub(crate) struct ContainerRuntime {
    pub(crate) vm: VirtualMachine<Running>,
    pub(crate) client: VminitdClient,
    pub(crate) container_id: ContainerId,
    pub(crate) process_id: ProcessId,
    pub(crate) rootfs_path: String,
    #[cfg_attr(not(feature = "snapshot"), allow(dead_code))]
    pub(crate) staging: RuntimeStaging,
    pub(crate) next_stdio_port: u32,
    pub(crate) stop_vm_on_wait: bool,
    pub(crate) shared_vm_ports: bool,
    pub(crate) stdin_task: Option<JoinHandle<Result<ChildStdin>>>,
    pub(crate) pty_task: Option<JoinHandle<Result<Pty>>>,
    pub(crate) stdout_task: Option<JoinHandle<Result<ChildStdout>>>,
    pub(crate) stderr_task: Option<JoinHandle<Result<ChildStderr>>>,
    pub(crate) socket_tasks: Vec<JoinHandle<Result<()>>>,
    pub(crate) socket_proxy_ids: Vec<String>,
    pub(crate) exit_status: Option<ExitStatus>,
}
impl ContainerRuntime {
    pub(crate) async fn wait(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.exit_status {
            return Ok(status);
        }
        let wait = self
            .client
            .wait_process(tonic::Request::new(ProcessCreate::wait_request(
                &self.process_id,
                Some(&self.container_id),
            )))
            .await
            .map_err(runtime_rpc_error("wait process"))?
            .into_inner();
        let status = ExitStatus::from_code(wait.exit_code);
        self.exit_status = Some(status);
        self.stop_socket_relays().await?;
        let _ = self
            .client
            .delete_process(tonic::Request::new(ProcessCreate::delete_request(
                &self.process_id,
                Some(&self.container_id),
            )))
            .await;
        if self.stop_vm_on_wait {
            self.vm
                .clone()
                .stop_with_grace(Duration::from_millis(100))
                .await
                .map_err(runtime_vmm_error("stop VM"))?;
        }
        Ok(status)
    }
    async fn stop_socket_relays(&mut self) -> Result<()> {
        for id in std::mem::take(&mut self.socket_proxy_ids) {
            let _ = self
                .client
                .stop_vsock_proxy(tonic::Request::new(stop_socket_proxy_request(id)))
                .await;
        }
        for task in self.socket_tasks.drain(..) {
            task.abort();
        }
        Ok(())
    }
    pub(crate) async fn kill(&mut self, signal: Signal) -> Result<()> {
        self.client
            .kill_process(tonic::Request::new(ProcessCreate::kill_request(
                &self.process_id,
                Some(&self.container_id),
                signal.get(),
            )))
            .await
            .map_err(runtime_rpc_error("kill process"))?;
        Ok(())
    }
    pub(crate) async fn stop_with_grace(&mut self, grace: Duration) -> Result<ExitStatus> {
        if let Some(status) = self.exit_status {
            return Ok(status);
        }
        self.kill(SIGTERM).await?;
        tokio::time::sleep(grace).await;
        let _ = self.kill(SIGKILL).await;
        self.wait().await
    }
    fn allocate_host_port(&mut self, operation: &'static str) -> Result<VsockPort> {
        if self.shared_vm_ports {
            return allocate_vm_stdio_port(self.vm.id(), operation);
        }
        let port = self.next_stdio_port;
        self.next_stdio_port = port.checked_add(1).ok_or_else(|| Error::RuntimeOperation {
            operation,
            reason: "host vsock port range exhausted".to_owned(),
        })?;
        Ok(VsockPort::new(port))
    }
}
#[derive(Debug)]
pub(crate) struct ProcessRuntime {
    container_runtime: Arc<Mutex<ContainerRuntime>>,
    pub(crate) container_id: ContainerId,
    pub(crate) process_id: ProcessId,
    pub(crate) stdin_task: Option<JoinHandle<Result<ChildStdin>>>,
    pub(crate) pty_task: Option<JoinHandle<Result<Pty>>>,
    pub(crate) stdout_task: Option<JoinHandle<Result<ChildStdout>>>,
    pub(crate) stderr_task: Option<JoinHandle<Result<ChildStderr>>>,
    pub(crate) exit_status: Option<ExitStatus>,
}
impl ProcessRuntime {
    pub(crate) async fn wait(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.exit_status {
            return Ok(status);
        }
        let (mut client, process_id, container_id) = {
            let runtime = self.container_runtime.lock().await;
            (
                runtime.client.clone(),
                self.process_id.clone(),
                self.container_id.clone(),
            )
        };
        let wait = client
            .wait_process(tonic::Request::new(ProcessCreate::wait_request(
                &process_id,
                Some(&container_id),
            )))
            .await
            .map_err(runtime_rpc_error("wait exec process"))?
            .into_inner();
        let status = ExitStatus::from_code(wait.exit_code);
        self.exit_status = Some(status);
        let _ = client
            .delete_process(tonic::Request::new(ProcessCreate::delete_request(
                &process_id,
                Some(&container_id),
            )))
            .await;
        Ok(status)
    }
    pub(crate) async fn kill(&mut self, signal: Signal) -> Result<()> {
        let (mut client, process_id, container_id) = {
            let runtime = self.container_runtime.lock().await;
            (
                runtime.client.clone(),
                self.process_id.clone(),
                self.container_id.clone(),
            )
        };
        client
            .kill_process(tonic::Request::new(ProcessCreate::kill_request(
                &process_id,
                Some(&container_id),
                signal.get(),
            )))
            .await
            .map_err(runtime_rpc_error("kill exec process"))?;
        Ok(())
    }
}
#[derive(Debug)]
struct ProcessStdioPlan {
    stdio: ProcessStdio,
    pub(crate) stdin_task: Option<JoinHandle<Result<ChildStdin>>>,
    pub(crate) pty_task: Option<JoinHandle<Result<Pty>>>,
    pub(crate) stdout_task: Option<JoinHandle<Result<ChildStdout>>>,
    pub(crate) stderr_task: Option<JoinHandle<Result<ChildStderr>>>,
}
/// Live container handle.
#[derive(Debug)]
pub struct Container<S = Streams> {
    pub(crate) id: ContainerId,
    pub(crate) pid: Option<i32>,
    pub(crate) pty: Option<Pty>,
    pub(crate) runtime: Arc<Mutex<ContainerRuntime>>,
    pub(crate) startup_timing: ContainerStartupTiming,
    pub(crate) state: PhantomData<S>,
}

/// Host-side phase timing for starting a container init process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContainerStartupTiming {
    spec_build: Duration,
    vminitd_connect: Duration,
    socket_relays: Duration,
    stdio_prepare: Duration,
    config_write_rpc: Duration,
    request_encode: Duration,
    create_process_rpc: Duration,
    start_gate_wait: Duration,
    start_process_rpc: Duration,
    total: Duration,
}

impl ContainerStartupTiming {
    /// Construct host-side phase timing for a container init process start.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        spec_build: Duration,
        vminitd_connect: Duration,
        socket_relays: Duration,
        stdio_prepare: Duration,
        config_write_rpc: Duration,
        request_encode: Duration,
        create_process_rpc: Duration,
        start_gate_wait: Duration,
        start_process_rpc: Duration,
        total: Duration,
    ) -> Self {
        Self {
            spec_build,
            vminitd_connect,
            socket_relays,
            stdio_prepare,
            config_write_rpc,
            request_encode,
            create_process_rpc,
            start_gate_wait,
            start_process_rpc,
            total,
        }
    }

    /// Time spent building the OCI runtime spec.
    #[must_use]
    pub const fn spec_build(self) -> Duration {
        self.spec_build
    }

    /// Time spent connecting to vminitd for this start operation.
    #[must_use]
    pub const fn vminitd_connect(self) -> Duration {
        self.vminitd_connect
    }

    /// Time spent preparing socket relays.
    #[must_use]
    pub const fn socket_relays(self) -> Duration {
        self.socket_relays
    }

    /// Time spent preparing host stdio listeners/tasks.
    #[must_use]
    pub const fn stdio_prepare(self) -> Duration {
        self.stdio_prepare
    }

    /// Time spent writing the prepared container config to the guest.
    #[must_use]
    pub const fn config_write_rpc(self) -> Duration {
        self.config_write_rpc
    }

    /// Time spent encoding the vminitd create-process request.
    #[must_use]
    pub const fn request_encode(self) -> Duration {
        self.request_encode
    }

    /// Time spent in the vminitd create-process RPC.
    #[must_use]
    pub const fn create_process_rpc(self) -> Duration {
        self.create_process_rpc
    }

    /// Time spent waiting for the optional process start gate.
    #[must_use]
    pub const fn start_gate_wait(self) -> Duration {
        self.start_gate_wait
    }

    /// Time spent in the vminitd start-process RPC.
    #[must_use]
    pub const fn start_process_rpc(self) -> Duration {
        self.start_process_rpc
    }

    /// Total host-side start time covered by this timing record.
    #[must_use]
    pub const fn total(self) -> Duration {
        self.total
    }
}

impl Container<Streams> {
    /// Start a builder with an implicit single-use VM.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidContainerId`] when `id` fails validation.
    pub fn builder(id: impl IntoContainerId) -> Result<ContainerBuilder<ImplicitVm, Init>> {
        Ok(ContainerBuilder::new(id.into_container_id()?))
    }
}
impl<S: ContainerStdio> Container<S> {
    /// Return the container ID.
    #[must_use]
    pub const fn id(&self) -> &ContainerId {
        &self.id
    }
    /// Return the guest-side init process PID, if known.
    #[must_use]
    pub const fn pid(&self) -> Option<i32> {
        self.pid
    }
    /// Return host-side phase timing for this container init process start.
    #[must_use]
    pub const fn startup_timing(&self) -> ContainerStartupTiming {
        self.startup_timing
    }
    /// Wait for the init process to exit.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if the vminitd wait RPC or implicit VM cleanup
    /// fails.
    pub async fn wait(&mut self) -> Result<ExitStatus> {
        let mut runtime = self.runtime.lock().await;
        runtime.wait().await
    }
    /// Wait for the init process and collect stdout and stderr.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if process wait, stdio capture, or implicit VM
    /// cleanup fails.
    pub async fn wait_with_output(mut self) -> Result<Output> {
        let pty = self.pty.take();
        let (pty_task, stdout_task, stderr_task) = {
            let mut runtime = self.runtime.lock().await;
            (
                runtime.pty_task.take(),
                runtime.stdout_task.take(),
                runtime.stderr_task.take(),
            )
        };
        let wait = async {
            let mut runtime = self.runtime.lock().await;
            runtime.wait().await
        };
        let (status, stdout, stderr) = if S::TERMINAL {
            let (status, stdout) =
                tokio::try_join!(wait, read_pty_optional(pty, pty_task, "capture pty"))?;
            (status, stdout, Vec::new())
        } else {
            tokio::try_join!(
                wait,
                read_stdout_optional(stdout_task, "capture stdout"),
                read_stderr_optional(stderr_task, "capture stderr")
            )?
        };
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
    /// Send a signal to the init process.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd rejects the kill RPC.
    pub async fn kill(&mut self, signal: Signal) -> Result<()> {
        let mut runtime = self.runtime.lock().await;
        runtime.kill(signal).await
    }
    /// Take the init process stdin handle, if stdin was configured as piped.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd never connects the stdin stream or
    /// the accept task fails.
    pub async fn take_stdin(&mut self) -> Result<Option<ChildStdin>> {
        let stdin_task = {
            let mut runtime = self.runtime.lock().await;
            runtime.stdin_task.take()
        };
        match stdin_task {
            Some(task) => await_stdin(task, "accept stdin").await.map(Some),
            None => Ok(None),
        }
    }
    /// Take the init process stdout handle, if stdout was configured as piped.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd never connects the stdout stream or
    /// the accept task fails.
    pub async fn take_stdout(&mut self) -> Result<Option<ChildStdout>> {
        let stdout_task = {
            let mut runtime = self.runtime.lock().await;
            runtime.stdout_task.take()
        };
        match stdout_task {
            Some(task) => await_stdout(task, "accept stdout").await.map(Some),
            None => Ok(None),
        }
    }
    /// Take the init process stderr handle, if stderr was configured as piped.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd never connects the stderr stream or
    /// the accept task fails.
    pub async fn take_stderr(&mut self) -> Result<Option<ChildStderr>> {
        let stderr_task = {
            let mut runtime = self.runtime.lock().await;
            runtime.stderr_task.take()
        };
        match stderr_task {
            Some(task) => await_stderr(task, "accept stderr").await.map(Some),
            None => Ok(None),
        }
    }
    /// Stop the container using the default grace period.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if process wait or implicit VM cleanup fails.
    pub async fn stop(self) -> Result<ExitStatus> {
        self.stop_with_grace(Duration::from_secs(10)).await
    }
    /// Stop the container, escalating from SIGTERM to SIGKILL after `grace`.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if the kill, wait, or implicit VM cleanup path
    /// fails.
    pub async fn stop_with_grace(self, grace: Duration) -> Result<ExitStatus> {
        let mut runtime = self.runtime.lock().await;
        runtime.stop_with_grace(grace).await
    }
    /// Pause the underlying VM.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if Virtualization.framework rejects the pause.
    pub async fn pause(&mut self) -> Result<()> {
        let runtime = self.runtime.lock().await;
        runtime
            .vm
            .pause()
            .await
            .map_err(runtime_vmm_error("pause VM"))
    }
    /// Resume the underlying VM.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if Virtualization.framework rejects the resume.
    pub async fn resume(&mut self) -> Result<()> {
        let runtime = self.runtime.lock().await;
        runtime
            .vm
            .resume()
            .await
            .map_err(runtime_vmm_error("resume VM"))
    }
    /// Save the underlying VM state to `path`.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if Virtualization.framework rejects the snapshot
    /// operation or if this build does not include the `snapshot` feature.
    #[cfg(feature = "snapshot")]
    pub async fn save_snapshot(&self, path: impl AsRef<Path>) -> Result<()> {
        let runtime = self.runtime.lock().await;
        runtime
            .vm
            .save_snapshot(path)
            .await
            .map_err(runtime_vmm_error("save VM snapshot"))
    }
    /// Return the state that must be persisted beside a VM snapshot.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if the container was not spawned with persistent
    /// staging.
    #[cfg(feature = "snapshot")]
    pub async fn snapshot_state(&self) -> Result<ContainerSnapshotState> {
        let runtime = self.runtime.lock().await;
        if matches!(runtime.staging, RuntimeStaging::Temp(_)) {
            return Err(Error::RuntimeOperation {
                operation: "read snapshot restore state",
                reason: "container was not spawned with persistent staging".to_owned(),
            });
        }
        Ok(ContainerSnapshotState {
            staging_dir: runtime.staging.path_buf(),
            machine_identifier: runtime.vm.config().machine_identifier().to_vec(),
            network_macs: runtime.vm.config().network_macs().to_vec(),
        })
    }
    /// Dial a guest-side vsock port scoped to this container's VM.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if the port is reserved or Virtualization.framework
    /// cannot establish the connection.
    pub async fn dial_vsock(&self, port: VsockPort) -> Result<firkin_vsock::VsockStream> {
        let vm = {
            let runtime = self.runtime.lock().await;
            runtime.vm.clone()
        };
        vm.dial(port)
            .await
            .map_err(runtime_vmm_error("dial container vsock"))
    }
    /// Request point-in-time resource statistics for this container.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd rejects the statistics RPC or does not
    /// return statistics for this container.
    pub async fn statistics(&self, categories: StatCategory) -> Result<ContainerStatistics> {
        let container_id = self.id.to_string();
        let mut client = {
            let runtime = self.runtime.lock().await;
            runtime.client.clone()
        };
        let response = client
            .container_statistics(tonic::Request::new(
                ContainerStatisticsQuery::new([container_id.clone()])
                    .categories(categories)
                    .into_request(),
            ))
            .await
            .map_err(runtime_rpc_error("container statistics"))?
            .into_inner();
        ContainerStatistics::list_from_response(response, categories)
            .into_iter()
            .find(|stats| stats.id == container_id)
            .ok_or_else(|| Error::RuntimeOperation {
                operation: "container statistics",
                reason: format!("vminitd did not return statistics for {container_id}"),
            })
    }
    /// Copy one host file into this container's rootfs.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if the host file cannot be read, the guest path
    /// cannot be represented for vminitd, or the vminitd copy operation fails.
    pub async fn copy_in(
        &self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<()> {
        let source = source.as_ref().to_path_buf();
        let metadata = tokio::fs::metadata(&source)
            .await
            .map_err(io_runtime_error("stat copy-in source"))?;
        let payload = prepare_copy_in_payload(source, metadata).await?;
        let guest_path = container_guest_path(&self.id, destination.as_ref())?;
        let (listener, client, port) = {
            let mut runtime = self.runtime.lock().await;
            let port = runtime.allocate_host_port("allocate copy-in port")?;
            let listener = runtime
                .vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error("listen for copy in"))?;
            (listener, runtime.client.clone(), port)
        };
        let request = CopyTransfer::copy_in(guest_path, port)
            .mode(payload.mode())
            .create_parents(true)
            .archive(payload.is_archive())
            .into_request();
        let copy_task = tokio::spawn(run_copy_control(client, request, None));
        let (mut stream, _peer) = listener
            .accept()
            .await
            .map_err(vsock_runtime_error("accept copy-in connection"))?;
        let mut file = tokio::fs::File::open(payload.path())
            .await
            .map_err(io_runtime_error("open copy-in source"))?;
        tokio::io::copy(&mut file, &mut stream)
            .await
            .map_err(io_runtime_error("stream copy-in source"))?;
        stream
            .shutdown()
            .await
            .map_err(io_runtime_error("finish copy-in stream"))?;
        await_copy_control(copy_task, "copy in").await?;
        Ok(())
    }
    /// Copy one guest file out of this container's rootfs.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if the destination cannot be written or vminitd
    /// reports a failed copy operation.
    pub async fn copy_out(
        &self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<()> {
        let destination = destination.as_ref().to_path_buf();
        if let Some(parent) = destination.parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(io_runtime_error("create copy-out parent directory"))?;
        }
        let guest_path = container_guest_path(&self.id, source.as_ref())?;
        let (listener, client, port) = {
            let mut runtime = self.runtime.lock().await;
            let port = runtime.allocate_host_port("allocate copy-out port")?;
            let listener = runtime
                .vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error("listen for copy out"))?;
            (listener, runtime.client.clone(), port)
        };
        let request = CopyTransfer::copy_out(guest_path, port).into_request();
        let (metadata_tx, metadata_rx) = oneshot::channel();
        let copy_task = tokio::spawn(run_copy_control(client, request, Some(metadata_tx)));
        let metadata = metadata_rx
            .await
            .map_err(|error| Error::RuntimeOperation {
                operation: "copy out metadata",
                reason: error.to_string(),
            })??;
        let (mut stream, _peer) = listener
            .accept()
            .await
            .map_err(vsock_runtime_error("accept copy-out connection"))?;
        if metadata.is_archive {
            tokio::fs::create_dir_all(&destination)
                .await
                .map_err(io_runtime_error("create copy-out destination directory"))?;
            let archive =
                tempfile::NamedTempFile::new().map_err(|error| Error::RuntimeOperation {
                    operation: "create copy-out archive",
                    reason: error.to_string(),
                })?;
            let mut file = tokio::fs::File::create(archive.path())
                .await
                .map_err(io_runtime_error("create copy-out archive"))?;
            tokio::io::copy(&mut stream, &mut file)
                .await
                .map_err(io_runtime_error("stream copy-out archive"))?;
            file.flush()
                .await
                .map_err(io_runtime_error("flush copy-out archive"))?;
            drop(file);
            await_copy_control(copy_task, "copy out").await?;
            extract_directory_archive(archive.path().to_path_buf(), destination).await?;
            return Ok(());
        }
        let mut file = tokio::fs::File::create(&destination)
            .await
            .map_err(io_runtime_error("create copy-out destination"))?;
        tokio::io::copy(&mut stream, &mut file)
            .await
            .map_err(io_runtime_error("stream copy-out destination"))?;
        file.flush()
            .await
            .map_err(io_runtime_error("flush copy-out destination"))?;
        await_copy_control(copy_task, "copy out").await?;
        Ok(())
    }
    /// Exec an additional process inside this container.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd rejects process creation or startup.
    pub async fn exec<E: ContainerStdio>(
        &mut self,
        id: impl IntoProcessId,
        config: ExecConfig<E>,
    ) -> Result<Process<E>> {
        let process_id = id.into_process_id()?;
        let container_id = self.id.clone();
        let mut runtime = self.runtime.lock().await;
        let spec_started = Instant::now();
        let spec = config.runtime_spec_with_root_path(
            &container_id,
            runtime.rootfs_path.as_str(),
            E::TERMINAL,
        )?;
        let spec_build = spec_started.elapsed();
        let stdio_started = Instant::now();
        let stdio_plan = prepare_exec_stdio(&mut runtime, &config, &container_id, &process_id)?;
        let stdio_prepare = stdio_started.elapsed();
        let ProcessStdioPlan {
            stdio,
            stdin_task,
            pty_task,
            stdout_task,
            stderr_task,
        } = stdio_plan;
        let encode_started = Instant::now();
        let create = ProcessCreate::new(process_id.clone(), container_id.clone(), spec)
            .stdio(stdio)
            .into_request()
            .map_err(runtime_vminitd_error("encode exec process request"))?;
        let request_encode = encode_started.elapsed();
        let create_started = Instant::now();
        runtime
            .client
            .create_process(tonic::Request::new(create))
            .await
            .map_err(runtime_rpc_error("create exec process"))?;
        let create_process_rpc = create_started.elapsed();
        let start_started = Instant::now();
        let start = runtime
            .client
            .start_process(tonic::Request::new(ProcessCreate::start_request(
                &process_id,
                Some(&container_id),
            )))
            .await
            .map_err(runtime_rpc_error("start exec process"))?
            .into_inner();
        let start_process_rpc = start_started.elapsed();
        let pty = if E::TERMINAL {
            match pty_task {
                Some(task) => Some(await_pty(task, "accept exec pty").await?),
                None => None,
            }
        } else {
            None
        };
        Ok(Process {
            id: process_id.clone(),
            pid: Some(start.pid),
            pty,
            runtime: Arc::new(Mutex::new(ProcessRuntime {
                container_runtime: Arc::clone(&self.runtime),
                container_id,
                process_id,
                stdin_task,
                pty_task: None,
                stdout_task,
                stderr_task,
                exit_status: None,
            })),
            startup_timing: ExecStartupTiming::new(
                spec_build,
                stdio_prepare,
                request_encode,
                create_process_rpc,
                start_process_rpc,
            ),
            state: PhantomData,
        })
    }
}
impl<S> Container<S> {
    /// Return guest-visible VM network interfaces for this container.
    pub async fn network_interfaces(&self) -> Vec<NetworkInterface> {
        self.runtime.lock().await.vm.network_interfaces().to_vec()
    }
}
impl Container<Pty> {
    /// Return the init process pty handle.
    ///
    /// # Panics
    ///
    /// Panics if the pty handle has already been consumed by [`Self::take_pty`].
    #[must_use]
    pub fn pty(&mut self) -> &mut Pty {
        self.pty
            .as_mut()
            .expect("Container<Pty> pty handle was already taken")
    }
    /// Take the init process pty handle.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd never connects the terminal streams
    /// or the accept task fails.
    pub async fn take_pty(&mut self) -> Result<Option<Pty>> {
        if self.pty.is_some() {
            return Ok(self.pty.take());
        }
        let pty_task = {
            let mut runtime = self.runtime.lock().await;
            runtime.pty_task.take()
        };
        match pty_task {
            Some(task) => await_pty(task, "accept pty").await.map(Some),
            None => Ok(None),
        }
    }
}
async fn prepare_implicit_start(
    builder: ContainerBuilder<ImplicitVm, Ready>,
    terminal: bool,
) -> Result<ImplicitStartContext> {
    prepare_implicit_start_with_staging(builder, terminal, RuntimeStaging::temp()?).await
}
async fn prepare_implicit_start_with_staging(
    builder: ContainerBuilder<ImplicitVm, Ready>,
    terminal: bool,
    staging: RuntimeStaging,
) -> Result<ImplicitStartContext> {
    let prepared = builder.prepare_implicit_vm_in(staging.path())?;
    let spec = builder.runtime_spec_with_terminal(terminal)?;
    let rosetta_enabled = prepared.config().rosetta_enabled();
    let process_id =
        ProcessId::new(builder.id.to_string()).map_err(|error| Error::RuntimeOperation {
            operation: "build process id",
            reason: error.to_string(),
        })?;
    let vm = boot_vm_with_transient_network_retry(prepared.config)
        .await
        .map_err(runtime_vmm_error("boot VM"))?;
    let mut client = connect_vminitd(&vm).await?;
    standard_guest_setup(&mut client).await?;
    let bundle = ContainerBundle::for_id(&builder.id);
    client
        .mkdir(tonic::Request::new(bundle.mkdir_rootfs_request(0o755)))
        .await
        .map_err(runtime_rpc_error("mkdir rootfs"))?;
    let rootfs_source = block_device_guest_path(prepared.rootfs_device)?;
    let writable_layer_source = prepared
        .writable_layer_device
        .map(block_device_guest_path)
        .transpose()?;
    mount_container_rootfs(
        &mut client,
        &bundle,
        &rootfs_source,
        builder.writable_layer.as_ref(),
        writable_layer_source.as_deref(),
    )
    .await?;
    if rosetta_enabled {
        RosettaSetup::amd64()
            .apply_to(&mut client)
            .await
            .map_err(runtime_vminitd_error("configure rosetta"))?;
    }
    configure_guest_networks(&vm, &mut client, bundle.rootfs_path()).await?;
    configure_container_name_files(
        &mut client,
        bundle.rootfs_path(),
        builder.dns.as_ref(),
        builder.hosts.as_ref(),
    )
    .await?;
    mount_file_mount_holding_dirs(&mut client, &builder.file_mounts).await?;
    let mut next_stdio_port = EXEC_STDIO_PORT_START;
    let socket_relays = prepare_socket_relays(
        &vm,
        &mut client,
        &builder,
        &bundle,
        false,
        &mut next_stdio_port,
    )
    .await?;
    client
        .write_file(tonic::Request::new(
            bundle
                .write_config_request(&spec)
                .map_err(runtime_vminitd_error("encode runtime spec"))?,
        ))
        .await
        .map_err(runtime_rpc_error("write config.json"))?;
    Ok(ImplicitStartContext {
        staging,
        vm,
        client,
        spec,
        process_id,
        socket_tasks: socket_relays.tasks,
        socket_proxy_ids: socket_relays.proxy_ids,
        next_stdio_port,
    })
}
pub(crate) async fn start_implicit_container(
    builder: ContainerBuilder<ImplicitVm, Ready>,
) -> Result<Container<Streams>> {
    let context = prepare_implicit_start(builder.clone(), false).await?;
    finish_implicit_container_start(builder, context).await
}
pub(crate) async fn start_implicit_container_with_staging(
    builder: ContainerBuilder<ImplicitVm, Ready>,
    staging: RuntimeStaging,
) -> Result<Container<Streams>> {
    let context = prepare_implicit_start_with_staging(builder.clone(), false, staging).await?;
    finish_implicit_container_start(builder, context).await
}
async fn finish_implicit_container_start(
    builder: ContainerBuilder<ImplicitVm, Ready>,
    context: ImplicitStartContext,
) -> Result<Container<Streams>> {
    let ImplicitStartContext {
        staging,
        vm,
        mut client,
        spec,
        process_id,
        socket_tasks,
        socket_proxy_ids,
        next_stdio_port,
    } = context;
    let stdin_plan = prepare_fixed_stdin(&vm, builder.stdin, STDIN_PORT, "listen for stdin")?;
    let stdout_plan = prepare_fixed_stdout(&vm, builder.stdout, STDOUT_PORT, "listen for stdout")?;
    let stderr_plan = prepare_fixed_stderr(&vm, builder.stderr, STDERR_PORT, "listen for stderr")?;
    let mut stdio = ProcessStdio::new();
    if stdin_plan.0 {
        stdio = stdio.stdin(STDIN_PORT);
    }
    if stdout_plan.0 {
        stdio = stdio.stdout(STDOUT_PORT);
    }
    if stderr_plan.0 {
        stdio = stdio.stderr(STDERR_PORT);
    }
    let create = ProcessCreate::new(process_id.clone(), builder.id.clone(), spec)
        .stdio(stdio)
        .into_request()
        .map_err(runtime_vminitd_error("encode create process request"))?;
    client
        .create_process(tonic::Request::new(create))
        .await
        .map_err(runtime_rpc_error("create process"))?;
    let start = client
        .start_process(tonic::Request::new(ProcessCreate::start_request(
            &process_id,
            Some(&builder.id),
        )))
        .await
        .map_err(runtime_rpc_error("start process"))?
        .into_inner();
    let rootfs_path = ContainerBundle::for_id(&builder.id)
        .rootfs_path()
        .to_owned();
    Ok(Container {
        id: builder.id.clone(),
        pid: Some(start.pid),
        pty: None,
        runtime: Arc::new(Mutex::new(ContainerRuntime {
            vm,
            client,
            container_id: builder.id,
            process_id,
            rootfs_path,
            staging,
            next_stdio_port,
            stop_vm_on_wait: true,
            shared_vm_ports: false,
            stdin_task: stdin_plan.1,
            pty_task: None,
            stdout_task: stdout_plan.1,
            stderr_task: stderr_plan.1,
            socket_tasks,
            socket_proxy_ids,
            exit_status: None,
        })),
        startup_timing: ContainerStartupTiming::default(),
        state: PhantomData,
    })
}
pub(crate) async fn start_implicit_container_pty(
    builder: ContainerBuilder<ImplicitVm, ReadyPty>,
) -> Result<Container<Pty>> {
    let stream_builder = builder.as_stream_ready();
    let context = prepare_implicit_start(stream_builder, true).await?;
    let ImplicitStartContext {
        staging,
        vm,
        mut client,
        spec,
        process_id,
        socket_tasks,
        socket_proxy_ids,
        next_stdio_port,
    } = context;
    let input_listener = vm
        .listen_reserved_port(STDIN_PORT)
        .map_err(runtime_vmm_error("listen for pty input"))?;
    let output_listener = vm
        .listen_reserved_port(STDOUT_PORT)
        .map_err(runtime_vmm_error("listen for pty output"))?;
    let pty_task = tokio::spawn(accept_pty(
        input_listener,
        output_listener,
        builder.pty.unwrap_or_default(),
        client.clone(),
        builder.id.clone(),
        process_id.clone(),
    ));
    let create = ProcessCreate::new(process_id.clone(), builder.id.clone(), spec)
        .stdio(ProcessStdio::new().stdin(STDIN_PORT).stdout(STDOUT_PORT))
        .into_request()
        .map_err(runtime_vminitd_error("encode create process request"))?;
    client
        .create_process(tonic::Request::new(create))
        .await
        .map_err(runtime_rpc_error("create process"))?;
    let start = client
        .start_process(tonic::Request::new(ProcessCreate::start_request(
            &process_id,
            Some(&builder.id),
        )))
        .await
        .map_err(runtime_rpc_error("start process"))?
        .into_inner();
    let pty = await_pty(pty_task, "accept pty").await?;
    let rootfs_path = ContainerBundle::for_id(&builder.id)
        .rootfs_path()
        .to_owned();
    Ok(Container {
        id: builder.id.clone(),
        pid: Some(start.pid),
        pty: Some(pty),
        runtime: Arc::new(Mutex::new(ContainerRuntime {
            vm,
            client,
            container_id: builder.id,
            process_id,
            rootfs_path,
            staging,
            next_stdio_port,
            stop_vm_on_wait: true,
            shared_vm_ports: false,
            stdin_task: None,
            pty_task: None,
            stdout_task: None,
            stderr_task: None,
            socket_tasks,
            socket_proxy_ids,
            exit_status: None,
        })),
        startup_timing: ContainerStartupTiming::default(),
        state: PhantomData,
    })
}
pub(crate) struct SocketRelayPlan {
    pub(crate) tasks: Vec<JoinHandle<Result<()>>>,
    pub(crate) proxy_ids: Vec<String>,
}
pub(crate) async fn prepare_socket_relays<Vm: VmContext, S: BuilderState>(
    vm: &VirtualMachine<Running>,
    client: &mut VminitdClient,
    builder: &ContainerBuilder<Vm, S>,
    _bundle: &ContainerBundle,
    shared_vm_ports: bool,
    next_port: &mut u32,
) -> Result<SocketRelayPlan> {
    let mut tasks = Vec::new();
    let mut proxy_ids = Vec::new();
    for socket in &builder.sockets {
        let port = allocate_socket_relay_port(vm, shared_vm_ports, next_port)?;
        let (request, task) = match socket.direction {
            SocketDirection::Into => {
                let guest_path = guest_socket_staging_path(&builder.id, &socket.id);
                let listener = vm
                    .listen_reserved_port(port)
                    .map_err(runtime_vmm_error("listen for socket relay"))?;
                let task = tokio::spawn(relay_guest_vsock_to_host_unix(
                    listener,
                    socket.source.clone(),
                ));
                let request = SocketProxy::into_guest(&socket.id, port, Path::new(&guest_path))
                    .permissions(socket.permissions)
                    .into_request();
                (request, task)
            }
            SocketDirection::OutOf => {
                let guest_path = container_guest_path(&builder.id, &socket.source)?;
                let listener = bind_host_unix_listener(&socket.destination, socket.permissions)?;
                let task = tokio::spawn(relay_host_unix_to_guest_vsock(listener, vm.clone(), port));
                let request = SocketProxy::out_of_guest(&socket.id, port, Path::new(&guest_path))
                    .permissions(socket.permissions)
                    .into_request();
                (request, task)
            }
        };
        if let Err(error) = client.proxy_vsock(tonic::Request::new(request)).await {
            task.abort();
            return Err(runtime_rpc_error("proxy socket relay")(error));
        }
        proxy_ids.push(socket.id.clone());
        tasks.push(task);
    }
    Ok(SocketRelayPlan { tasks, proxy_ids })
}
fn allocate_socket_relay_port(
    vm: &VirtualMachine<Running>,
    shared_vm_ports: bool,
    next_port: &mut u32,
) -> Result<VsockPort> {
    if shared_vm_ports {
        return allocate_vm_stdio_port(vm.id(), "allocate socket relay port");
    }
    let port = *next_port;
    *next_port = port.checked_add(1).ok_or_else(|| Error::RuntimeOperation {
        operation: "allocate socket relay port",
        reason: "socket relay port range exhausted".to_owned(),
    })?;
    Ok(VsockPort::new(port))
}
fn bind_host_unix_listener(path: &Path, permissions: Option<u32>) -> Result<UnixListener> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(io_runtime_error("create socket relay parent"))?;
    }
    if path.exists() {
        std::fs::remove_file(path).map_err(io_runtime_error("remove existing socket relay"))?;
    }
    let listener = UnixListener::bind(path).map_err(io_runtime_error("bind socket relay"))?;
    if let Some(mode) = permissions {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(io_runtime_error("set socket relay permissions"))?;
    }
    Ok(listener)
}
pub(crate) fn allocate_vm_stdio_port(vm_id: &VmId, operation: &'static str) -> Result<VsockPort> {
    let mut ports = ON_VM_STDIO_PORTS
        .lock()
        .map_err(|error| Error::RuntimeOperation {
            operation,
            reason: error.to_string(),
        })?;
    let next = ports.entry(*vm_id).or_insert(EXEC_STDIO_PORT_START);
    let port = *next;
    *next = next.checked_add(1).ok_or_else(|| Error::RuntimeOperation {
        operation,
        reason: "host vsock port range exhausted".to_owned(),
    })?;
    Ok(VsockPort::new(port))
}
pub(crate) async fn configure_guest_networks(
    vm: &VirtualMachine<Running>,
    client: &mut VminitdClient,
    dns_location: &str,
) -> Result<()> {
    for interface in vm.network_interfaces() {
        let config = NetworkConfig::new(
            interface.ipv4_address(),
            interface.prefix(),
            interface.gateway(),
            dns_location,
            [interface.gateway().to_string(), "8.8.8.8".to_owned()],
        );
        config
            .apply_to(client, interface.name())
            .await
            .map_err(runtime_vminitd_error("configure guest network"))?;
    }
    Ok(())
}
pub(crate) async fn configure_container_name_files(
    client: &mut VminitdClient,
    location: &str,
    dns: Option<&DnsConfig>,
    hosts: Option<&HostsConfig>,
) -> Result<()> {
    if let Some(dns) = dns {
        client
            .configure_dns(tonic::Request::new(configure_dns_request(dns, location)))
            .await
            .map_err(runtime_rpc_error("configure DNS"))?;
    }
    if let Some(hosts) = hosts {
        client
            .configure_hosts(tonic::Request::new(configure_hosts_request(
                hosts, location,
            )))
            .await
            .map_err(runtime_rpc_error("configure hosts"))?;
    }
    Ok(())
}
pub(crate) fn configure_dns_request(
    dns: &DnsConfig,
    location: impl Into<String>,
) -> firkin_vminitd_client::pb::ConfigureDnsRequest {
    firkin_vminitd_client::pb::ConfigureDnsRequest {
        location: location.into(),
        nameservers: dns.nameservers.iter().map(ToString::to_string).collect(),
        domain: dns.domain.clone(),
        search_domains: dns.search.clone(),
        options: dns.options.clone(),
    }
}
pub(crate) fn configure_hosts_request(
    hosts: &HostsConfig,
    location: impl Into<String>,
) -> firkin_vminitd_client::pb::ConfigureHostsRequest {
    firkin_vminitd_client::pb::ConfigureHostsRequest {
        location: location.into(),
        entries: hosts
            .entries
            .iter()
            .map(
                |entry| firkin_vminitd_client::pb::configure_hosts_request::HostsEntry {
                    ip_address: entry.ip.to_string(),
                    hostnames: entry.hostnames.clone(),
                    comment: entry.comment.clone(),
                },
            )
            .collect(),
        comment: hosts.comment.clone(),
    }
}
pub(crate) async fn mount_file_mount_holding_dirs(
    client: &mut VminitdClient,
    file_mounts: &[FileMount],
) -> Result<()> {
    let file_mounts = prepare_file_mounts(file_mounts)?;
    let mut mounted = HashSet::new();
    for file_mount in file_mounts {
        if !mounted.insert(file_mount.tag.clone()) {
            continue;
        }
        client
            .mkdir(tonic::Request::new(
                firkin_vminitd_client::pb::MkdirRequest {
                    path: file_mount.guest_holding_path.clone(),
                    all: true,
                    perms: 0o755,
                },
            ))
            .await
            .map_err(runtime_rpc_error("mkdir file mount holding directory"))?;
        client
            .mount(tonic::Request::new(
                firkin_vminitd_client::pb::MountRequest {
                    r#type: "virtiofs".to_owned(),
                    source: file_mount.tag.as_str().to_owned(),
                    destination: file_mount.guest_holding_path,
                    options: Vec::new(),
                },
            ))
            .await
            .map_err(runtime_rpc_error("mount file mount holding directory"))?;
    }
    Ok(())
}
pub(crate) async fn mount_container_rootfs(
    client: &mut VminitdClient,
    bundle: &ContainerBundle,
    rootfs_source: &str,
    writable_layer: Option<&Mount>,
    writable_layer_source: Option<&str>,
) -> Result<()> {
    let Some(writable_layer) = writable_layer else {
        client
            .mount(tonic::Request::new(
                bundle.mount_rootfs_request(rootfs_source, ["rw"]),
            ))
            .await
            .map_err(runtime_rpc_error("mount rootfs"))?;
        return Ok(());
    };
    ensure_writable_layer_is_block(writable_layer)?;
    let writable_layer_source = writable_layer_source.ok_or_else(|| Error::RuntimeOperation {
        operation: "mount writable layer",
        reason: "writable layer source is unavailable in the guest".to_owned(),
    })?;
    let lower_path = format!("{}/lower", bundle.path());
    let upper_mount_path = format!("{}/upper", bundle.path());
    let upper_path = format!("{upper_mount_path}/diff");
    let work_path = format!("{upper_mount_path}/work");
    for path in [&lower_path, &upper_mount_path] {
        client
            .mkdir(tonic::Request::new(
                firkin_vminitd_client::pb::MkdirRequest {
                    path: path.clone(),
                    all: true,
                    perms: 0o755,
                },
            ))
            .await
            .map_err(runtime_rpc_error("mkdir writable-layer mount path"))?;
    }
    client
        .mount(tonic::Request::new(
            firkin_vminitd_client::pb::MountRequest {
                r#type: "ext4".to_owned(),
                source: rootfs_source.to_owned(),
                destination: lower_path.clone(),
                options: vec!["ro".to_owned()],
            },
        ))
        .await
        .map_err(runtime_rpc_error("mount rootfs lower layer"))?;
    client
        .mount(tonic::Request::new(
            firkin_vminitd_client::pb::MountRequest {
                r#type: writable_layer.kind.clone(),
                source: writable_layer_source.to_owned(),
                destination: upper_mount_path,
                options: writable_layer.options.clone(),
            },
        ))
        .await
        .map_err(runtime_rpc_error("mount writable layer"))?;
    for path in [&upper_path, &work_path] {
        client
            .mkdir(tonic::Request::new(
                firkin_vminitd_client::pb::MkdirRequest {
                    path: path.clone(),
                    all: true,
                    perms: 0o755,
                },
            ))
            .await
            .map_err(runtime_rpc_error("mkdir overlay work path"))?;
    }
    client
        .mount(tonic::Request::new(
            firkin_vminitd_client::pb::MountRequest {
                r#type: "overlay".to_owned(),
                source: "overlay".to_owned(),
                destination: bundle.rootfs_path().to_owned(),
                options: vec![
                    format!("lowerdir={lower_path}"),
                    format!("upperdir={upper_path}"),
                    format!("workdir={work_path}"),
                ],
            },
        ))
        .await
        .map_err(runtime_rpc_error("mount overlay rootfs"))?;
    Ok(())
}
pub(crate) async fn standard_guest_setup(client: &mut VminitdClient) -> Result<()> {
    for (path, perms) in [("/tmp", 0o1777), ("/dev/pts", 0o755)] {
        client
            .mkdir(tonic::Request::new(
                firkin_vminitd_client::pb::MkdirRequest {
                    path: path.to_owned(),
                    all: true,
                    perms,
                },
            ))
            .await
            .map_err(runtime_rpc_error("mkdir standard guest path"))?;
    }
    for request in [
        firkin_vminitd_client::pb::MountRequest {
            r#type: "tmpfs".to_owned(),
            source: "tmpfs".to_owned(),
            destination: "/tmp".to_owned(),
            options: Vec::new(),
        },
        firkin_vminitd_client::pb::MountRequest {
            r#type: "devpts".to_owned(),
            source: "devpts".to_owned(),
            destination: "/dev/pts".to_owned(),
            options: vec![
                "gid=5".to_owned(),
                "mode=620".to_owned(),
                "ptmxmode=666".to_owned(),
            ],
        },
    ] {
        client
            .mount(tonic::Request::new(request))
            .await
            .map_err(runtime_rpc_error("mount standard guest filesystem"))?;
    }
    ensure_loopback_up(client).await?;
    Ok(())
}
async fn ensure_loopback_up(client: &mut VminitdClient) -> Result<()> {
    client
        .ip_link_set(tonic::Request::new(
            firkin_vminitd_client::pb::IpLinkSetRequest {
                interface: "lo".to_owned(),
                up: true,
                mtu: None,
            },
        ))
        .await
        .map_err(runtime_rpc_error("configure loopback"))?;
    Ok(())
}
fn prepare_exec_stdio<E: ContainerStdio>(
    runtime: &mut ContainerRuntime,
    config: &ExecConfig<E>,
    container_id: &ContainerId,
    process_id: &ProcessId,
) -> Result<ProcessStdioPlan> {
    if E::TERMINAL {
        return prepare_pty_exec_stdio(runtime, config, container_id, process_id);
    }
    prepare_stream_exec_stdio(runtime, config)
}
fn prepare_pty_exec_stdio<E>(
    runtime: &mut ContainerRuntime,
    config: &ExecConfig<E>,
    container_id: &ContainerId,
    process_id: &ProcessId,
) -> Result<ProcessStdioPlan> {
    let input_port = runtime.allocate_host_port("allocate exec pty ports")?;
    let output_port = runtime.allocate_host_port("allocate exec pty ports")?;
    let input_listener = runtime
        .vm
        .listen_reserved_port(input_port)
        .map_err(runtime_vmm_error("listen for exec pty input"))?;
    let output_listener = runtime
        .vm
        .listen_reserved_port(output_port)
        .map_err(runtime_vmm_error("listen for exec pty output"))?;
    Ok(ProcessStdioPlan {
        stdio: ProcessStdio::new().stdin(input_port).stdout(output_port),
        stdin_task: None,
        pty_task: Some(tokio::spawn(accept_pty(
            input_listener,
            output_listener,
            config.pty.unwrap_or_default(),
            runtime.client.clone(),
            container_id.clone(),
            process_id.clone(),
        ))),
        stdout_task: None,
        stderr_task: None,
    })
}
fn prepare_stream_exec_stdio<E>(
    runtime: &mut ContainerRuntime,
    config: &ExecConfig<E>,
) -> Result<ProcessStdioPlan> {
    let stdin_plan = prepare_stream_stdin(runtime, config.stdin)?;
    let stdout_plan = prepare_stream_stdout(runtime, config.stdout)?;
    let stderr_plan = prepare_stream_stderr(runtime, config.stderr)?;
    let mut process_stdio = ProcessStdio::new();
    if let Some(port) = stdin_plan.0 {
        process_stdio = process_stdio.stdin(port);
    }
    if let Some(port) = stdout_plan.0 {
        process_stdio = process_stdio.stdout(port);
    }
    if let Some(port) = stderr_plan.0 {
        process_stdio = process_stdio.stderr(port);
    }
    Ok(ProcessStdioPlan {
        stdio: process_stdio,
        stdin_task: stdin_plan.1,
        pty_task: None,
        stdout_task: stdout_plan.1,
        stderr_task: stderr_plan.1,
    })
}
fn prepare_stream_stdin(
    runtime: &mut ContainerRuntime,
    config: Stdio,
) -> Result<OptionalStreamTask<ChildStdin>> {
    match config {
        Stdio::Null => Ok((None, None)),
        Stdio::Piped => {
            let port = runtime.allocate_host_port("allocate exec stdin port")?;
            let listener = runtime
                .vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error("listen for exec stdin"))?;
            Ok((Some(port), Some(tokio::spawn(accept_child_stdin(listener)))))
        }
        Stdio::Inherit => {
            let port = runtime.allocate_host_port("allocate exec stdin port")?;
            let listener = runtime
                .vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error("listen for exec stdin"))?;
            detach_stdio_relay(tokio::spawn(relay_inherited_stdin(listener)));
            Ok((Some(port), None))
        }
    }
}
fn prepare_stream_stdout(
    runtime: &mut ContainerRuntime,
    config: Stdio,
) -> Result<OptionalStreamTask<ChildStdout>> {
    match config {
        Stdio::Null => Ok((None, None)),
        Stdio::Piped => {
            let port = runtime.allocate_host_port("allocate exec stdout port")?;
            let listener = runtime
                .vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error("listen for exec stdout"))?;
            Ok((
                Some(port),
                Some(tokio::spawn(accept_child_stdout(listener))),
            ))
        }
        Stdio::Inherit => {
            let port = runtime.allocate_host_port("allocate exec stdout port")?;
            let listener = runtime
                .vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error("listen for exec stdout"))?;
            detach_stdio_relay(tokio::spawn(relay_inherited_stdout(listener)));
            Ok((Some(port), None))
        }
    }
}
fn prepare_stream_stderr(
    runtime: &mut ContainerRuntime,
    config: Stdio,
) -> Result<OptionalStreamTask<ChildStderr>> {
    match config {
        Stdio::Null => Ok((None, None)),
        Stdio::Piped => {
            let port = runtime.allocate_host_port("allocate exec stderr port")?;
            let listener = runtime
                .vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error("listen for exec stderr"))?;
            Ok((
                Some(port),
                Some(tokio::spawn(accept_child_stderr(listener))),
            ))
        }
        Stdio::Inherit => {
            let port = runtime.allocate_host_port("allocate exec stderr port")?;
            let listener = runtime
                .vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error("listen for exec stderr"))?;
            detach_stdio_relay(tokio::spawn(relay_inherited_stderr(listener)));
            Ok((Some(port), None))
        }
    }
}
pub(crate) async fn run_copy_control(
    mut client: VminitdClient,
    request: firkin_vminitd_client::pb::CopyRequest,
    mut metadata_tx: Option<oneshot::Sender<Result<CopyMetadata>>>,
) -> Result<Option<CopyMetadata>> {
    let mut stream = client
        .copy(tonic::Request::new(request))
        .await
        .map_err(runtime_rpc_error("start copy"))?
        .into_inner();
    let mut metadata = None;
    let mut complete = false;
    while let Some(response) = stream
        .message()
        .await
        .map_err(runtime_rpc_error("read copy response"))?
    {
        match CopyResponseEvent::try_from(response)
            .map_err(runtime_vminitd_error("parse copy response"))?
        {
            CopyResponseEvent::Metadata(next) => {
                metadata = Some(next);
                if let Some(tx) = metadata_tx.take() {
                    let _ = tx.send(Ok(next));
                }
            }
            CopyResponseEvent::Complete => {
                complete = true;
            }
        }
    }
    if let Some(tx) = metadata_tx.take() {
        let _ = tx.send(Err(Error::RuntimeOperation {
            operation: "copy metadata",
            reason: "copy response stream ended before metadata".to_owned(),
        }));
    }
    if !complete {
        return Err(Error::RuntimeOperation {
            operation: "copy control",
            reason: "copy response stream ended before completion".to_owned(),
        });
    }
    Ok(metadata)
}
async fn prepare_copy_in_payload(
    source: PathBuf,
    metadata: std::fs::Metadata,
) -> Result<CopyInPayload> {
    if metadata.is_file() {
        return Ok(CopyInPayload::File {
            path: source,
            mode: host_file_mode(&metadata),
        });
    }
    if metadata.is_dir() {
        let archive = create_directory_archive(source).await?;
        return Ok(CopyInPayload::DirectoryArchive { archive });
    }
    Err(Error::RuntimeOperation {
        operation: "copy in",
        reason: format!(
            "{} is neither a regular file nor a directory",
            source.display()
        ),
    })
}
async fn create_directory_archive(source: PathBuf) -> Result<tempfile::NamedTempFile> {
    tokio::task::spawn_blocking(move || {
        let archive = tempfile::NamedTempFile::new().map_err(|error| Error::RuntimeOperation {
            operation: "create copy-in archive",
            reason: error.to_string(),
        })?;
        let file =
            std::fs::File::create(archive.path()).map_err(|error| Error::RuntimeOperation {
                operation: "create copy-in archive",
                reason: error.to_string(),
            })?;
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        builder
            .append_dir_all(".", &source)
            .map_err(|error| Error::RuntimeOperation {
                operation: "archive copy-in directory",
                reason: error.to_string(),
            })?;
        let encoder = builder
            .into_inner()
            .map_err(|error| Error::RuntimeOperation {
                operation: "finish copy-in tar archive",
                reason: error.to_string(),
            })?;
        encoder.finish().map_err(|error| Error::RuntimeOperation {
            operation: "finish copy-in gzip archive",
            reason: error.to_string(),
        })?;
        Ok(archive)
    })
    .await
    .map_err(|error| Error::RuntimeOperation {
        operation: "create copy-in archive",
        reason: error.to_string(),
    })?
}
async fn extract_directory_archive(archive: PathBuf, destination: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&archive).map_err(|error| Error::RuntimeOperation {
            operation: "open copy-out archive",
            reason: error.to_string(),
        })?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(&destination)
            .map_err(|error| Error::RuntimeOperation {
                operation: "extract copy-out archive",
                reason: error.to_string(),
            })
    })
    .await
    .map_err(|error| Error::RuntimeOperation {
        operation: "extract copy-out archive",
        reason: error.to_string(),
    })?
}
pub(crate) async fn await_copy_control(
    task: JoinHandle<Result<Option<CopyMetadata>>>,
    operation: &'static str,
) -> Result<Option<CopyMetadata>> {
    task.await.map_err(|error| Error::RuntimeOperation {
        operation,
        reason: error.to_string(),
    })?
}
async fn relay_guest_vsock_to_host_unix(
    listener: firkin_vmm::VsockListener,
    host_path: PathBuf,
) -> Result<()> {
    loop {
        let (vsock, _peer) = listener
            .accept()
            .await
            .map_err(vsock_runtime_error("accept socket relay vsock"))?;
        let host_path = host_path.clone();
        drop(tokio::spawn(async move {
            let unix = UnixStream::connect(&host_path)
                .await
                .map_err(io_runtime_error("connect socket relay source"))?;
            relay_bidirectional(vsock, unix, "relay guest socket to host socket").await
        }));
    }
}
async fn relay_host_unix_to_guest_vsock(
    listener: UnixListener,
    vm: VirtualMachine<Running>,
    port: VsockPort,
) -> Result<()> {
    loop {
        let (unix, _addr) = listener
            .accept()
            .await
            .map_err(io_runtime_error("accept host socket relay"))?;
        let vm = vm.clone();
        drop(tokio::spawn(async move {
            let vsock = vm
                .dial_reserved_port(port)
                .await
                .map_err(runtime_vmm_error("dial socket relay vsock"))?;
            relay_bidirectional(vsock, unix, "relay host socket to guest socket").await
        }));
    }
}
async fn relay_bidirectional<A, B>(mut left: A, mut right: B, operation: &'static str) -> Result<()>
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    tokio::io::copy_bidirectional(&mut left, &mut right)
        .await
        .map_err(io_runtime_error(operation))?;
    Ok(())
}
pub(crate) async fn connect_vminitd(vm: &VirtualMachine<Running>) -> Result<VminitdClient> {
    let started = std::time::Instant::now();
    loop {
        let result = firkin_vminitd_client::connect_with_dialer({
            let vm = vm.clone();
            move |port| {
                let vm = vm.clone();
                async move {
                    vm.dial_reserved_port(port).await.map_err(|error| {
                        firkin_vsock::Error::Io(std::io::Error::other(error.to_string()))
                    })
                }
            }
        })
        .await;
        match result {
            Ok(client) => return Ok(client),
            Err(_) if started.elapsed() < VMINITD_CONNECT_TIMEOUT => {
                tokio::time::sleep(VMINITD_CONNECT_RETRY).await;
            }
            Err(error) => {
                return Err(Error::RuntimeOperation {
                    operation: "connect to vminitd",
                    reason: format!(
                        "timed out after {}s: {error}",
                        VMINITD_CONNECT_TIMEOUT.as_secs()
                    ),
                });
            }
        }
    }
}
fn container_guest_path(container_id: &ContainerId, path: &Path) -> Result<String> {
    let bundle = ContainerBundle::for_id(container_id);
    let relative = path.strip_prefix(Path::new("/")).unwrap_or(path);
    for component in relative.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::Prefix(_)
        ) {
            return Err(Error::RuntimeOperation {
                operation: "resolve container path",
                reason: format!("{} escapes the container rootfs", path.display()),
            });
        }
    }
    let relative = relative.to_str().ok_or(Error::RuntimeOperation {
        operation: "resolve container path",
        reason: "guest path is not valid UTF-8".to_owned(),
    })?;
    if relative.is_empty() {
        Ok(bundle.rootfs_path().to_owned())
    } else {
        Ok(format!("{}/{}", bundle.rootfs_path(), relative))
    }
}
fn host_file_mode(metadata: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0o644
    }
}
pub(crate) async fn accept_child_stdin(listener: firkin_vmm::VsockListener) -> Result<ChildStdin> {
    let (stream, _peer) = listener
        .accept()
        .await
        .map_err(|error| Error::RuntimeOperation {
            operation: "accept stdin vsock",
            reason: error.to_string(),
        })?;
    Ok(ChildStdin::new(stream))
}
pub(crate) async fn accept_child_stdout(
    listener: firkin_vmm::VsockListener,
) -> Result<ChildStdout> {
    let (stream, _peer) = listener
        .accept()
        .await
        .map_err(|error| Error::RuntimeOperation {
            operation: "accept stdout vsock",
            reason: error.to_string(),
        })?;
    Ok(ChildStdout::new(stream))
}
pub(crate) async fn accept_child_stderr(
    listener: firkin_vmm::VsockListener,
) -> Result<ChildStderr> {
    let (stream, _peer) = listener
        .accept()
        .await
        .map_err(|error| Error::RuntimeOperation {
            operation: "accept stderr vsock",
            reason: error.to_string(),
        })?;
    Ok(ChildStderr::new(stream))
}
pub(crate) async fn relay_stdio_stream<R, W>(
    mut reader: R,
    mut writer: W,
    operation: &'static str,
) -> Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let copied = tokio::io::copy(&mut reader, &mut writer)
        .await
        .map_err(io_runtime_error(operation))?;
    writer
        .shutdown()
        .await
        .map_err(io_runtime_error(operation))?;
    Ok(copied)
}
pub(crate) async fn relay_inherited_stdin(listener: firkin_vmm::VsockListener) -> Result<u64> {
    let stdin = accept_child_stdin(listener).await?;
    relay_stdio_stream(tokio::io::stdin(), stdin, "relay inherited stdin").await
}
pub(crate) async fn relay_inherited_stdout(listener: firkin_vmm::VsockListener) -> Result<u64> {
    let stdout = accept_child_stdout(listener).await?;
    relay_stdio_stream(stdout, tokio::io::stdout(), "relay inherited stdout").await
}
pub(crate) async fn relay_inherited_stderr(listener: firkin_vmm::VsockListener) -> Result<u64> {
    let stderr = accept_child_stderr(listener).await?;
    relay_stdio_stream(stderr, tokio::io::stderr(), "relay inherited stderr").await
}
pub(crate) fn detach_stdio_relay(task: JoinHandle<Result<u64>>) {
    drop(task);
}
pub(crate) async fn accept_pty(
    input_listener: firkin_vmm::VsockListener,
    output_listener: firkin_vmm::VsockListener,
    size: PtyConfig,
    client: VminitdClient,
    container_id: ContainerId,
    process_id: ProcessId,
) -> Result<Pty> {
    let input = async {
        input_listener
            .accept()
            .await
            .map(|(stream, _peer)| stream)
            .map_err(vsock_runtime_error("accept pty input vsock"))
    };
    let output = async {
        output_listener
            .accept()
            .await
            .map(|(stream, _peer)| stream)
            .map_err(vsock_runtime_error("accept pty output vsock"))
    };
    let (input, output) = tokio::try_join!(input, output)?;
    Ok(Pty::new(
        input,
        output,
        size,
        client,
        container_id,
        process_id,
    ))
}
pub(crate) fn block_device_guest_path(id: BlockDeviceId) -> Result<String> {
    let slot = id.slot().get();
    let index = u8::try_from(slot.saturating_sub(1)).unwrap_or(u8::MAX);
    let Some(letter) = b'b'.checked_add(index) else {
        return Err(Error::RuntimeOperation {
            operation: "map block device",
            reason: format!("block device slot {slot} is too large for /dev/vd* mapping"),
        });
    };
    if letter > b'z' {
        return Err(Error::RuntimeOperation {
            operation: "map block device",
            reason: format!("block device slot {slot} is too large for /dev/vd* mapping"),
        });
    }
    Ok(format!("/dev/vd{}", char::from(letter)))
}
pub(crate) async fn boot_vm_with_transient_network_retry(
    config: VmConfig,
) -> firkin_vmm::Result<VirtualMachine<Running>> {
    let mut attempt = 1;
    loop {
        match VirtualMachine::new(config.clone()).boot().await {
            Ok(vm) => return Ok(vm),
            Err(error)
                if attempt < TRANSIENT_VMNET_BOOT_ATTEMPTS
                    && is_transient_vmnet_boot_error(&error) =>
            {
                attempt += 1;
                tokio::time::sleep(TRANSIENT_VMNET_BOOT_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
}
pub(crate) fn is_transient_vmnet_boot_error(error: &firkin_vmm::Error) -> bool {
    let firkin_vmm::Error::UnclassifiedVz { reason } = error else {
        return false;
    };
    reason.contains("vmnet_network_create failed")
        && (reason.contains("VMNET_FAILURE")
            || reason.contains("VMNET_SHARING_SERVICE_BUSY")
            || reason.contains("status=1001")
            || reason.contains("status=1009"))
}
pub(crate) fn runtime_vmm_error(operation: &'static str) -> impl Fn(firkin_vmm::Error) -> Error {
    move |error| Error::RuntimeOperation {
        operation,
        reason: error.to_string(),
    }
}
pub(crate) fn runtime_vminitd_error(
    operation: &'static str,
) -> impl Fn(firkin_vminitd_client::VminitdError) -> Error {
    move |error| Error::RuntimeOperation {
        operation,
        reason: error.to_string(),
    }
}
pub(crate) fn io_runtime_error(operation: &'static str) -> impl Fn(std::io::Error) -> Error {
    move |error| Error::RuntimeOperation {
        operation,
        reason: error.to_string(),
    }
}
pub(crate) fn vsock_runtime_error(
    operation: &'static str,
) -> impl Fn(firkin_vsock::Error) -> Error {
    move |error| Error::RuntimeOperation {
        operation,
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_startup_timing_preserves_all_phase_durations() {
        let timing = ContainerStartupTiming::new(
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
            Duration::from_millis(4),
            Duration::from_millis(5),
            Duration::from_millis(6),
            Duration::from_millis(7),
            Duration::from_millis(8),
            Duration::from_millis(9),
            Duration::from_millis(10),
        );

        assert_eq!(timing.spec_build(), Duration::from_millis(1));
        assert_eq!(timing.vminitd_connect(), Duration::from_millis(2));
        assert_eq!(timing.socket_relays(), Duration::from_millis(3));
        assert_eq!(timing.stdio_prepare(), Duration::from_millis(4));
        assert_eq!(timing.config_write_rpc(), Duration::from_millis(5));
        assert_eq!(timing.request_encode(), Duration::from_millis(6));
        assert_eq!(timing.create_process_rpc(), Duration::from_millis(7));
        assert_eq!(timing.start_gate_wait(), Duration::from_millis(8));
        assert_eq!(timing.start_process_rpc(), Duration::from_millis(9));
        assert_eq!(timing.total(), Duration::from_millis(10));
    }
}
