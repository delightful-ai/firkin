//! builder — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::await_pty;
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use crate::io::{
    ChildStderr, ChildStdin, ChildStdout, DnsConfig, EXEC_STDIO_PORT_START, FileMount, HostsConfig,
    Pty, PtyConfig, SocketDirection, Stdio, Streams, UnixSocketConfig, prepare_file_mounts,
};
#[allow(unused_imports)]
use crate::os_strings_to_strings;
#[allow(unused_imports)]
use crate::prepare_fixed_stderr;
#[allow(unused_imports)]
use crate::prepare_fixed_stdin;
#[allow(unused_imports)]
use crate::prepare_fixed_stdout;
#[allow(unused_imports)]
use crate::process::{DEFAULT_PATH_ENV, ExitStatus, LinuxCapabilities, LinuxRlimit, Output, User};
#[allow(unused_imports)]
use crate::rootfs::{Rootfs, RootfsChoice, StagedRootfs, VmRootfs};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::runtime::is_transient_vmnet_boot_error;
#[allow(unused_imports)]
use crate::runtime::{
    Container, ContainerRuntime, ContainerStartupTiming, FixedStreamTask, RuntimeStaging,
    VminitdClient, accept_pty, allocate_vm_stdio_port, block_device_guest_path,
    configure_container_name_files, configure_guest_networks, connect_vminitd,
    mount_container_rootfs, mount_file_mount_holding_dirs, prepare_socket_relays,
    runtime_vminitd_error, runtime_vmm_error, standard_guest_setup, start_implicit_container,
    start_implicit_container_pty, start_implicit_container_with_staging,
};
#[allow(unused_imports)]
use crate::runtime_rpc_error;
#[allow(unused_imports)]
use crate::sealed;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::snapshot::{
    ContainerRestoreTimings, ContainerSnapshotState, RestoredRootfsStage,
    RestoredRootfsStageMethod, TimedContainerRestore,
};
#[allow(unused_imports)]
use firkin_ext4::Writer;
#[allow(unused_imports)]
use firkin_oci::ImageBundle;
#[allow(unused_imports)]
use firkin_oci::LinuxSeccomp;
#[allow(unused_imports)]
use firkin_oci::{ImageConfig, Spec};
#[allow(unused_imports)]
use firkin_oci::{
    Linux, LinuxNamespace, LinuxNamespaceType, LinuxResources, Process as OciProcess, Root,
};
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
use firkin_types::ProcessId;
#[allow(unused_imports)]
use firkin_types::{Arch, ContainerId, Hostname, Size, VirtiofsTag};
#[allow(unused_imports)]
use firkin_vminitd_bytes::{VMEXEC_AARCH64, VMINITD_AARCH64};
#[allow(unused_imports)]
use firkin_vminitd_client::ContainerBundle;
#[allow(unused_imports)]
use firkin_vminitd_client::RosettaSetup;
#[allow(unused_imports)]
use firkin_vminitd_client::{ProcessCreate, ProcessStdio};
#[allow(unused_imports)]
use firkin_vmm::{BootLog, KernelImage, Network, VmConfig};
#[allow(unused_imports)]
use firkin_vmm::{Running, VirtualMachine};
#[allow(unused_imports)]
use std::collections::{BTreeMap, HashSet};
#[allow(unused_imports)]
use std::ffi::OsString;
#[allow(unused_imports)]
use std::marker::PhantomData;
#[allow(unused_imports)]
use std::num::NonZeroU32;
#[cfg(all(feature = "snapshot", target_os = "macos"))]
#[allow(unused_imports)]
use std::os::unix::fs::MetadataExt;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[cfg(all(feature = "snapshot", target_os = "macos"))]
#[allow(unused_imports)]
use std::process::{Command, Stdio as StdProcessStdio};
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::LazyLock;
#[allow(unused_imports)]
use std::time::{Duration, Instant};

struct ExistingVmStdioPlan {
    stdio: ProcessStdio,
    stdin: FixedStreamTask<ChildStdin>,
    stdout: FixedStreamTask<ChildStdout>,
    stderr: FixedStreamTask<ChildStderr>,
}
#[allow(unused_imports)]
use tokio::sync::{Mutex, Semaphore};
#[allow(unused_imports)]
use tokio::task::JoinHandle;
pub(crate) const TRANSIENT_VMNET_BOOT_ATTEMPTS: usize = 3;
pub(crate) const TRANSIENT_VMNET_BOOT_RETRY_DELAY: Duration = Duration::from_millis(500);
/// VM context marker trait.
pub trait VmContext: sealed::Sealed {}
/// Builder-state marker trait.
pub trait BuilderState: sealed::Sealed {}
/// Process/container stdio-shape marker trait.
pub trait ContainerStdio: sealed::Sealed {
    /// Whether this shape uses OCI terminal mode.
    const TERMINAL: bool;
}
/// Builder owns an implicit single-use VM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImplicitVm;
impl sealed::Sealed for ImplicitVm {}
impl VmContext for ImplicitVm {}
/// Builder targets a borrowed running VM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnVm<'vm> {
    pub(crate) state: PhantomData<&'vm VirtualMachine<Running>>,
}
impl sealed::Sealed for OnVm<'_> {}
impl VmContext for OnVm<'_> {}
/// Builder targets an `Arc`-shared running VM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnVmArc;
impl sealed::Sealed for OnVmArc {}
impl VmContext for OnVmArc {}
/// Rootfs has not been set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Init;
impl sealed::Sealed for Init {}
impl BuilderState for Init {}
/// Rootfs is set and stream stdio is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ready;
impl sealed::Sealed for Ready {}
impl BuilderState for Ready {}
/// Rootfs is set and pty stdio is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadyPty;
impl sealed::Sealed for ReadyPty {}
impl BuilderState for ReadyPty {}
/// Exec command has not been set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissingCommand;
impl sealed::Sealed for MissingCommand {}
/// Exec command has been set and stream stdio is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandSet;
impl sealed::Sealed for CommandSet {}
/// Exec command has been set and pty stdio is selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandSetPty;
impl sealed::Sealed for CommandSetPty {}
/// Pre-spawn container configuration.
#[derive(Clone, Debug)]
pub struct ContainerBuilder<Vm: VmContext, S: BuilderState = Init> {
    pub(crate) id: ContainerId,
    pub(crate) vm: Option<Box<VirtualMachine<Running>>>,
    pub(crate) command: Vec<OsString>,
    command_explicit: bool,
    pub(crate) env: Vec<OsString>,
    pub(crate) working_dir: PathBuf,
    pub(crate) user: User,
    pub(crate) hostname: Option<Hostname>,
    sysctl: BTreeMap<String, String>,
    pub(crate) no_new_privileges: bool,
    pub(crate) capabilities: LinuxCapabilities,
    pub(crate) rlimits: Vec<LinuxRlimit>,
    pub(crate) selinux_label: String,
    pub(crate) apparmor_profile: String,
    pub(crate) mounts: Vec<Mount>,
    default_mounts: Option<Vec<Mount>>,
    pub(crate) writable_layer: Option<Mount>,
    pub(crate) file_mounts: Vec<FileMount>,
    seccomp: Option<SeccompConfig>,
    use_init: UseInit,
    pub(crate) dns: Option<DnsConfig>,
    pub(crate) hosts: Option<HostsConfig>,
    pub(crate) sockets: Vec<UnixSocketConfig>,
    pub(crate) stdin: Stdio,
    pub(crate) stdout: Stdio,
    pub(crate) stderr: Stdio,
    pub(crate) pty: Option<PtyConfig>,
    cpus: NonZeroU32,
    memory: Size,
    networks: Vec<Network>,
    virtiofs_shares: Vec<(VirtiofsTag, PathBuf)>,
    rosetta: bool,
    nested_virtualization: NestedVirtualization,
    boot_log: Option<BootLog>,
    kernel: Option<KernelImage>,
    cmdline_extra: Vec<String>,
    pub(crate) rootfs: Option<RootfsChoice>,
    pub(crate) state: PhantomData<(Vm, S)>,
}
impl<Vm: VmContext, S: BuilderState> ContainerBuilder<Vm, S> {
    pub(crate) fn new(id: ContainerId) -> Self {
        Self {
            id,
            vm: None,
            command: Vec::new(),
            command_explicit: false,
            env: vec![OsString::from(DEFAULT_PATH_ENV)],
            working_dir: PathBuf::from("/"),
            user: User::default(),
            hostname: None,
            sysctl: BTreeMap::new(),
            no_new_privileges: false,
            capabilities: LinuxCapabilities::default_oci(),
            rlimits: Vec::new(),
            selinux_label: String::new(),
            apparmor_profile: String::new(),
            mounts: Vec::new(),
            default_mounts: None,
            writable_layer: None,
            file_mounts: Vec::new(),
            seccomp: None,
            use_init: UseInit::Disabled,
            dns: None,
            hosts: None,
            sockets: Vec::new(),
            stdin: Stdio::Null,
            stdout: Stdio::Null,
            stderr: Stdio::Null,
            pty: None,
            cpus: NonZeroU32::new(4).expect("default CPU count is nonzero"),
            memory: Size::gib(1),
            networks: vec![Network::Nat],
            virtiofs_shares: Vec::new(),
            rosetta: false,
            nested_virtualization: NestedVirtualization::Disabled,
            boot_log: None,
            kernel: None,
            cmdline_extra: Vec::new(),
            rootfs: None,
            state: PhantomData,
        }
    }
    /// Return the container ID.
    #[must_use]
    pub const fn id(&self) -> &ContainerId {
        &self.id
    }
    /// Return configured command arguments.
    #[must_use]
    pub fn command_args(&self) -> &[OsString] {
        &self.command
    }
    /// Set command arguments.
    #[must_use]
    pub fn command<I, A>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        self.command = args.into_iter().map(Into::into).collect();
        self.command_explicit = true;
        self
    }
    /// Set one environment variable in `KEY=VALUE` form.
    #[must_use]
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        let mut entry = key.into();
        entry.push("=");
        entry.push(value.into());
        self.env.push(entry);
        self
    }
    /// Set environment variables in `KEY=VALUE` form.
    #[must_use]
    pub fn envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        for (key, value) in vars {
            let mut entry = key.into();
            entry.push("=");
            entry.push(value.into());
            self.env.push(entry);
        }
        self
    }
    /// Set the process working directory.
    #[must_use]
    pub fn working_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.working_dir = path.into();
        self
    }
    /// Set the process user.
    #[must_use]
    pub fn user(mut self, user: impl Into<User>) -> Self {
        self.user = user.into();
        self
    }
    /// Set the container hostname.
    #[must_use]
    pub fn hostname(mut self, hostname: Hostname) -> Self {
        self.hostname = Some(hostname);
        self
    }
    /// Set one Linux sysctl inside the container context.
    #[must_use]
    pub fn sysctl(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.sysctl.insert(key.into(), value.into());
        self
    }
    /// Set Linux sysctls inside the container context.
    #[must_use]
    pub fn sysctls<I, K, V>(mut self, settings: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in settings {
            self.sysctl.insert(key.into(), value.into());
        }
        self
    }
    /// Set the init process `no_new_privileges` bit.
    #[must_use]
    pub const fn no_new_privileges(mut self, on: bool) -> Self {
        self.no_new_privileges = on;
        self
    }
    /// Set the Linux capability sets for the init process.
    #[must_use]
    pub fn capabilities(mut self, capabilities: LinuxCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
    /// Append one POSIX resource limit for the init process.
    #[must_use]
    pub fn rlimit(mut self, rlimit: LinuxRlimit) -> Self {
        self.rlimits.push(rlimit);
        self
    }
    /// Append POSIX resource limits for the init process.
    #[must_use]
    pub fn rlimits<I>(mut self, rlimits: I) -> Self
    where
        I: IntoIterator<Item = LinuxRlimit>,
    {
        self.rlimits.extend(rlimits);
        self
    }
    /// Set the `SELinux` label for the init process.
    #[must_use]
    pub fn selinux_label(mut self, label: impl Into<String>) -> Self {
        self.selinux_label = label.into();
        self
    }
    /// Set the `AppArmor` profile for the init process.
    #[must_use]
    pub fn apparmor_profile(mut self, profile: impl Into<String>) -> Self {
        self.apparmor_profile = profile.into();
        self
    }
    /// Append one mount to the container runtime spec.
    #[must_use]
    pub fn mount(mut self, mount: Mount) -> Self {
        self.mounts.push(mount);
        self
    }
    /// Append mounts to the container runtime spec.
    #[must_use]
    pub fn mounts<I>(mut self, mounts: I) -> Self
    where
        I: IntoIterator<Item = Mount>,
    {
        self.mounts.extend(mounts);
        self
    }
    /// Replace the default Linux mount set used by this container.
    #[must_use]
    pub fn default_mounts(mut self, mounts: Vec<Mount>) -> Self {
        self.default_mounts = Some(mounts);
        self
    }
    /// Attach a writable block-backed upper layer over the container rootfs.
    #[must_use]
    pub fn writable_layer(mut self, mount: Mount) -> Self {
        self.writable_layer = Some(mount);
        self
    }
    /// Configure a structured seccomp policy for this container.
    #[must_use]
    pub fn seccomp(mut self, seccomp: Seccomp) -> Self {
        self.seccomp = Some(SeccompConfig::Structured(seccomp));
        self
    }
    /// Configure a raw runc-compatible seccomp policy JSON object.
    #[must_use]
    pub fn seccomp_profile_json(mut self, json: impl Into<String>) -> Self {
        self.seccomp = Some(SeccompConfig::RawJson(json.into()));
        self
    }
    /// Run the container through the minimal vminitd init wrapper.
    #[must_use]
    pub const fn use_init(mut self, on: bool) -> Self {
        self.use_init = UseInit::from_bool(on);
        self
    }
    /// Append one Unix socket relay.
    #[must_use]
    pub fn socket(mut self, socket: UnixSocketConfig) -> Self {
        self.sockets.push(socket);
        self
    }
    /// Append Unix socket relays.
    #[must_use]
    pub fn sockets<I>(mut self, sockets: I) -> Self
    where
        I: IntoIterator<Item = UnixSocketConfig>,
    {
        self.sockets.extend(sockets);
        self
    }
    /// Set stdin behavior for the init process.
    #[must_use]
    pub const fn stdin(mut self, stdin: Stdio) -> Self {
        self.stdin = stdin;
        self
    }
    /// Pull process defaults from an OCI image config.
    #[must_use]
    pub fn image_config(mut self, config: &ImageConfig) -> Self {
        if !self.command_explicit {
            self.command = config
                .command_args()
                .into_iter()
                .map(OsString::from)
                .collect();
        }
        if let Some(env) = &config.env {
            self.env = env.iter().cloned().map(OsString::from).collect();
        }
        if let Some(working_dir) = &config.working_dir {
            self.working_dir = PathBuf::from(working_dir);
        }
        if let Some(user) = &config.user {
            self.user = parse_image_user(user);
        }
        self
    }
    /// Set container memory.
    #[must_use]
    pub const fn memory(mut self, size: Size) -> Self {
        self.memory = size;
        self
    }
    /// Return configured CPU count.
    #[must_use]
    pub const fn cpu_count(&self) -> NonZeroU32 {
        self.cpus
    }
    /// Return configured memory.
    #[must_use]
    pub const fn memory_size(&self) -> Size {
        self.memory
    }
    /// Explicitly request Rosetta for amd64 Linux binaries in this VM.
    #[must_use]
    pub const fn rosetta(mut self, enabled: bool) -> Self {
        self.rosetta = enabled;
        self
    }
}
impl<Vm: VmContext> ContainerBuilder<Vm, Init> {
    /// Set stdout behavior for the init process.
    #[must_use]
    pub const fn stdout(mut self, stdout: Stdio) -> Self {
        self.stdout = stdout;
        self
    }
    /// Set stderr behavior for the init process.
    #[must_use]
    pub const fn stderr(mut self, stderr: Stdio) -> Self {
        self.stderr = stderr;
        self
    }
}
impl<Vm: VmContext> ContainerBuilder<Vm, Ready> {
    /// Set stdout behavior for the init process.
    #[must_use]
    pub const fn stdout(mut self, stdout: Stdio) -> Self {
        self.stdout = stdout;
        self
    }
    /// Set stderr behavior for the init process.
    #[must_use]
    pub const fn stderr(mut self, stderr: Stdio) -> Self {
        self.stderr = stderr;
        self
    }
}
impl<S: BuilderState> ContainerBuilder<ImplicitVm, S> {
    /// Set implicit VM CPU count.
    #[must_use]
    pub const fn cpus(mut self, cpus: NonZeroU32) -> Self {
        self.cpus = cpus;
        self
    }
    /// Append a VM network attachment.
    #[must_use]
    pub fn network(mut self, network: Network) -> Self {
        self.networks.push(network);
        self
    }
    /// Replace VM network attachments.
    #[must_use]
    pub fn networks(mut self, networks: impl IntoIterator<Item = Network>) -> Self {
        self.networks = networks.into_iter().collect();
        self
    }
    /// Configure `/etc/resolv.conf` for the container.
    #[must_use]
    pub fn dns(mut self, dns: DnsConfig) -> Self {
        self.dns = Some(dns);
        self
    }
    /// Configure `/etc/hosts` for the container.
    #[must_use]
    pub fn hosts(mut self, hosts: HostsConfig) -> Self {
        self.hosts = Some(hosts);
        self
    }
    /// Append one single-file mount for the implicit VM.
    #[must_use]
    pub fn file_mount(mut self, mount: FileMount) -> Self {
        self.file_mounts.push(mount);
        self
    }
    /// Append single-file mounts for the implicit VM.
    #[must_use]
    pub fn file_mounts<I>(mut self, mounts: I) -> Self
    where
        I: IntoIterator<Item = FileMount>,
    {
        self.file_mounts.extend(mounts);
        self
    }
    /// Append an implicit VM `virtiofs` share.
    #[must_use]
    pub fn virtiofs_share(mut self, tag: VirtiofsTag, host: impl Into<PathBuf>) -> Self {
        self.virtiofs_shares.push((tag, host.into()));
        self
    }
    /// Enable or disable nested virtualization for the implicit VM.
    #[must_use]
    pub const fn nested_virtualization(mut self, enabled: bool) -> Self {
        self.nested_virtualization = NestedVirtualization::from_bool(enabled);
        self
    }
    /// Set the implicit VM boot-log sink.
    #[must_use]
    pub fn boot_log(mut self, boot_log: BootLog) -> Self {
        self.boot_log = Some(boot_log);
        self
    }
    /// Set the implicit VM kernel image.
    #[must_use]
    pub fn kernel(mut self, kernel: KernelImage) -> Self {
        self.kernel = Some(kernel);
        self
    }
    /// Append an extra implicit VM kernel command-line fragment.
    #[must_use]
    pub fn cmdline_extra(mut self, value: impl Into<String>) -> Self {
        self.cmdline_extra.push(value.into());
        self
    }
}
impl ContainerBuilder<ImplicitVm, Ready> {
    /// Return the implicit rootfs.
    ///
    /// # Panics
    ///
    /// Panics only if the internal `Ready` typestate invariant is violated.
    #[must_use]
    pub fn rootfs(&self) -> &Rootfs {
        match self.rootfs.as_ref().expect("ready builder has rootfs") {
            RootfsChoice::Implicit(rootfs) => rootfs.as_ref(),
            RootfsChoice::OnVm(_) => unreachable!("implicit builder stores Rootfs"),
        }
    }
    #[allow(dead_code)]
    pub(crate) fn stage_rootfs_to(&self, dest: impl Into<PathBuf>) -> Result<StagedRootfs> {
        let dest = dest.into();
        match self.rootfs() {
            Rootfs::Ext4Image(path) | Rootfs::RawBlock(path) => Ok(StagedRootfs::new(path)),
            Rootfs::OciBundle(bundle) => {
                let mut writer = Writer::new(&dest, oci_rootfs_size(bundle))
                    .map_err(rootfs_error("create ext4 writer"))?
                    .write_oci_layers(bundle.as_ref())
                    .map_err(rootfs_error("write OCI layers"))?;
                if self.rosetta || bundle.platform().arch == Arch::Amd64 {
                    writer = writer
                        .write_dir("/run", 0o755)
                        .map_err(rootfs_error("create /run"))?
                        .write_dir("/run/rosetta", 0o755)
                        .map_err(rootfs_error("create /run/rosetta"))?;
                }
                writer
                    .finalize()
                    .map_err(rootfs_error("finalize ext4 rootfs"))?;
                Ok(StagedRootfs::new(dest))
            }
        }
    }
    #[cfg(feature = "snapshot")]
    fn stage_restored_rootfs_to(
        &self,
        dest: impl Into<PathBuf>,
    ) -> Result<(StagedRootfs, RestoredRootfsStage)> {
        let dest = dest.into();
        match self.rootfs() {
            Rootfs::Ext4Image(path) | Rootfs::RawBlock(path) => {
                let report = copy_restored_rootfs(path, &dest)?;
                Ok((StagedRootfs::new(dest), report))
            }
            Rootfs::OciBundle(_) => {
                let started = std::time::Instant::now();
                let rootfs = self.stage_rootfs_to(&dest)?;
                let elapsed = started.elapsed();
                let source_bytes = rootfs_source_bytes(&dest)?;
                Ok((
                    rootfs,
                    RestoredRootfsStage::new(
                        RestoredRootfsStageMethod::Rebuild,
                        source_bytes,
                        elapsed,
                    ),
                ))
            }
        }
    }
    #[cfg(feature = "snapshot")]
    pub(crate) fn stage_restored_rootfs_from_snapshot_state_to(
        &self,
        state: &ContainerSnapshotState,
        dest: impl Into<PathBuf>,
    ) -> Result<(StagedRootfs, RestoredRootfsStage)> {
        let source = state.staging_dir().join(format!("{}-rootfs.ext4", self.id));
        let dest = dest.into();
        let report = copy_restored_rootfs(&source, &dest)?;
        Ok((StagedRootfs::new(dest), report))
    }
    #[allow(dead_code)]
    pub(crate) fn prepare_implicit_vm_in(
        &self,
        staging_dir: impl AsRef<Path>,
    ) -> Result<PreparedImplicitVm> {
        let staging_dir = staging_dir.as_ref();
        std::fs::create_dir_all(staging_dir).map_err(|error| Error::RuntimeArtifact {
            operation: "create VM staging directory",
            reason: error.to_string(),
        })?;
        let rootfs_dest = staging_dir.join(format!("{}-rootfs.ext4", self.id));
        let rootfs = self.stage_rootfs_to(rootfs_dest)?;
        let init_block = resolve_init_block()?;
        let kernel = match &self.kernel {
            Some(kernel) => kernel.clone(),
            None => KernelImage::from_file(resolve_kernel()?),
        };
        let mut builder = VmConfig::builder()
            .cpus(self.cpus)
            .memory(self.memory)
            .networks(self.networks.clone())
            .rosetta(self.rosetta || self.rootfs_requires_rosetta())
            .nested_virtualization(self.nested_virtualization.enabled())
            .kernel(kernel)
            .boot_log(self.boot_log.clone().unwrap_or_else(resolve_boot_log))
            .init_block(init_block.path());
        for (tag, host_path) in &self.virtiofs_shares {
            builder = builder.virtiofs_share(tag.clone(), host_path);
        }
        let file_mounts = prepare_file_mounts(&self.file_mounts)?;
        let mut shared_file_mount_parents = HashSet::new();
        for file_mount in &file_mounts {
            if shared_file_mount_parents.insert(file_mount.tag.clone()) {
                builder = builder.virtiofs_share(file_mount.tag.clone(), &file_mount.parent);
            }
        }
        for value in &self.cmdline_extra {
            builder = builder.cmdline_extra(value.clone());
        }
        let (builder, rootfs_device) = builder.block_device(rootfs.path());
        let (builder, writable_layer_device) =
            if let Some(writable_layer) = self.writable_layer.as_ref() {
                ensure_writable_layer_is_block(writable_layer)?;
                let (builder, device) = builder.block_device(&writable_layer.source);
                (builder, Some(device))
            } else {
                (builder, None)
            };
        let config = builder
            .build()
            .map_err(vmm_config_error("build VM config"))?;
        Ok(PreparedImplicitVm {
            config,
            init_block,
            rootfs,
            #[cfg(feature = "snapshot")]
            rootfs_stage: None,
            rootfs_device,
            writable_layer_device,
        })
    }
    #[cfg(feature = "snapshot")]
    pub(crate) fn prepare_restored_implicit_vm_in(
        &self,
        staging_dir: impl AsRef<Path>,
        machine_identifier: Vec<u8>,
        network_macs: Vec<String>,
    ) -> Result<PreparedImplicitVm> {
        let staging_dir = staging_dir.as_ref();
        std::fs::create_dir_all(staging_dir).map_err(|error| Error::RuntimeArtifact {
            operation: "create restored VM staging directory",
            reason: error.to_string(),
        })?;
        let rootfs_dest = staging_dir.join(format!("{}-rootfs.ext4", self.id));
        let (rootfs, rootfs_stage) = self.stage_restored_rootfs_to(rootfs_dest)?;
        self.prepare_restored_implicit_vm_with_rootfs(
            rootfs,
            Some(rootfs_stage),
            machine_identifier,
            network_macs,
        )
    }
    #[cfg(feature = "snapshot")]
    fn prepare_restored_implicit_vm_from_snapshot_state_in(
        &self,
        staging_dir: impl AsRef<Path>,
        state: &ContainerSnapshotState,
    ) -> Result<PreparedImplicitVm> {
        let staging_dir = staging_dir.as_ref();
        std::fs::create_dir_all(staging_dir).map_err(|error| Error::RuntimeArtifact {
            operation: "create restored VM staging directory",
            reason: error.to_string(),
        })?;
        let rootfs_dest = staging_dir.join(format!("{}-rootfs.ext4", self.id));
        let (rootfs, rootfs_stage) =
            self.stage_restored_rootfs_from_snapshot_state_to(state, rootfs_dest)?;
        self.prepare_restored_implicit_vm_with_rootfs(
            rootfs,
            Some(rootfs_stage),
            state.machine_identifier().to_vec(),
            state.network_macs().to_vec(),
        )
    }
    #[cfg(feature = "snapshot")]
    fn prepare_restored_implicit_vm_with_rootfs(
        &self,
        rootfs: StagedRootfs,
        rootfs_stage: Option<RestoredRootfsStage>,
        machine_identifier: Vec<u8>,
        network_macs: Vec<String>,
    ) -> Result<PreparedImplicitVm> {
        let init_block = resolve_init_block()?;
        let kernel = match &self.kernel {
            Some(kernel) => kernel.clone(),
            None => KernelImage::from_file(resolve_kernel()?),
        };
        let mut builder = VmConfig::builder()
            .cpus(self.cpus)
            .memory(self.memory)
            .networks(self.networks.clone())
            .rosetta(self.rosetta || self.rootfs_requires_rosetta())
            .nested_virtualization(self.nested_virtualization.enabled())
            .kernel(kernel)
            .boot_log(self.boot_log.clone().unwrap_or_else(resolve_boot_log))
            .init_block(init_block.path())
            .machine_identifier(machine_identifier)
            .network_macs(network_macs);
        for (tag, host_path) in &self.virtiofs_shares {
            builder = builder.virtiofs_share(tag.clone(), host_path);
        }
        let file_mounts = prepare_file_mounts(&self.file_mounts)?;
        let mut shared_file_mount_parents = HashSet::new();
        for file_mount in &file_mounts {
            if shared_file_mount_parents.insert(file_mount.tag.clone()) {
                builder = builder.virtiofs_share(file_mount.tag.clone(), &file_mount.parent);
            }
        }
        for value in &self.cmdline_extra {
            builder = builder.cmdline_extra(value.clone());
        }
        let (builder, rootfs_device) = builder.block_device(rootfs.path());
        let (builder, writable_layer_device) =
            if let Some(writable_layer) = self.writable_layer.as_ref() {
                ensure_writable_layer_is_block(writable_layer)?;
                let (builder, device) = builder.block_device(&writable_layer.source);
                (builder, Some(device))
            } else {
                (builder, None)
            };
        let config = builder
            .build()
            .map_err(vmm_config_error("build restored VM config"))?;
        Ok(PreparedImplicitVm {
            config,
            init_block,
            rootfs,
            rootfs_stage,
            rootfs_device,
            writable_layer_device,
        })
    }
    fn rootfs_requires_rosetta(&self) -> bool {
        matches!(
            self.rootfs(), Rootfs::OciBundle(bundle) if bundle.platform().arch ==
            Arch::Amd64
        )
    }
    /// Generate the OCI runtime spec vminitd will receive for this container.
    ///
    /// # Errors
    ///
    /// Returns an error if process fields contain non-UTF-8 data that cannot
    /// be represented in the OCI JSON spec.
    pub fn runtime_spec(&self) -> Result<Spec> {
        self.runtime_spec_with_terminal(false)
    }
    pub(crate) fn runtime_spec_with_terminal(&self, terminal: bool) -> Result<Spec> {
        runtime_spec_for_builder(
            self,
            terminal,
            self.rosetta || self.rootfs_requires_rosetta(),
        )
    }
    /// Spawn the container.
    ///
    /// # Errors
    ///
    /// Returns an error if VM preparation, VM boot, or vminitd process startup
    /// fails.
    pub async fn spawn(self) -> Result<Container<Streams>> {
        start_implicit_container(self).await
    }
    /// Spawn the container using a caller-owned persistent staging directory.
    ///
    /// The staged rootfs remains on disk after the container is stopped, which
    /// allows a saved VM snapshot to be restored later with the same block-device
    /// paths.
    ///
    /// # Errors
    ///
    /// Returns an error if VM preparation, VM boot, or vminitd process startup
    /// fails.
    pub async fn spawn_with_staging_dir(
        self,
        staging_dir: impl Into<PathBuf>,
    ) -> Result<Container<Streams>> {
        start_implicit_container_with_staging(self, RuntimeStaging::persistent(staging_dir)?).await
    }
    /// Restore a previously snapshotted implicit-VM container.
    ///
    /// `staging_dir` must be the persistent staging directory used by
    /// [`spawn_with_staging_dir`](Self::spawn_with_staging_dir), and the machine
    /// identifier/MACs must be the values returned by
    /// [`Container::snapshot_state`].
    ///
    /// # Errors
    ///
    /// Returns an error if the staged rootfs is missing, VM restore fails, or
    /// vminitd cannot be reached after restore.
    #[cfg(feature = "snapshot")]
    pub async fn restore_from_snapshot(
        self,
        snapshot_path: impl AsRef<Path>,
        staging_dir: impl Into<PathBuf>,
        machine_identifier: Vec<u8>,
        network_macs: Vec<String>,
    ) -> Result<Container<Streams>> {
        Ok(restore_implicit_container_from_snapshot(
            self,
            snapshot_path,
            RuntimeStaging::persistent(staging_dir)?,
            machine_identifier,
            network_macs,
        )
        .await?
        .into_parts()
        .0)
    }
    /// Restore a previously snapshotted implicit-VM container with phase timings.
    ///
    /// # Errors
    ///
    /// Returns an error if the staged rootfs is missing, VM restore fails, or
    /// vminitd cannot be reached after restore.
    #[cfg(feature = "snapshot")]
    pub async fn restore_from_snapshot_with_timings(
        self,
        snapshot_path: impl AsRef<Path>,
        staging_dir: impl Into<PathBuf>,
        machine_identifier: Vec<u8>,
        network_macs: Vec<String>,
    ) -> Result<TimedContainerRestore> {
        restore_implicit_container_from_snapshot(
            self,
            snapshot_path,
            RuntimeStaging::persistent(staging_dir)?,
            machine_identifier,
            network_macs,
        )
        .await
    }
    /// Restore a previously snapshotted implicit-VM container from persisted
    /// snapshot state.
    ///
    /// # Errors
    ///
    /// Returns an error if the staged rootfs is missing, VM restore fails, or
    /// vminitd cannot be reached after restore.
    #[cfg(feature = "snapshot")]
    pub async fn restore_from_snapshot_state(
        self,
        snapshot_path: impl AsRef<Path>,
        state: &ContainerSnapshotState,
    ) -> Result<Container<Streams>> {
        Ok(restore_implicit_container_from_snapshot_state(
            self,
            snapshot_path,
            RuntimeStaging::temp()?,
            state,
        )
        .await?
        .into_parts()
        .0)
    }
    /// Restore a previously snapshotted implicit-VM container from persisted
    /// snapshot state into a caller-owned persistent staging directory.
    ///
    /// The restored container can itself be snapshotted later because the
    /// staging directory remains stable after restore.
    ///
    /// # Errors
    ///
    /// Returns an error if the staged rootfs is missing, VM restore fails, or
    /// vminitd cannot be reached after restore.
    #[cfg(feature = "snapshot")]
    pub async fn restore_from_snapshot_state_with_staging_dir(
        self,
        snapshot_path: impl AsRef<Path>,
        staging_dir: impl Into<PathBuf>,
        state: &ContainerSnapshotState,
    ) -> Result<Container<Streams>> {
        Ok(restore_implicit_container_from_snapshot_state(
            self,
            snapshot_path,
            RuntimeStaging::persistent(staging_dir)?,
            state,
        )
        .await?
        .into_parts()
        .0)
    }
    /// Restore a previously snapshotted implicit-VM container from persisted
    /// snapshot state with phase timings.
    ///
    /// # Errors
    ///
    /// Returns an error if the staged rootfs is missing, VM restore fails, or
    /// vminitd cannot be reached after restore.
    #[cfg(feature = "snapshot")]
    pub async fn restore_from_snapshot_state_with_timings(
        self,
        snapshot_path: impl AsRef<Path>,
        state: &ContainerSnapshotState,
    ) -> Result<TimedContainerRestore> {
        restore_implicit_container_from_snapshot_state(
            self,
            snapshot_path,
            RuntimeStaging::temp()?,
            state,
        )
        .await
    }
    /// Restore a previously snapshotted implicit-VM container from persisted
    /// snapshot state into a persistent staging directory with phase timings.
    ///
    /// # Errors
    ///
    /// Returns an error if the staged rootfs is missing, VM restore fails, or
    /// vminitd cannot be reached after restore.
    #[cfg(feature = "snapshot")]
    pub async fn restore_from_snapshot_state_with_staging_dir_and_timings(
        self,
        snapshot_path: impl AsRef<Path>,
        staging_dir: impl Into<PathBuf>,
        state: &ContainerSnapshotState,
    ) -> Result<TimedContainerRestore> {
        restore_implicit_container_from_snapshot_state(
            self,
            snapshot_path,
            RuntimeStaging::persistent(staging_dir)?,
            state,
        )
        .await
    }
    /// One-shot terminal that waits for exit status.
    ///
    /// # Errors
    ///
    /// Returns an error if VM preparation, VM boot, vminitd process startup,
    /// process wait, or VM cleanup fails.
    pub async fn status(self) -> Result<ExitStatus> {
        self.spawn().await?.wait().await
    }
    /// One-shot terminal that captures stdout and stderr.
    ///
    /// # Errors
    ///
    /// Returns an error if VM preparation, VM boot, vminitd process startup,
    /// process wait, stdio capture, or VM cleanup fails.
    pub async fn output(mut self) -> Result<Output> {
        self.stdout = Stdio::Piped;
        self.stderr = Stdio::Piped;
        run_implicit_output(self).await
    }
}
impl<Vm: VmContext> ContainerBuilder<Vm, Ready> {
    /// Switch the init process to pseudo-terminal mode.
    #[must_use]
    pub fn pty(self, config: impl Into<PtyConfig>) -> ContainerBuilder<Vm, ReadyPty> {
        ContainerBuilder {
            id: self.id,
            vm: self.vm,
            command: self.command,
            command_explicit: self.command_explicit,
            env: self.env,
            working_dir: self.working_dir,
            user: self.user,
            hostname: self.hostname,
            sysctl: self.sysctl,
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities,
            rlimits: self.rlimits,
            selinux_label: self.selinux_label,
            apparmor_profile: self.apparmor_profile,
            mounts: self.mounts,
            default_mounts: self.default_mounts,
            writable_layer: self.writable_layer,
            file_mounts: self.file_mounts,
            seccomp: self.seccomp,
            use_init: self.use_init,
            dns: self.dns,
            hosts: self.hosts,
            sockets: self.sockets,
            stdin: Stdio::Null,
            stdout: Stdio::Null,
            stderr: Stdio::Null,
            pty: Some(config.into()),
            cpus: self.cpus,
            memory: self.memory,
            networks: self.networks,
            virtiofs_shares: self.virtiofs_shares,
            rosetta: self.rosetta,
            nested_virtualization: self.nested_virtualization,
            boot_log: self.boot_log,
            kernel: self.kernel,
            cmdline_extra: self.cmdline_extra,
            rootfs: self.rootfs,
            state: PhantomData,
        }
    }
}
impl ContainerBuilder<ImplicitVm, ReadyPty> {
    pub(crate) fn as_stream_ready(&self) -> ContainerBuilder<ImplicitVm, Ready> {
        ContainerBuilder {
            id: self.id.clone(),
            vm: self.vm.clone(),
            command: self.command.clone(),
            command_explicit: self.command_explicit,
            env: self.env.clone(),
            working_dir: self.working_dir.clone(),
            user: self.user.clone(),
            hostname: self.hostname.clone(),
            sysctl: self.sysctl.clone(),
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities.clone(),
            rlimits: self.rlimits.clone(),
            selinux_label: self.selinux_label.clone(),
            apparmor_profile: self.apparmor_profile.clone(),
            mounts: self.mounts.clone(),
            default_mounts: self.default_mounts.clone(),
            writable_layer: self.writable_layer.clone(),
            file_mounts: self.file_mounts.clone(),
            seccomp: self.seccomp.clone(),
            use_init: self.use_init,
            dns: self.dns.clone(),
            hosts: self.hosts.clone(),
            sockets: self.sockets.clone(),
            stdin: self.stdin,
            stdout: self.stdout,
            stderr: self.stderr,
            pty: None,
            cpus: self.cpus,
            memory: self.memory,
            networks: self.networks.clone(),
            virtiofs_shares: self.virtiofs_shares.clone(),
            rosetta: self.rosetta,
            nested_virtualization: self.nested_virtualization,
            boot_log: self.boot_log.clone(),
            kernel: self.kernel.clone(),
            cmdline_extra: self.cmdline_extra.clone(),
            rootfs: self.rootfs.clone(),
            state: PhantomData,
        }
    }
    /// Spawn the pty-backed container.
    ///
    /// # Errors
    ///
    /// Returns an error if VM preparation, VM boot, or vminitd process startup
    /// fails.
    pub async fn spawn(self) -> Result<Container<Pty>> {
        start_implicit_container_pty(self).await
    }
    /// One-shot terminal that waits for exit status.
    ///
    /// # Errors
    ///
    /// Returns an error if VM preparation, VM boot, vminitd process startup,
    /// process wait, or VM cleanup fails.
    pub async fn status(self) -> Result<ExitStatus> {
        self.spawn().await?.wait().await
    }
    /// One-shot terminal that captures combined pty output.
    ///
    /// # Errors
    ///
    /// Returns an error if VM preparation, VM boot, vminitd process startup,
    /// process wait, stdio capture, or VM cleanup fails.
    pub async fn output(self) -> Result<Output> {
        self.spawn().await?.wait_with_output().await
    }
}
impl ContainerBuilder<ImplicitVm, Init> {
    /// Set rootfs and advance the builder to `Ready`.
    #[must_use]
    pub fn rootfs(mut self, rootfs: impl Into<Rootfs>) -> ContainerBuilder<ImplicitVm, Ready> {
        self.rootfs = Some(RootfsChoice::Implicit(Box::new(rootfs.into())));
        ContainerBuilder {
            id: self.id,
            vm: self.vm,
            command: self.command,
            command_explicit: self.command_explicit,
            env: self.env,
            working_dir: self.working_dir,
            user: self.user,
            hostname: self.hostname,
            sysctl: self.sysctl,
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities,
            rlimits: self.rlimits,
            selinux_label: self.selinux_label,
            apparmor_profile: self.apparmor_profile,
            mounts: self.mounts,
            default_mounts: self.default_mounts,
            writable_layer: self.writable_layer,
            file_mounts: self.file_mounts,
            seccomp: self.seccomp,
            use_init: self.use_init,
            dns: self.dns,
            hosts: self.hosts,
            sockets: self.sockets,
            stdin: self.stdin,
            stdout: self.stdout,
            stderr: self.stderr,
            pty: self.pty,
            cpus: self.cpus,
            memory: self.memory,
            networks: self.networks,
            virtiofs_shares: self.virtiofs_shares,
            rosetta: self.rosetta,
            nested_virtualization: self.nested_virtualization,
            boot_log: self.boot_log,
            kernel: self.kernel,
            cmdline_extra: self.cmdline_extra,
            rootfs: self.rootfs,
            state: PhantomData,
        }
    }
}
impl<'vm> ContainerBuilder<OnVm<'vm>, Init> {
    /// Set a predeclared VM rootfs and advance the builder to `Ready`.
    #[must_use]
    pub fn rootfs(mut self, rootfs: impl Into<VmRootfs>) -> ContainerBuilder<OnVm<'vm>, Ready> {
        self.rootfs = Some(RootfsChoice::OnVm(rootfs.into()));
        ContainerBuilder {
            id: self.id,
            vm: self.vm,
            command: self.command,
            command_explicit: self.command_explicit,
            env: self.env,
            working_dir: self.working_dir,
            user: self.user,
            hostname: self.hostname,
            sysctl: self.sysctl,
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities,
            rlimits: self.rlimits,
            selinux_label: self.selinux_label,
            apparmor_profile: self.apparmor_profile,
            mounts: self.mounts,
            default_mounts: self.default_mounts,
            writable_layer: self.writable_layer,
            file_mounts: self.file_mounts,
            seccomp: self.seccomp,
            use_init: self.use_init,
            dns: self.dns,
            hosts: self.hosts,
            sockets: self.sockets,
            stdin: self.stdin,
            stdout: self.stdout,
            stderr: self.stderr,
            pty: self.pty,
            cpus: self.cpus,
            memory: self.memory,
            networks: self.networks,
            virtiofs_shares: self.virtiofs_shares,
            rosetta: self.rosetta,
            nested_virtualization: self.nested_virtualization,
            boot_log: self.boot_log,
            kernel: self.kernel,
            cmdline_extra: self.cmdline_extra,
            rootfs: self.rootfs,
            state: PhantomData,
        }
    }
}
impl ContainerBuilder<OnVmArc, Init> {
    /// Set a predeclared VM rootfs and advance the builder to `Ready`.
    #[must_use]
    pub fn rootfs(mut self, rootfs: impl Into<VmRootfs>) -> ContainerBuilder<OnVmArc, Ready> {
        self.rootfs = Some(RootfsChoice::OnVm(rootfs.into()));
        ContainerBuilder {
            id: self.id,
            vm: self.vm,
            command: self.command,
            command_explicit: self.command_explicit,
            env: self.env,
            working_dir: self.working_dir,
            user: self.user,
            hostname: self.hostname,
            sysctl: self.sysctl,
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities,
            rlimits: self.rlimits,
            selinux_label: self.selinux_label,
            apparmor_profile: self.apparmor_profile,
            mounts: self.mounts,
            default_mounts: self.default_mounts,
            writable_layer: self.writable_layer,
            file_mounts: self.file_mounts,
            seccomp: self.seccomp,
            use_init: self.use_init,
            dns: self.dns,
            hosts: self.hosts,
            sockets: self.sockets,
            stdin: self.stdin,
            stdout: self.stdout,
            stderr: self.stderr,
            pty: self.pty,
            cpus: self.cpus,
            memory: self.memory,
            networks: self.networks,
            virtiofs_shares: self.virtiofs_shares,
            rosetta: self.rosetta,
            nested_virtualization: self.nested_virtualization,
            boot_log: self.boot_log,
            kernel: self.kernel,
            cmdline_extra: self.cmdline_extra,
            rootfs: self.rootfs,
            state: PhantomData,
        }
    }
}
impl ContainerBuilder<OnVm<'_>, Ready> {
    /// Generate the OCI runtime spec vminitd will receive for this container.
    ///
    /// # Errors
    ///
    /// Returns an error if process fields contain non-UTF-8 data that cannot
    /// be represented in the OCI JSON spec.
    pub fn runtime_spec(&self) -> Result<Spec> {
        runtime_spec_for_on_vm_builder(self, false)
    }
    /// Spawn the container on the existing VM.
    ///
    /// # Errors
    ///
    /// Returns an error if vminitd process startup fails.
    pub async fn spawn(self) -> Result<Container<Streams>> {
        start_on_vm_container(self).await
    }
    /// One-shot terminal that waits for exit status.
    ///
    /// # Errors
    ///
    /// Returns an error if process startup or wait fails.
    pub async fn status(self) -> Result<ExitStatus> {
        self.spawn().await?.wait().await
    }
    /// One-shot terminal that captures stdout and stderr.
    ///
    /// # Errors
    ///
    /// Returns an error if process startup, wait, or stdio capture fails.
    pub async fn output(mut self) -> Result<Output> {
        self.stdout = Stdio::Piped;
        self.stderr = Stdio::Piped;
        self.spawn().await?.wait_with_output().await
    }
}
impl ContainerBuilder<OnVmArc, Ready> {
    /// Generate the OCI runtime spec vminitd will receive for this container.
    ///
    /// # Errors
    ///
    /// Returns an error if process fields contain non-UTF-8 data that cannot
    /// be represented in the OCI JSON spec.
    pub fn runtime_spec(&self) -> Result<Spec> {
        runtime_spec_for_on_vm_builder(self, false)
    }
    /// Spawn the container on the existing VM.
    ///
    /// # Errors
    ///
    /// Returns an error if vminitd process startup fails.
    pub async fn spawn(self) -> Result<Container<Streams>> {
        start_on_vm_container(self).await
    }
    pub(crate) async fn spawn_prepared_guest_path_with_start_gate(
        self,
        start_gate: Arc<Semaphore>,
    ) -> Result<Container<Streams>> {
        start_prepared_guest_path_container(self, Some(start_gate)).await
    }
    /// One-shot terminal that waits for exit status.
    ///
    /// # Errors
    ///
    /// Returns an error if process startup or wait fails.
    pub async fn status(self) -> Result<ExitStatus> {
        self.spawn().await?.wait().await
    }
    /// One-shot terminal that captures stdout and stderr.
    ///
    /// # Errors
    ///
    /// Returns an error if process startup, wait, or stdio capture fails.
    pub async fn output(mut self) -> Result<Output> {
        self.stdout = Stdio::Piped;
        self.stderr = Stdio::Piped;
        self.spawn().await?.wait_with_output().await
    }
}
impl ContainerBuilder<OnVm<'_>, ReadyPty> {
    /// Spawn the pty-backed container on the existing VM.
    ///
    /// # Errors
    ///
    /// Returns an error if vminitd process startup fails.
    pub async fn spawn(self) -> Result<Container<Pty>> {
        start_on_vm_container_pty(self).await
    }
    /// One-shot terminal that waits for exit status.
    ///
    /// # Errors
    ///
    /// Returns an error if process startup or wait fails.
    pub async fn status(self) -> Result<ExitStatus> {
        self.spawn().await?.wait().await
    }
    /// One-shot terminal that captures combined pty output.
    ///
    /// # Errors
    ///
    /// Returns an error if process startup, wait, or pty capture fails.
    pub async fn output(self) -> Result<Output> {
        self.spawn().await?.wait_with_output().await
    }
}
impl ContainerBuilder<OnVmArc, ReadyPty> {
    /// Spawn the pty-backed container on the existing VM.
    ///
    /// # Errors
    ///
    /// Returns an error if vminitd process startup fails.
    pub async fn spawn(self) -> Result<Container<Pty>> {
        start_on_vm_container_pty(self).await
    }
    /// One-shot terminal that waits for exit status.
    ///
    /// # Errors
    ///
    /// Returns an error if process startup or wait fails.
    pub async fn status(self) -> Result<ExitStatus> {
        self.spawn().await?.wait().await
    }
    /// One-shot terminal that captures combined pty output.
    ///
    /// # Errors
    ///
    /// Returns an error if process startup, wait, or pty capture fails.
    pub async fn output(self) -> Result<Output> {
        self.spawn().await?.wait_with_output().await
    }
}
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StagedInitBlock {
    pub(crate) path: PathBuf,
}
#[allow(dead_code)]
impl StagedInitBlock {
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedImplicitVm {
    pub(crate) config: VmConfig,
    init_block: StagedInitBlock,
    pub(crate) rootfs: StagedRootfs,
    #[cfg(feature = "snapshot")]
    pub(crate) rootfs_stage: Option<RestoredRootfsStage>,
    pub(crate) rootfs_device: BlockDeviceId,
    pub(crate) writable_layer_device: Option<BlockDeviceId>,
}
#[allow(dead_code)]
impl PreparedImplicitVm {
    pub(crate) fn config(&self) -> &VmConfig {
        &self.config
    }
    pub(crate) fn init_block(&self) -> &StagedInitBlock {
        &self.init_block
    }
    pub(crate) fn rootfs(&self) -> &StagedRootfs {
        &self.rootfs
    }
    #[cfg(feature = "snapshot")]
    const fn rootfs_stage(&self) -> Option<RestoredRootfsStage> {
        self.rootfs_stage
    }
    pub(crate) const fn rootfs_device(&self) -> BlockDeviceId {
        self.rootfs_device
    }
    pub(crate) const fn writable_layer_device(&self) -> Option<BlockDeviceId> {
        self.writable_layer_device
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum SeccompConfig {
    Structured(Seccomp),
    RawJson(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NestedVirtualization {
    Disabled,
    Enabled,
}
impl NestedVirtualization {
    const fn from_bool(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
    const fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UseInit {
    Disabled,
    Enabled,
}
impl UseInit {
    const fn from_bool(enabled: bool) -> Self {
        if enabled {
            Self::Enabled
        } else {
            Self::Disabled
        }
    }
    const fn enabled(self) -> bool {
        matches!(self, Self::Enabled)
    }
}
fn runtime_spec_for_builder<Vm: VmContext, S: BuilderState>(
    builder: &ContainerBuilder<Vm, S>,
    terminal: bool,
    rosetta_mount: bool,
) -> Result<Spec> {
    if let Some(writable_layer) = builder.writable_layer.as_ref() {
        ensure_writable_layer_is_block(writable_layer)?;
    }
    let bundle = ContainerBundle::for_id(&builder.id);
    let mounts = runtime_spec_mounts(builder, rosetta_mount)?;
    let seccomp = runtime_spec_seccomp(builder)?;
    let root_path = runtime_spec_root_path(builder, &bundle);
    let mut process_args = os_strings_to_strings(&builder.command, |index| {
        Error::InvalidCommandArgument { index }
    })?;
    if builder.use_init.enabled() {
        let mut init_args = vec!["/.cz-init".to_owned(), "--".to_owned()];
        init_args.extend(process_args);
        process_args = init_args;
    }
    Ok(Spec {
        version: "1.1.0".to_owned(),
        process: Some(OciProcess {
            args: process_args,
            env: os_strings_to_strings(&builder.env, |index| Error::InvalidEnvironmentVariable {
                index,
            })?,
            cwd: builder
                .working_dir
                .to_str()
                .ok_or(Error::InvalidWorkingDirectory)?
                .to_owned(),
            user: builder.user.clone().into_oci(),
            no_new_privileges: builder.no_new_privileges,
            capabilities: Some(builder.capabilities.clone().into_oci()),
            rlimits: builder
                .rlimits
                .iter()
                .copied()
                .map(LinuxRlimit::into_oci)
                .collect(),
            selinux_label: builder.selinux_label.clone(),
            apparmor_profile: builder.apparmor_profile.clone(),
            terminal,
            ..OciProcess::default()
        }),
        hostname: builder
            .hostname
            .as_ref()
            .map_or_else(|| builder.id.to_string(), ToString::to_string),
        mounts,
        root: Some(Root {
            path: root_path,
            readonly: false,
        }),
        linux: Some(Linux {
            resources: Some(LinuxResources::default()),
            cgroups_path: format!("/container/{}", builder.id),
            namespaces: vec![
                LinuxNamespace::unshare(LinuxNamespaceType::Pid),
                LinuxNamespace::unshare(LinuxNamespaceType::Mount),
                LinuxNamespace::unshare(LinuxNamespaceType::Ipc),
                LinuxNamespace::unshare(LinuxNamespaceType::Uts),
            ],
            sysctl: (!builder.sysctl.is_empty()).then(|| builder.sysctl.clone()),
            seccomp,
            ..Linux::default()
        }),
        ..Spec::default()
    })
}
fn runtime_spec_root_path<Vm: VmContext, S: BuilderState>(
    builder: &ContainerBuilder<Vm, S>,
    bundle: &ContainerBundle,
) -> String {
    match builder.rootfs.as_ref() {
        Some(RootfsChoice::OnVm(VmRootfs::GuestPath(path))) => path.as_str().to_owned(),
        _ => bundle.rootfs_path().to_owned(),
    }
}
fn runtime_spec_for_on_vm_builder<Vm: VmContext>(
    builder: &ContainerBuilder<Vm, Ready>,
    terminal: bool,
) -> Result<Spec> {
    runtime_spec_for_builder(
        builder,
        terminal,
        builder
            .vm
            .as_deref()
            .is_some_and(|vm| vm.config().rosetta_enabled()),
    )
}
fn runtime_spec_seccomp<Vm: VmContext, S: BuilderState>(
    builder: &ContainerBuilder<Vm, S>,
) -> Result<Option<LinuxSeccomp>> {
    match builder.seccomp.as_ref() {
        None => Ok(None),
        Some(SeccompConfig::Structured(seccomp)) => Ok(Some(LinuxSeccomp::from(seccomp.clone()))),
        Some(SeccompConfig::RawJson(json)) => {
            let value = serde_json::from_str(json).map_err(|error| Error::InvalidSeccomp {
                reason: error.to_string(),
            })?;
            Ok(Some(LinuxSeccomp::Raw(value)))
        }
    }
}
pub(crate) fn ensure_writable_layer_is_block(mount: &Mount) -> Result<()> {
    if matches!(mount.kind.as_str(), "ext4" | "xfs" | "btrfs") {
        Ok(())
    } else {
        Err(Error::InvalidWritableLayer {
            kind: mount.kind.clone(),
        })
    }
}
fn runtime_spec_mounts<Vm: VmContext, S: BuilderState>(
    builder: &ContainerBuilder<Vm, S>,
    rosetta_mount: bool,
) -> Result<Vec<Mount>> {
    let mut mounts = builder
        .default_mounts
        .clone()
        .unwrap_or_else(Mount::defaults);
    mounts.extend(builder.mounts.iter().cloned());
    if rosetta_mount {
        mounts.push(Mount::virtiofs("rosetta", "/run/rosetta"));
    }
    for socket in builder
        .sockets
        .iter()
        .filter(|socket| socket.direction == SocketDirection::Into)
    {
        let destination = socket
            .destination
            .to_str()
            .ok_or_else(|| Error::RuntimeOperation {
                operation: "build socket bind mount",
                reason: format!(
                    "socket destination {} is not valid UTF-8",
                    socket.destination.display()
                ),
            })?;
        mounts.push(
            Mount::custom(
                "bind",
                guest_socket_staging_path(&builder.id, &socket.id),
                destination,
            )
            .extra_option("bind"),
        );
    }
    for file_mount in prepare_file_mounts(&builder.file_mounts)? {
        mounts.push(file_mount.oci_bind_mount()?);
    }
    if builder.use_init.enabled() {
        mounts.push(
            Mount::custom("bind", "/sbin/vminitd", "/.cz-init")
                .extra_option("bind")
                .read_only(),
        );
    }
    Ok(clean_and_sort_mounts(mounts))
}
fn clean_and_sort_mounts(mounts: Vec<Mount>) -> Vec<Mount> {
    let mut indexed_mounts = mounts
        .into_iter()
        .enumerate()
        .map(|(index, mut mount)| {
            mount.destination = normalize_mount_destination(&mount.destination);
            (index, mount)
        })
        .collect::<Vec<_>>();
    indexed_mounts.sort_by_key(|(index, mount)| (mount_depth(&mount.destination), *index));
    indexed_mounts.into_iter().map(|(_, mount)| mount).collect()
}
fn normalize_mount_destination(destination: &str) -> String {
    let path = Path::new(destination);
    let mut absolute = false;
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => {
                parts.push(prefix.as_os_str().to_string_lossy().into_owned());
            }
            std::path::Component::RootDir => {
                absolute = true;
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if parts.last().is_some_and(|part| part != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..".to_owned());
                }
            }
            std::path::Component::Normal(part) => {
                parts.push(part.to_string_lossy().into_owned());
            }
        }
    }
    if absolute {
        if parts.is_empty() {
            "/".to_owned()
        } else {
            format!("/{}", parts.join("/"))
        }
    } else if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}
fn mount_depth(destination: &str) -> usize {
    destination
        .split('/')
        .filter(|component| !component.is_empty())
        .count()
}
fn parse_image_user(raw: &str) -> User {
    if let Some((uid, gid)) = raw.split_once(':')
        && let (Ok(uid), Ok(gid)) = (uid.parse::<u32>(), gid.parse::<u32>())
    {
        return User::numeric(uid, gid);
    }
    if let Ok(uid) = raw.parse::<u32>() {
        return User::from(uid);
    }
    User::named(raw)
}
#[allow(dead_code)]
fn oci_rootfs_size(bundle: &ImageBundle) -> Size {
    let compressed = bundle.total_size().as_bytes();
    Size::bytes(
        compressed
            .saturating_mul(4)
            .saturating_add(Size::mib(128).as_bytes())
            .max(Size::mib(128).as_bytes()),
    )
}
#[allow(dead_code)]
fn rootfs_error(operation: &'static str) -> impl Fn(firkin_ext4::Error) -> Error {
    move |error| Error::RootfsAssembly {
        operation,
        reason: error.to_string(),
    }
}
#[allow(dead_code)]
pub(crate) fn resolve_init_block() -> Result<StagedInitBlock> {
    static INIT_BLOCK: LazyLock<std::result::Result<PathBuf, String>> =
        LazyLock::new(|| resolve_init_block_path().map_err(|error| error.to_string()));
    INIT_BLOCK
        .as_ref()
        .map(|path| StagedInitBlock::new(path.clone()))
        .map_err(|reason| Error::RuntimeArtifact {
            operation: "resolve init.block",
            reason: reason.clone(),
        })
}
#[allow(dead_code)]
fn resolve_init_block_path() -> Result<PathBuf> {
    if VMINITD_AARCH64.is_empty() || VMEXEC_AARCH64.is_empty() {
        return Err(Error::RuntimeArtifact {
            operation: "load vminitd runtime ELFs",
            reason: "runtime-download builds do not embed vminitd/vmexec bytes yet".to_owned(),
        });
    }
    firkin_ext4::init_block::synthesize(VMINITD_AARCH64, VMEXEC_AARCH64)
        .map_err(runtime_artifact_error("synthesize init.block"))
}
#[allow(dead_code)]
fn runtime_artifact_error(operation: &'static str) -> impl Fn(firkin_ext4::Error) -> Error {
    move |error| Error::RuntimeArtifact {
        operation,
        reason: error.to_string(),
    }
}
#[allow(dead_code)]
pub(crate) fn resolve_kernel() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("FIRKIN_KERNEL_PATH") {
        return canonical_artifact_path(&PathBuf::from(path), "resolve kernel");
    }
    canonical_artifact_path(&repo_root().join("bin/vmlinux"), "resolve bundled kernel")
}
fn resolve_boot_log() -> BootLog {
    std::env::var_os("FIRKIN_BOOT_LOG_PATH")
        .map(PathBuf::from)
        .map_or(BootLog::None, BootLog::File)
}
#[allow(dead_code)]
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
#[allow(dead_code)]
fn canonical_artifact_path(path: &Path, operation: &'static str) -> Result<PathBuf> {
    path.canonicalize().map_err(|error| Error::RuntimeArtifact {
        operation,
        reason: format!("{}: {error}", path.display()),
    })
}
#[allow(dead_code)]
fn vmm_config_error(operation: &'static str) -> impl Fn(firkin_vmm::Error) -> Error {
    move |error| Error::RuntimeArtifact {
        operation,
        reason: error.to_string(),
    }
}
pub(crate) struct ImplicitStartContext {
    pub(crate) staging: RuntimeStaging,
    pub(crate) vm: VirtualMachine<Running>,
    pub(crate) client: VminitdClient,
    pub(crate) spec: Spec,
    pub(crate) process_id: ProcessId,
    pub(crate) socket_tasks: Vec<JoinHandle<Result<()>>>,
    pub(crate) socket_proxy_ids: Vec<String>,
    pub(crate) next_stdio_port: u32,
}
#[cfg(feature = "snapshot")]
async fn restore_implicit_container_from_snapshot(
    builder: ContainerBuilder<ImplicitVm, Ready>,
    snapshot_path: impl AsRef<Path>,
    staging: RuntimeStaging,
    machine_identifier: Vec<u8>,
    network_macs: Vec<String>,
) -> Result<TimedContainerRestore> {
    let staging_started = std::time::Instant::now();
    let prepared = builder.prepare_restored_implicit_vm_in(
        staging.path(),
        machine_identifier,
        network_macs,
    )?;
    let staging_elapsed = staging_started.elapsed();
    let process_id =
        ProcessId::new(builder.id.to_string()).map_err(|error| Error::RuntimeOperation {
            operation: "build restored process id",
            reason: error.to_string(),
        })?;
    let vm_restore_started = std::time::Instant::now();
    let rootfs_stage = prepared.rootfs_stage();
    let vm =
        boot_or_restore_vm_with_transient_network_retry(prepared.config, snapshot_path.as_ref())
            .await
            .map_err(runtime_vmm_error("restore VM snapshot"))?;
    let vm_restore = vm_restore_started.elapsed();
    let vminitd_started = std::time::Instant::now();
    let mut client = connect_vminitd(&vm).await?;
    let bundle = ContainerBundle::for_id(&builder.id);
    configure_guest_networks(&vm, &mut client, bundle.rootfs_path()).await?;
    let vminitd_connect = vminitd_started.elapsed();
    let rootfs_path = bundle.rootfs_path().to_owned();
    let container = Container {
        id: builder.id.clone(),
        pid: None,
        pty: None,
        runtime: Arc::new(Mutex::new(ContainerRuntime {
            vm,
            client,
            container_id: builder.id,
            process_id,
            rootfs_path,
            staging,
            next_stdio_port: EXEC_STDIO_PORT_START,
            stop_vm_on_wait: true,
            shared_vm_ports: false,
            stdin_task: None,
            pty_task: None,
            stdout_task: None,
            stderr_task: None,
            socket_tasks: Vec::new(),
            socket_proxy_ids: Vec::new(),
            exit_status: None,
        })),
        startup_timing: ContainerStartupTiming::default(),
        state: PhantomData,
    };
    Ok(TimedContainerRestore::new(
        container,
        ContainerRestoreTimings::new(staging_elapsed, vm_restore, vminitd_connect, rootfs_stage),
    ))
}
#[cfg(feature = "snapshot")]
async fn restore_implicit_container_from_snapshot_state(
    builder: ContainerBuilder<ImplicitVm, Ready>,
    snapshot_path: impl AsRef<Path>,
    staging: RuntimeStaging,
    state: &ContainerSnapshotState,
) -> Result<TimedContainerRestore> {
    let staging_started = std::time::Instant::now();
    let prepared =
        builder.prepare_restored_implicit_vm_from_snapshot_state_in(staging.path(), state)?;
    let staging_elapsed = staging_started.elapsed();
    let process_id =
        ProcessId::new(builder.id.to_string()).map_err(|error| Error::RuntimeOperation {
            operation: "build restored process id",
            reason: error.to_string(),
        })?;
    let vm_restore_started = std::time::Instant::now();
    let rootfs_stage = prepared.rootfs_stage();
    let vm =
        boot_or_restore_vm_with_transient_network_retry(prepared.config, snapshot_path.as_ref())
            .await
            .map_err(runtime_vmm_error("restore VM snapshot"))?;
    let vm_restore = vm_restore_started.elapsed();
    let vminitd_started = std::time::Instant::now();
    let mut client = connect_vminitd(&vm).await?;
    let bundle = ContainerBundle::for_id(&builder.id);
    configure_guest_networks(&vm, &mut client, bundle.rootfs_path()).await?;
    let vminitd_connect = vminitd_started.elapsed();
    let rootfs_path = bundle.rootfs_path().to_owned();
    let container = Container {
        id: builder.id.clone(),
        pid: None,
        pty: None,
        runtime: Arc::new(Mutex::new(ContainerRuntime {
            vm,
            client,
            container_id: builder.id,
            process_id,
            rootfs_path,
            staging,
            next_stdio_port: EXEC_STDIO_PORT_START,
            stop_vm_on_wait: true,
            shared_vm_ports: false,
            stdin_task: None,
            pty_task: None,
            stdout_task: None,
            stderr_task: None,
            socket_tasks: Vec::new(),
            socket_proxy_ids: Vec::new(),
            exit_status: None,
        })),
        startup_timing: ContainerStartupTiming::default(),
        state: PhantomData,
    };
    Ok(TimedContainerRestore::new(
        container,
        ContainerRestoreTimings::new(staging_elapsed, vm_restore, vminitd_connect, rootfs_stage),
    ))
}
#[cfg(feature = "snapshot")]
pub(crate) fn copy_restored_rootfs(source: &Path, dest: &Path) -> Result<RestoredRootfsStage> {
    let source_bytes = rootfs_source_bytes(source)?;
    let started = std::time::Instant::now();
    if restored_rootfs_clone_candidate(source, dest) && clone_restored_rootfs(source, dest).is_ok()
    {
        return Ok(RestoredRootfsStage::new(
            RestoredRootfsStageMethod::Clone,
            source_bytes,
            started.elapsed(),
        ));
    }
    let _ = std::fs::remove_file(dest);
    std::fs::copy(source, dest).map_err(|error| Error::RuntimeArtifact {
        operation: "copy restored rootfs",
        reason: format!("{} -> {}: {error}", source.display(), dest.display()),
    })?;
    Ok(RestoredRootfsStage::new(
        RestoredRootfsStageMethod::Copy,
        source_bytes,
        started.elapsed(),
    ))
}
#[cfg(feature = "snapshot")]
fn rootfs_source_bytes(source: &Path) -> Result<u64> {
    std::fs::metadata(source)
        .map(|metadata| metadata.len())
        .map_err(|error| Error::RuntimeArtifact {
            operation: "stat restored rootfs",
            reason: format!("{}: {error}", source.display()),
        })
}
#[cfg(all(feature = "snapshot", target_os = "macos"))]
fn restored_rootfs_clone_candidate(source: &Path, dest: &Path) -> bool {
    let Some(dest_parent) = dest.parent() else {
        return false;
    };
    let Ok(source_metadata) = std::fs::metadata(source) else {
        return false;
    };
    let Ok(dest_parent_metadata) = std::fs::metadata(dest_parent) else {
        return false;
    };
    source_metadata.dev() == dest_parent_metadata.dev()
}
#[cfg(all(feature = "snapshot", not(target_os = "macos")))]
fn restored_rootfs_clone_candidate(_source: &Path, _dest: &Path) -> bool {
    false
}
#[cfg(all(feature = "snapshot", target_os = "macos"))]
fn clone_restored_rootfs(source: &Path, dest: &Path) -> Result<()> {
    let status = Command::new("/bin/cp")
        .arg("-c")
        .arg(source)
        .arg(dest)
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .status()
        .map_err(|error| Error::RuntimeArtifact {
            operation: "clone restored rootfs",
            reason: format!("{} -> {}: {error}", source.display(), dest.display()),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::RuntimeArtifact {
            operation: "clone restored rootfs",
            reason: format!(
                "{} -> {} exited with {status}",
                source.display(),
                dest.display()
            ),
        })
    }
}
#[cfg(all(feature = "snapshot", not(target_os = "macos")))]
fn clone_restored_rootfs(_source: &Path, _dest: &Path) -> Result<()> {
    Err(Error::RuntimeArtifact {
        operation: "clone restored rootfs",
        reason: "copy-on-write clone is not available on this platform".to_owned(),
    })
}
async fn start_on_vm_container<Vm: VmContext>(
    builder: ContainerBuilder<Vm, Ready>,
) -> Result<Container<Streams>> {
    let vm = builder
        .vm
        .as_deref()
        .cloned()
        .ok_or_else(|| Error::RuntimeOperation {
            operation: "spawn on existing VM",
            reason: "builder does not carry a running VM handle".to_owned(),
        })?;
    let RootfsChoice::OnVm(rootfs) = builder.rootfs.as_ref().expect("ready builder has rootfs")
    else {
        return Err(Error::RuntimeOperation {
            operation: "spawn on existing VM",
            reason: "builder rootfs is not a predeclared VM block device".to_owned(),
        });
    };
    let staging = tempfile::tempdir().map_err(|error| Error::RuntimeArtifact {
        operation: "create existing-VM runtime staging directory",
        reason: error.to_string(),
    })?;
    let spec = runtime_spec_for_builder(&builder, false, vm.config().rosetta_enabled())?;
    let process_id =
        ProcessId::new(builder.id.to_string()).map_err(|error| Error::RuntimeOperation {
            operation: "build process id",
            reason: error.to_string(),
        })?;
    let mut client = prepare_on_vm_bundle(&vm, &builder, rootfs, &spec).await?;
    let bundle = ContainerBundle::for_id(&builder.id);
    let rootfs_path = runtime_spec_root_path(&builder, &bundle);
    let mut next_socket_port = EXEC_STDIO_PORT_START;
    let socket_relays = prepare_socket_relays(
        &vm,
        &mut client,
        &builder,
        &bundle,
        true,
        &mut next_socket_port,
    )
    .await?;
    let stdin_port = allocate_vm_stdio_port(vm.id(), "allocate init stdin port")?;
    let stdout_port = allocate_vm_stdio_port(vm.id(), "allocate init stdout port")?;
    let stderr_port = allocate_vm_stdio_port(vm.id(), "allocate init stderr port")?;
    let stdin_plan = prepare_fixed_stdin(&vm, builder.stdin, stdin_port, "listen for stdin")?;
    let stdout_plan = prepare_fixed_stdout(&vm, builder.stdout, stdout_port, "listen for stdout")?;
    let stderr_plan = prepare_fixed_stderr(&vm, builder.stderr, stderr_port, "listen for stderr")?;
    let mut stdio = ProcessStdio::new();
    if stdin_plan.0 {
        stdio = stdio.stdin(stdin_port);
    }
    if stdout_plan.0 {
        stdio = stdio.stdout(stdout_port);
    }
    if stderr_plan.0 {
        stdio = stdio.stderr(stderr_port);
    }
    let create = ProcessCreate::new(process_id.clone(), builder.id.clone(), spec)
        .stdio(stdio)
        .into_request()
        .map_err(runtime_vminitd_error("encode create process request"))?;
    client
        .create_process(tonic::Request::new(create))
        .await
        .map_err(runtime_rpc_error("create process"))?;
    let pid =
        start_process_with_transient_vmexec_retry(&mut client, &process_id, &builder.id, None)
            .await?;
    Ok(Container {
        id: builder.id.clone(),
        pid: Some(pid),
        pty: None,
        runtime: Arc::new(Mutex::new(ContainerRuntime {
            vm,
            client,
            container_id: builder.id,
            process_id,
            rootfs_path,
            staging: RuntimeStaging::Temp(staging),
            next_stdio_port: EXEC_STDIO_PORT_START,
            stop_vm_on_wait: false,
            shared_vm_ports: true,
            stdin_task: stdin_plan.1,
            pty_task: None,
            stdout_task: stdout_plan.1,
            stderr_task: stderr_plan.1,
            socket_tasks: socket_relays.tasks,
            socket_proxy_ids: socket_relays.proxy_ids,
            exit_status: None,
        })),
        startup_timing: ContainerStartupTiming::default(),
        state: PhantomData,
    })
}

const TRANSIENT_VMEXEC_START_ATTEMPTS: usize = 3;
const TRANSIENT_VMEXEC_START_RETRY_DELAY: Duration = Duration::from_millis(10);

struct ProcessStartAttemptTiming {
    pid: i32,
    start_gate_wait: Duration,
    start_process_rpc: Duration,
}

async fn start_process_with_transient_vmexec_retry(
    client: &mut VminitdClient,
    process_id: &ProcessId,
    container_id: &ContainerId,
    start_gate: Option<Arc<Semaphore>>,
) -> Result<i32> {
    start_process_with_transient_vmexec_retry_timed(client, process_id, container_id, start_gate)
        .await
        .map(|timing| timing.pid)
}

async fn start_process_with_transient_vmexec_retry_timed(
    client: &mut VminitdClient,
    process_id: &ProcessId,
    container_id: &ContainerId,
    start_gate: Option<Arc<Semaphore>>,
) -> Result<ProcessStartAttemptTiming> {
    let mut attempt = 1;
    let mut start_gate_wait = Duration::ZERO;
    let mut start_process_rpc = Duration::ZERO;
    loop {
        let gate_started = Instant::now();
        let start_permit = match start_gate.clone() {
            Some(gate) => {
                Some(
                    gate.acquire_owned()
                        .await
                        .map_err(|error| Error::RuntimeOperation {
                            operation: "acquire process start gate",
                            reason: error.to_string(),
                        })?,
                )
            }
            None => None,
        };
        start_gate_wait += gate_started.elapsed();
        let start_started = Instant::now();
        let result = client
            .start_process(tonic::Request::new(ProcessCreate::start_request(
                process_id,
                Some(container_id),
            )))
            .await
            .map_err(runtime_rpc_error("start process"));
        start_process_rpc += start_started.elapsed();
        match result {
            Ok(response) => {
                return Ok(ProcessStartAttemptTiming {
                    pid: response.into_inner().pid,
                    start_gate_wait,
                    start_process_rpc,
                });
            }
            Err(error)
                if attempt < TRANSIENT_VMEXEC_START_ATTEMPTS
                    && is_transient_vmexec_start_error(&error) =>
            {
                drop(start_permit);
                attempt += 1;
                tokio::time::sleep(TRANSIENT_VMEXEC_START_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_transient_vmexec_start_error(error: &Error) -> bool {
    let Error::RuntimeOperation {
        operation: "start process",
        reason,
    } = error
    else {
        return false;
    };
    reason.contains("vmexec error")
        && (reason.contains("No such file or directory") || reason.contains("ENOENT"))
}
fn prepare_existing_vm_stdio(
    vm: &VirtualMachine<Running>,
    stdin_config: Stdio,
    stdout_config: Stdio,
    stderr_config: Stdio,
) -> Result<ExistingVmStdioPlan> {
    let stdin_port = allocate_vm_stdio_port(vm.id(), "allocate init stdin port")?;
    let stdout_port = allocate_vm_stdio_port(vm.id(), "allocate init stdout port")?;
    let stderr_port = allocate_vm_stdio_port(vm.id(), "allocate init stderr port")?;
    let stdin_plan = prepare_fixed_stdin(vm, stdin_config, stdin_port, "listen for stdin")?;
    let stdout_plan = prepare_fixed_stdout(vm, stdout_config, stdout_port, "listen for stdout")?;
    let stderr_plan = prepare_fixed_stderr(vm, stderr_config, stderr_port, "listen for stderr")?;
    let mut stdio = ProcessStdio::new();
    if stdin_plan.0 {
        stdio = stdio.stdin(stdin_port);
    }
    if stdout_plan.0 {
        stdio = stdio.stdout(stdout_port);
    }
    if stderr_plan.0 {
        stdio = stdio.stderr(stderr_port);
    }
    Ok(ExistingVmStdioPlan {
        stdio,
        stdin: stdin_plan,
        stdout: stdout_plan,
        stderr: stderr_plan,
    })
}

fn prepared_guest_path_vm<Vm: VmContext>(
    builder: &ContainerBuilder<Vm, Ready>,
) -> Result<VirtualMachine<Running>> {
    builder
        .vm
        .as_deref()
        .cloned()
        .ok_or_else(|| Error::RuntimeOperation {
            operation: "spawn prepared guest-path container",
            reason: "builder does not carry a running VM handle".to_owned(),
        })
}

fn ensure_prepared_guest_path_rootfs<Vm: VmContext>(
    builder: &ContainerBuilder<Vm, Ready>,
) -> Result<()> {
    let RootfsChoice::OnVm(VmRootfs::GuestPath(_)) =
        builder.rootfs.as_ref().expect("ready builder has rootfs")
    else {
        return Err(Error::RuntimeOperation {
            operation: "spawn prepared guest-path container",
            reason: "builder rootfs is not a prepared guest path".to_owned(),
        });
    };
    Ok(())
}

fn prepared_guest_path_runtime_staging() -> Result<tempfile::TempDir> {
    tempfile::tempdir().map_err(|error| Error::RuntimeArtifact {
        operation: "create prepared guest-path runtime staging directory",
        reason: error.to_string(),
    })
}

async fn start_prepared_guest_path_container<Vm: VmContext>(
    builder: ContainerBuilder<Vm, Ready>,
    start_gate: Option<Arc<Semaphore>>,
) -> Result<Container<Streams>> {
    let total_started = Instant::now();
    let vm = prepared_guest_path_vm(&builder)?;
    ensure_prepared_guest_path_rootfs(&builder)?;
    let staging = prepared_guest_path_runtime_staging()?;
    let spec_started = Instant::now();
    let spec = runtime_spec_for_builder(&builder, false, vm.config().rosetta_enabled())?;
    let spec_build = spec_started.elapsed();
    let process_id =
        ProcessId::new(builder.id.to_string()).map_err(|error| Error::RuntimeOperation {
            operation: "build process id",
            reason: error.to_string(),
        })?;
    let vminitd_started = Instant::now();
    let mut client = connect_vminitd(&vm).await?;
    let vminitd_connect = vminitd_started.elapsed();
    let bundle = ContainerBundle::for_id(&builder.id);
    let rootfs_path = runtime_spec_root_path(&builder, &bundle);
    let mut next_socket_port = EXEC_STDIO_PORT_START;
    let socket_relays_started = Instant::now();
    let socket_relays = prepare_socket_relays(
        &vm,
        &mut client,
        &builder,
        &bundle,
        true,
        &mut next_socket_port,
    )
    .await?;
    let socket_relays_prepare = socket_relays_started.elapsed();
    let stdio_started = Instant::now();
    let stdio_plan = prepare_existing_vm_stdio(&vm, builder.stdin, builder.stdout, builder.stderr)?;
    let stdio_prepare = stdio_started.elapsed();
    let config_write_started = Instant::now();
    client
        .write_file(tonic::Request::new(
            bundle
                .write_config_request(&spec)
                .map_err(runtime_vminitd_error("encode runtime spec"))?,
        ))
        .await
        .map_err(runtime_rpc_error("write config.json"))?;
    let config_write_rpc = config_write_started.elapsed();
    let request_encode_started = Instant::now();
    let create = ProcessCreate::new(process_id.clone(), builder.id.clone(), spec)
        .stdio(stdio_plan.stdio)
        .into_request()
        .map_err(runtime_vminitd_error("encode create process request"))?;
    let request_encode = request_encode_started.elapsed();
    let create_started = Instant::now();
    client
        .create_process(tonic::Request::new(create))
        .await
        .map_err(runtime_rpc_error("create process"))?;
    let create_process_rpc = create_started.elapsed();
    let start_timing = start_process_with_transient_vmexec_retry_timed(
        &mut client,
        &process_id,
        &builder.id,
        start_gate,
    )
    .await?;
    let startup_timing = ContainerStartupTiming::new(
        spec_build,
        vminitd_connect,
        socket_relays_prepare,
        stdio_prepare,
        config_write_rpc,
        request_encode,
        create_process_rpc,
        start_timing.start_gate_wait,
        start_timing.start_process_rpc,
        total_started.elapsed(),
    );
    Ok(Container {
        id: builder.id.clone(),
        pid: Some(start_timing.pid),
        pty: None,
        runtime: Arc::new(Mutex::new(ContainerRuntime {
            vm,
            client,
            container_id: builder.id,
            process_id,
            rootfs_path,
            staging: RuntimeStaging::Temp(staging),
            next_stdio_port: EXEC_STDIO_PORT_START,
            stop_vm_on_wait: false,
            shared_vm_ports: true,
            stdin_task: stdio_plan.stdin.1,
            pty_task: None,
            stdout_task: stdio_plan.stdout.1,
            stderr_task: stdio_plan.stderr.1,
            socket_tasks: socket_relays.tasks,
            socket_proxy_ids: socket_relays.proxy_ids,
            exit_status: None,
        })),
        startup_timing,
        state: PhantomData,
    })
}
async fn start_on_vm_container_pty<Vm: VmContext>(
    builder: ContainerBuilder<Vm, ReadyPty>,
) -> Result<Container<Pty>> {
    let vm = builder
        .vm
        .as_deref()
        .cloned()
        .ok_or_else(|| Error::RuntimeOperation {
            operation: "spawn pty on existing VM",
            reason: "builder does not carry a running VM handle".to_owned(),
        })?;
    let RootfsChoice::OnVm(rootfs) = builder.rootfs.as_ref().expect("ready builder has rootfs")
    else {
        return Err(Error::RuntimeOperation {
            operation: "spawn pty on existing VM",
            reason: "builder rootfs is not a predeclared VM block device".to_owned(),
        });
    };
    let staging = tempfile::tempdir().map_err(|error| Error::RuntimeArtifact {
        operation: "create existing-VM pty runtime staging directory",
        reason: error.to_string(),
    })?;
    let spec = runtime_spec_for_builder(&builder, true, vm.config().rosetta_enabled())?;
    let process_id =
        ProcessId::new(builder.id.to_string()).map_err(|error| Error::RuntimeOperation {
            operation: "build process id",
            reason: error.to_string(),
        })?;
    let mut client = prepare_on_vm_bundle(&vm, &builder, rootfs, &spec).await?;
    let bundle = ContainerBundle::for_id(&builder.id);
    let rootfs_path = runtime_spec_root_path(&builder, &bundle);
    let mut next_socket_port = EXEC_STDIO_PORT_START;
    let socket_relays = prepare_socket_relays(
        &vm,
        &mut client,
        &builder,
        &bundle,
        true,
        &mut next_socket_port,
    )
    .await?;
    let input_port = allocate_vm_stdio_port(vm.id(), "allocate init pty input port")?;
    let output_port = allocate_vm_stdio_port(vm.id(), "allocate init pty output port")?;
    let input_listener = vm
        .listen_reserved_port(input_port)
        .map_err(runtime_vmm_error("listen for pty input"))?;
    let output_listener = vm
        .listen_reserved_port(output_port)
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
        .stdio(ProcessStdio::new().stdin(input_port).stdout(output_port))
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
            staging: RuntimeStaging::Temp(staging),
            next_stdio_port: EXEC_STDIO_PORT_START,
            stop_vm_on_wait: false,
            shared_vm_ports: true,
            stdin_task: None,
            pty_task: None,
            stdout_task: None,
            stderr_task: None,
            socket_tasks: socket_relays.tasks,
            socket_proxy_ids: socket_relays.proxy_ids,
            exit_status: None,
        })),
        startup_timing: ContainerStartupTiming::default(),
        state: PhantomData,
    })
}
async fn prepare_on_vm_bundle(
    vm: &VirtualMachine<Running>,
    builder: &ContainerBuilder<impl VmContext, impl BuilderState>,
    rootfs: &VmRootfs,
    spec: &Spec,
) -> Result<VminitdClient> {
    let mut client = connect_vminitd(vm).await?;
    standard_guest_setup(&mut client).await?;
    let bundle = ContainerBundle::for_id(&builder.id);
    let rootfs_path = match rootfs {
        VmRootfs::BlockDevice(rootfs_device) => {
            client
                .mkdir(tonic::Request::new(bundle.mkdir_rootfs_request(0o755)))
                .await
                .map_err(runtime_rpc_error("mkdir rootfs"))?;
            let rootfs_source = block_device_guest_path(*rootfs_device)?;
            mount_container_rootfs(
                &mut client,
                &bundle,
                &rootfs_source,
                builder.writable_layer.as_ref(),
                builder
                    .writable_layer
                    .as_ref()
                    .map(|writable_layer| writable_layer.source.as_str()),
            )
            .await?;
            bundle.rootfs_path().to_owned()
        }
        VmRootfs::GuestPath(path) => {
            if builder.writable_layer.is_some() {
                return Err(Error::RuntimeOperation {
                    operation: "prepare guest-path rootfs",
                    reason: "writable layers are only supported for block-device rootfses"
                        .to_owned(),
                });
            }
            path.as_str().to_owned()
        }
    };
    if vm.config().rosetta_enabled() {
        RosettaSetup::amd64()
            .apply_to(&mut client)
            .await
            .map_err(runtime_vminitd_error("configure rosetta"))?;
    }
    configure_guest_networks(vm, &mut client, &rootfs_path).await?;
    configure_container_name_files(
        &mut client,
        &rootfs_path,
        builder.dns.as_ref(),
        builder.hosts.as_ref(),
    )
    .await?;
    mount_file_mount_holding_dirs(&mut client, &builder.file_mounts).await?;
    client
        .write_file(tonic::Request::new(
            bundle
                .write_config_request(spec)
                .map_err(runtime_vminitd_error("encode runtime spec"))?,
        ))
        .await
        .map_err(runtime_rpc_error("write config.json"))?;
    Ok(client)
}
async fn run_implicit_output(builder: ContainerBuilder<ImplicitVm, Ready>) -> Result<Output> {
    builder.spawn().await?.wait_with_output().await
}
pub(crate) fn guest_socket_staging_path(container_id: &ContainerId, socket_id: &str) -> String {
    format!("/run/container/{container_id}/sockets/{socket_id}.sock")
}
#[cfg(feature = "snapshot")]
async fn boot_or_restore_vm_with_transient_network_retry(
    config: VmConfig,
    snapshot_path: &Path,
) -> firkin_vmm::Result<VirtualMachine<Running>> {
    let mut attempt = 1;
    loop {
        match VirtualMachine::new(config.clone())
            .boot_or_restore(snapshot_path)
            .await
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_vmexec_start_error_requires_start_process_enoent() {
        let error = Error::RuntimeOperation {
            operation: "start process",
            reason: "status: Internal, message: \"startProcess: failed to start process: vmexec error: exec /sbin/vmexec: No such file or directory\"".to_owned(),
        };

        assert!(is_transient_vmexec_start_error(&error));
    }

    #[test]
    fn transient_vmexec_start_error_rejects_unrelated_operations() {
        let wrong_operation = Error::RuntimeOperation {
            operation: "create process",
            reason: "vmexec error: No such file or directory".to_owned(),
        };
        let wrong_reason = Error::RuntimeOperation {
            operation: "start process",
            reason: "permission denied".to_owned(),
        };

        assert!(!is_transient_vmexec_start_error(&wrong_operation));
        assert!(!is_transient_vmexec_start_error(&wrong_reason));
    }

    #[cfg(all(feature = "snapshot", target_os = "macos"))]
    #[test]
    fn restored_rootfs_clone_candidate_requires_same_existing_volume() {
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("rootfs.ext4");
        std::fs::write(&source, b"rootfs").unwrap();

        let same_volume_dest = source_dir.path().join("restored-rootfs.ext4");
        assert!(restored_rootfs_clone_candidate(&source, &same_volume_dest));

        let missing_parent_dest = source_dir.path().join("missing").join("rootfs.ext4");
        assert!(!restored_rootfs_clone_candidate(
            &source,
            &missing_parent_dest
        ));
    }
}
