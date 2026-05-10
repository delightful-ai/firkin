//! process — auto-split from the parent module by `split-by-grouping`.
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
use crate::builder::{CommandSet, CommandSetPty, ContainerStdio, MissingCommand};
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use crate::io::{ChildStderr, ChildStdin, ChildStdout, Pty, PtyConfig, Stdio, Streams};
#[allow(unused_imports)]
use crate::os_strings_to_strings;
#[allow(unused_imports)]
use crate::read_pty_optional;
#[allow(unused_imports)]
use crate::read_stderr_optional;
#[allow(unused_imports)]
use crate::read_stdout_optional;
#[allow(unused_imports)]
use crate::runtime::ProcessRuntime;
#[allow(unused_imports)]
use firkin_oci::LinuxCapabilities as OciLinuxCapabilities;
#[allow(unused_imports)]
use firkin_oci::PosixRlimit as OciPosixRlimit;
#[allow(unused_imports)]
use firkin_oci::User as OciUser;
#[allow(unused_imports)]
use firkin_oci::{
    Linux, LinuxNamespace, LinuxNamespaceType, LinuxResources, Process as OciProcess, Root, Spec,
};
#[allow(unused_imports)]
use firkin_types::ContainerId;
#[allow(unused_imports)]
use firkin_types::ProcessId;
#[cfg(test)]
#[allow(unused_imports)]
use firkin_vminitd_client::ContainerBundle;
#[allow(unused_imports)]
use std::ffi::OsString;
#[allow(unused_imports)]
use std::marker::PhantomData;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
#[allow(unused_imports)]
use tokio::sync::Mutex;
pub(crate) const DEFAULT_PATH_ENV: &str =
    "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
pub(crate) const SIGTERM: Signal = Signal::new(15);
pub(crate) const SIGKILL: Signal = Signal::new(9);
/// Unix signal number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Signal(i32);
impl Signal {
    /// Construct a signal number.
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }
    /// Return the raw signal number.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }
}
/// Additional context for signaled exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KilledReason {
    /// Guest kernel OOM-killed the process.
    OomKill,
    /// A signal sent by the host-side library.
    SignalFromHost {
        /// Signal sent by the host.
        signal: Signal,
    },
    /// A signal sent from inside the guest.
    SignalFromGuest {
        /// Signal sent from inside the guest.
        signal: Signal,
    },
}
/// Container process exit status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    code: Option<i32>,
    signal: Option<Signal>,
    killed_reason: Option<KilledReason>,
}
impl ExitStatus {
    /// Construct a normal process exit status.
    #[must_use]
    pub const fn from_code(code: i32) -> Self {
        Self {
            code: Some(code),
            signal: None,
            killed_reason: None,
        }
    }
    /// Construct a signal-terminated process status.
    #[must_use]
    pub const fn from_signal(signal: Signal, reason: KilledReason) -> Self {
        Self {
            code: None,
            signal: Some(signal),
            killed_reason: Some(reason),
        }
    }
    /// True iff the process exited with code 0.
    #[must_use]
    pub const fn success(&self) -> bool {
        matches!(self.code, Some(0)) && self.signal.is_none()
    }
    /// Exit code, if the process exited normally.
    #[must_use]
    pub const fn code(&self) -> Option<i32> {
        self.code
    }
    /// Terminating signal, if any.
    #[must_use]
    pub const fn signal(&self) -> Option<Signal> {
        self.signal
    }
    /// Context for signal-terminated exits.
    #[must_use]
    pub const fn killed_reason(&self) -> Option<KilledReason> {
        self.killed_reason
    }
}
/// Captured process output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// Process exit status.
    pub status: ExitStatus,
    /// Captured stdout.
    pub stdout: Vec<u8>,
    /// Captured stderr.
    pub stderr: Vec<u8>,
}
/// Process identity inside the container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum User {
    /// A name resolved inside the guest via `/etc/passwd` at process start.
    Named(String),
    /// Numeric uid + gid with optional supplementary groups.
    Numeric {
        /// User ID.
        uid: u32,
        /// Primary group ID.
        gid: u32,
        /// Supplementary group IDs.
        extra_groups: Vec<u32>,
    },
}
impl User {
    /// The root user, uid 0 gid 0.
    #[must_use]
    pub fn root() -> Self {
        Self::Numeric {
            uid: 0,
            gid: 0,
            extra_groups: Vec::new(),
        }
    }
    /// A user name resolved inside the guest.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into())
    }
    /// A numeric uid + gid.
    #[must_use]
    pub fn numeric(uid: u32, gid: u32) -> Self {
        Self::Numeric {
            uid,
            gid,
            extra_groups: Vec::new(),
        }
    }
    /// Set supplementary groups on this user.
    #[must_use]
    pub fn with_extra_groups(mut self, groups: Vec<u32>) -> Self {
        if let Self::Numeric { extra_groups, .. } = &mut self {
            *extra_groups = groups;
        }
        self
    }
    pub(crate) fn into_oci(self) -> OciUser {
        match self {
            Self::Named(username) => OciUser {
                username,
                ..OciUser::default()
            },
            Self::Numeric {
                uid,
                gid,
                extra_groups,
            } => OciUser {
                uid,
                gid,
                additional_gids: extra_groups,
                ..OciUser::default()
            },
        }
    }
}
impl Default for User {
    fn default() -> Self {
        Self::root()
    }
}
impl From<u32> for User {
    fn from(uid: u32) -> Self {
        Self::numeric(uid, uid)
    }
}
/// Linux capability sets for a process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinuxCapabilities {
    /// Bounding capability set.
    pub bounding: Vec<Capability>,
    /// Effective capability set.
    pub effective: Vec<Capability>,
    /// Inheritable capability set.
    pub inheritable: Vec<Capability>,
    /// Permitted capability set.
    pub permitted: Vec<Capability>,
    /// Ambient capability set.
    pub ambient: Vec<Capability>,
}
impl LinuxCapabilities {
    /// Runc's default OCI capability set.
    #[must_use]
    pub fn default_oci() -> Self {
        Self::single_set(vec![
            Capability::Chown,
            Capability::DacOverride,
            Capability::Fsetid,
            Capability::Fowner,
            Capability::Mknod,
            Capability::NetRaw,
            Capability::Setgid,
            Capability::Setuid,
            Capability::Setfcap,
            Capability::Setpcap,
            Capability::NetBindService,
            Capability::SysChroot,
            Capability::Kill,
            Capability::AuditWrite,
        ])
    }
    /// Every known capability in every set.
    #[must_use]
    pub fn all() -> Self {
        let caps = Capability::ALL.to_vec();
        Self {
            bounding: caps.clone(),
            effective: caps.clone(),
            inheritable: caps.clone(),
            permitted: caps.clone(),
            ambient: caps,
        }
    }
    /// Empty capability sets.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            bounding: Vec::new(),
            effective: Vec::new(),
            inheritable: Vec::new(),
            permitted: Vec::new(),
            ambient: Vec::new(),
        }
    }
    /// Use the given set for bounding, effective, and permitted.
    #[must_use]
    pub fn single_set(caps: Vec<Capability>) -> Self {
        Self {
            bounding: caps.clone(),
            effective: caps.clone(),
            inheritable: Vec::new(),
            permitted: caps,
            ambient: Vec::new(),
        }
    }
    pub(crate) fn into_oci(self) -> OciLinuxCapabilities {
        OciLinuxCapabilities {
            bounding: Some(capabilities_to_oci(self.bounding)),
            effective: Some(capabilities_to_oci(self.effective)),
            inheritable: Some(capabilities_to_oci(self.inheritable)),
            permitted: Some(capabilities_to_oci(self.permitted)),
            ambient: Some(capabilities_to_oci(self.ambient)),
        }
    }
}
fn capabilities_to_oci(caps: Vec<Capability>) -> Vec<String> {
    caps.into_iter()
        .map(|capability| capability.as_cap_str().to_owned())
        .collect()
}
/// Linux capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// `CAP_CHOWN`.
    Chown,
    /// `CAP_DAC_OVERRIDE`.
    DacOverride,
    /// `CAP_DAC_READ_SEARCH`.
    DacReadSearch,
    /// `CAP_FOWNER`.
    Fowner,
    /// `CAP_FSETID`.
    Fsetid,
    /// `CAP_KILL`.
    Kill,
    /// `CAP_SETGID`.
    Setgid,
    /// `CAP_SETUID`.
    Setuid,
    /// `CAP_SETPCAP`.
    Setpcap,
    /// `CAP_LINUX_IMMUTABLE`.
    LinuxImmutable,
    /// `CAP_NET_BIND_SERVICE`.
    NetBindService,
    /// `CAP_NET_BROADCAST`.
    NetBroadcast,
    /// `CAP_NET_ADMIN`.
    NetAdmin,
    /// `CAP_NET_RAW`.
    NetRaw,
    /// `CAP_IPC_LOCK`.
    IpcLock,
    /// `CAP_IPC_OWNER`.
    IpcOwner,
    /// `CAP_SYS_MODULE`.
    SysModule,
    /// `CAP_SYS_RAWIO`.
    SysRawio,
    /// `CAP_SYS_CHROOT`.
    SysChroot,
    /// `CAP_SYS_PTRACE`.
    SysPtrace,
    /// `CAP_SYS_PACCT`.
    SysPacct,
    /// `CAP_SYS_ADMIN`.
    SysAdmin,
    /// `CAP_SYS_BOOT`.
    SysBoot,
    /// `CAP_SYS_NICE`.
    SysNice,
    /// `CAP_SYS_RESOURCE`.
    SysResource,
    /// `CAP_SYS_TIME`.
    SysTime,
    /// `CAP_SYS_TTY_CONFIG`.
    SysTtyConfig,
    /// `CAP_MKNOD`.
    Mknod,
    /// `CAP_LEASE`.
    Lease,
    /// `CAP_AUDIT_WRITE`.
    AuditWrite,
    /// `CAP_AUDIT_CONTROL`.
    AuditControl,
    /// `CAP_SETFCAP`.
    Setfcap,
    /// `CAP_MAC_OVERRIDE`.
    MacOverride,
    /// `CAP_MAC_ADMIN`.
    MacAdmin,
    /// `CAP_SYSLOG`.
    Syslog,
    /// `CAP_WAKE_ALARM`.
    WakeAlarm,
    /// `CAP_BLOCK_SUSPEND`.
    BlockSuspend,
    /// `CAP_AUDIT_READ`.
    AuditRead,
    /// `CAP_PERFMON`.
    Perfmon,
    /// `CAP_BPF`.
    Bpf,
    /// `CAP_CHECKPOINT_RESTORE`.
    CheckpointRestore,
}
impl Capability {
    /// Every known capability variant.
    pub const ALL: &'static [Self] = &[
        Self::Chown,
        Self::DacOverride,
        Self::DacReadSearch,
        Self::Fowner,
        Self::Fsetid,
        Self::Kill,
        Self::Setgid,
        Self::Setuid,
        Self::Setpcap,
        Self::LinuxImmutable,
        Self::NetBindService,
        Self::NetBroadcast,
        Self::NetAdmin,
        Self::NetRaw,
        Self::IpcLock,
        Self::IpcOwner,
        Self::SysModule,
        Self::SysRawio,
        Self::SysChroot,
        Self::SysPtrace,
        Self::SysPacct,
        Self::SysAdmin,
        Self::SysBoot,
        Self::SysNice,
        Self::SysResource,
        Self::SysTime,
        Self::SysTtyConfig,
        Self::Mknod,
        Self::Lease,
        Self::AuditWrite,
        Self::AuditControl,
        Self::Setfcap,
        Self::MacOverride,
        Self::MacAdmin,
        Self::Syslog,
        Self::WakeAlarm,
        Self::BlockSuspend,
        Self::AuditRead,
        Self::Perfmon,
        Self::Bpf,
        Self::CheckpointRestore,
    ];
    /// OCI-compatible capability string.
    #[must_use]
    pub const fn as_cap_str(self) -> &'static str {
        match self {
            Self::Chown => "CAP_CHOWN",
            Self::DacOverride => "CAP_DAC_OVERRIDE",
            Self::DacReadSearch => "CAP_DAC_READ_SEARCH",
            Self::Fowner => "CAP_FOWNER",
            Self::Fsetid => "CAP_FSETID",
            Self::Kill => "CAP_KILL",
            Self::Setgid => "CAP_SETGID",
            Self::Setuid => "CAP_SETUID",
            Self::Setpcap => "CAP_SETPCAP",
            Self::LinuxImmutable => "CAP_LINUX_IMMUTABLE",
            Self::NetBindService => "CAP_NET_BIND_SERVICE",
            Self::NetBroadcast => "CAP_NET_BROADCAST",
            Self::NetAdmin => "CAP_NET_ADMIN",
            Self::NetRaw => "CAP_NET_RAW",
            Self::IpcLock => "CAP_IPC_LOCK",
            Self::IpcOwner => "CAP_IPC_OWNER",
            Self::SysModule => "CAP_SYS_MODULE",
            Self::SysRawio => "CAP_SYS_RAWIO",
            Self::SysChroot => "CAP_SYS_CHROOT",
            Self::SysPtrace => "CAP_SYS_PTRACE",
            Self::SysPacct => "CAP_SYS_PACCT",
            Self::SysAdmin => "CAP_SYS_ADMIN",
            Self::SysBoot => "CAP_SYS_BOOT",
            Self::SysNice => "CAP_SYS_NICE",
            Self::SysResource => "CAP_SYS_RESOURCE",
            Self::SysTime => "CAP_SYS_TIME",
            Self::SysTtyConfig => "CAP_SYS_TTY_CONFIG",
            Self::Mknod => "CAP_MKNOD",
            Self::Lease => "CAP_LEASE",
            Self::AuditWrite => "CAP_AUDIT_WRITE",
            Self::AuditControl => "CAP_AUDIT_CONTROL",
            Self::Setfcap => "CAP_SETFCAP",
            Self::MacOverride => "CAP_MAC_OVERRIDE",
            Self::MacAdmin => "CAP_MAC_ADMIN",
            Self::Syslog => "CAP_SYSLOG",
            Self::WakeAlarm => "CAP_WAKE_ALARM",
            Self::BlockSuspend => "CAP_BLOCK_SUSPEND",
            Self::AuditRead => "CAP_AUDIT_READ",
            Self::Perfmon => "CAP_PERFMON",
            Self::Bpf => "CAP_BPF",
            Self::CheckpointRestore => "CAP_CHECKPOINT_RESTORE",
        }
    }
    /// Parse a capability string, with or without the `CAP_` prefix.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidCapability`] when the input does not name a known capability.
    pub fn parse(raw: &str) -> std::result::Result<Self, InvalidCapability> {
        let upper = raw.trim().to_ascii_uppercase();
        let name = upper.strip_prefix("CAP_").unwrap_or(&upper);
        match name {
            "CHOWN" => Ok(Self::Chown),
            "DAC_OVERRIDE" => Ok(Self::DacOverride),
            "DAC_READ_SEARCH" => Ok(Self::DacReadSearch),
            "FOWNER" => Ok(Self::Fowner),
            "FSETID" => Ok(Self::Fsetid),
            "KILL" => Ok(Self::Kill),
            "SETGID" => Ok(Self::Setgid),
            "SETUID" => Ok(Self::Setuid),
            "SETPCAP" => Ok(Self::Setpcap),
            "LINUX_IMMUTABLE" => Ok(Self::LinuxImmutable),
            "NET_BIND_SERVICE" => Ok(Self::NetBindService),
            "NET_BROADCAST" => Ok(Self::NetBroadcast),
            "NET_ADMIN" => Ok(Self::NetAdmin),
            "NET_RAW" => Ok(Self::NetRaw),
            "IPC_LOCK" => Ok(Self::IpcLock),
            "IPC_OWNER" => Ok(Self::IpcOwner),
            "SYS_MODULE" => Ok(Self::SysModule),
            "SYS_RAWIO" => Ok(Self::SysRawio),
            "SYS_CHROOT" => Ok(Self::SysChroot),
            "SYS_PTRACE" => Ok(Self::SysPtrace),
            "SYS_PACCT" => Ok(Self::SysPacct),
            "SYS_ADMIN" => Ok(Self::SysAdmin),
            "SYS_BOOT" => Ok(Self::SysBoot),
            "SYS_NICE" => Ok(Self::SysNice),
            "SYS_RESOURCE" => Ok(Self::SysResource),
            "SYS_TIME" => Ok(Self::SysTime),
            "SYS_TTY_CONFIG" => Ok(Self::SysTtyConfig),
            "MKNOD" => Ok(Self::Mknod),
            "LEASE" => Ok(Self::Lease),
            "AUDIT_WRITE" => Ok(Self::AuditWrite),
            "AUDIT_CONTROL" => Ok(Self::AuditControl),
            "SETFCAP" => Ok(Self::Setfcap),
            "MAC_OVERRIDE" => Ok(Self::MacOverride),
            "MAC_ADMIN" => Ok(Self::MacAdmin),
            "SYSLOG" => Ok(Self::Syslog),
            "WAKE_ALARM" => Ok(Self::WakeAlarm),
            "BLOCK_SUSPEND" => Ok(Self::BlockSuspend),
            "AUDIT_READ" => Ok(Self::AuditRead),
            "PERFMON" => Ok(Self::Perfmon),
            "BPF" => Ok(Self::Bpf),
            "CHECKPOINT_RESTORE" => Ok(Self::CheckpointRestore),
            _ => Err(InvalidCapability(raw.to_owned())),
        }
    }
}
/// Unknown Linux capability name.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
#[error("unknown capability `{0}`")]
pub struct InvalidCapability(String);
/// POSIX resource limit for a Linux process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LinuxRlimit {
    /// Limit kind.
    pub kind: RlimitKind,
    /// Hard limit.
    pub hard: u64,
    /// Soft limit.
    pub soft: u64,
}
impl LinuxRlimit {
    /// Construct a resource limit.
    #[must_use]
    pub const fn new(kind: RlimitKind, hard: u64, soft: u64) -> Self {
        Self { kind, hard, soft }
    }
    /// Construct a resource limit where hard and soft limits match.
    #[must_use]
    pub const fn symmetric(kind: RlimitKind, limit: u64) -> Self {
        Self {
            kind,
            hard: limit,
            soft: limit,
        }
    }
    pub(crate) fn into_oci(self) -> OciPosixRlimit {
        OciPosixRlimit {
            kind: self.kind.as_rlimit_str().to_owned(),
            hard: self.hard,
            soft: self.soft,
        }
    }
}
/// POSIX resource-limit kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RlimitKind {
    /// `RLIMIT_AS`.
    AddressSpace,
    /// `RLIMIT_CORE`.
    CoreFileSize,
    /// `RLIMIT_CPU`.
    CpuTime,
    /// `RLIMIT_DATA`.
    DataSize,
    /// `RLIMIT_FSIZE`.
    FileSize,
    /// `RLIMIT_LOCKS`.
    Locks,
    /// `RLIMIT_MEMLOCK`.
    LockedMemory,
    /// `RLIMIT_MSGQUEUE`.
    MessageQueue,
    /// `RLIMIT_NICE`.
    Nice,
    /// `RLIMIT_NOFILE`.
    OpenFiles,
    /// `RLIMIT_NPROC`.
    NumberOfProcesses,
    /// `RLIMIT_RSS`.
    ResidentSetSize,
    /// `RLIMIT_RTPRIO`.
    RealtimePriority,
    /// `RLIMIT_RTTIME`.
    RealtimeTimeout,
    /// `RLIMIT_SIGPENDING`.
    SignalsPending,
    /// `RLIMIT_STACK`.
    StackSize,
}
impl RlimitKind {
    /// OCI-compatible rlimit string.
    #[must_use]
    pub const fn as_rlimit_str(self) -> &'static str {
        match self {
            Self::AddressSpace => "RLIMIT_AS",
            Self::CoreFileSize => "RLIMIT_CORE",
            Self::CpuTime => "RLIMIT_CPU",
            Self::DataSize => "RLIMIT_DATA",
            Self::FileSize => "RLIMIT_FSIZE",
            Self::Locks => "RLIMIT_LOCKS",
            Self::LockedMemory => "RLIMIT_MEMLOCK",
            Self::MessageQueue => "RLIMIT_MSGQUEUE",
            Self::Nice => "RLIMIT_NICE",
            Self::OpenFiles => "RLIMIT_NOFILE",
            Self::NumberOfProcesses => "RLIMIT_NPROC",
            Self::ResidentSetSize => "RLIMIT_RSS",
            Self::RealtimePriority => "RLIMIT_RTPRIO",
            Self::RealtimeTimeout => "RLIMIT_RTTIME",
            Self::SignalsPending => "RLIMIT_SIGPENDING",
            Self::StackSize => "RLIMIT_STACK",
        }
    }
    /// Parse an rlimit kind, with or without the `RLIMIT_` prefix.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRlimit`] when the input does not name a known rlimit.
    pub fn parse(raw: &str) -> std::result::Result<Self, InvalidRlimit> {
        let upper = raw.trim().to_ascii_uppercase();
        let name = upper.strip_prefix("RLIMIT_").unwrap_or(&upper);
        match name {
            "AS" => Ok(Self::AddressSpace),
            "CORE" => Ok(Self::CoreFileSize),
            "CPU" => Ok(Self::CpuTime),
            "DATA" => Ok(Self::DataSize),
            "FSIZE" => Ok(Self::FileSize),
            "LOCKS" => Ok(Self::Locks),
            "MEMLOCK" => Ok(Self::LockedMemory),
            "MSGQUEUE" => Ok(Self::MessageQueue),
            "NICE" => Ok(Self::Nice),
            "NOFILE" => Ok(Self::OpenFiles),
            "NPROC" => Ok(Self::NumberOfProcesses),
            "RSS" => Ok(Self::ResidentSetSize),
            "RTPRIO" => Ok(Self::RealtimePriority),
            "RTTIME" => Ok(Self::RealtimeTimeout),
            "SIGPENDING" => Ok(Self::SignalsPending),
            "STACK" => Ok(Self::StackSize),
            _ => Err(InvalidRlimit(raw.to_owned())),
        }
    }
}
/// Unknown Linux rlimit name.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
#[error("unknown rlimit `{0}`")]
pub struct InvalidRlimit(String);
/// Pre-start configuration for an exec'd process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecConfig<S = Streams> {
    pub(crate) command: Vec<OsString>,
    pub(crate) env: Vec<OsString>,
    pub(crate) working_dir: PathBuf,
    pub(crate) user: User,
    pub(crate) no_new_privileges: bool,
    pub(crate) capabilities: LinuxCapabilities,
    pub(crate) rlimits: Vec<LinuxRlimit>,
    pub(crate) selinux_label: String,
    pub(crate) apparmor_profile: String,
    pub(crate) stdin: Stdio,
    pub(crate) stdout: Stdio,
    pub(crate) stderr: Stdio,
    pub(crate) pty: Option<PtyConfig>,
    pub(crate) state: PhantomData<S>,
}
impl ExecConfig<Streams> {
    /// Start an exec config builder.
    #[must_use]
    pub fn builder() -> ExecConfigBuilder<MissingCommand> {
        ExecConfigBuilder {
            command: Vec::new(),
            env: Vec::new(),
            working_dir: PathBuf::new(),
            user: User::default(),
            no_new_privileges: false,
            capabilities: LinuxCapabilities::default_oci(),
            rlimits: Vec::new(),
            selinux_label: String::new(),
            apparmor_profile: String::new(),
            stdin: Stdio::Null,
            stdout: Stdio::Null,
            stderr: Stdio::Null,
            pty: None,
            state: PhantomData,
        }
    }
}
impl<S> ExecConfig<S> {
    #[cfg(test)]
    pub(crate) fn runtime_spec(&self, container_id: &ContainerId, terminal: bool) -> Result<Spec> {
        let bundle = ContainerBundle::for_id(container_id);
        self.runtime_spec_with_root_path(container_id, bundle.rootfs_path(), terminal)
    }
    pub(crate) fn runtime_spec_with_root_path(
        &self,
        container_id: &ContainerId,
        rootfs_path: &str,
        terminal: bool,
    ) -> Result<Spec> {
        Ok(Spec {
            version: "1.1.0".to_owned(),
            process: Some(OciProcess {
                args: os_strings_to_strings(&self.command, |index| {
                    Error::InvalidCommandArgument { index }
                })?,
                env: os_strings_to_strings(&self.env, |index| Error::InvalidEnvironmentVariable {
                    index,
                })?,
                cwd: self
                    .working_dir
                    .to_str()
                    .ok_or(Error::InvalidWorkingDirectory)?
                    .to_owned(),
                user: self.user.clone().into_oci(),
                no_new_privileges: self.no_new_privileges,
                capabilities: Some(self.capabilities.clone().into_oci()),
                rlimits: self
                    .rlimits
                    .iter()
                    .copied()
                    .map(LinuxRlimit::into_oci)
                    .collect(),
                selinux_label: self.selinux_label.clone(),
                apparmor_profile: self.apparmor_profile.clone(),
                terminal,
                ..OciProcess::default()
            }),
            hostname: container_id.to_string(),
            root: Some(Root {
                path: rootfs_path.to_owned(),
                readonly: false,
            }),
            linux: Some(Linux {
                resources: Some(LinuxResources::default()),
                cgroups_path: format!("/container/{container_id}"),
                namespaces: vec![
                    LinuxNamespace::unshare(LinuxNamespaceType::Pid),
                    LinuxNamespace::unshare(LinuxNamespaceType::Mount),
                    LinuxNamespace::unshare(LinuxNamespaceType::Ipc),
                    LinuxNamespace::unshare(LinuxNamespaceType::Uts),
                ],
                ..Linux::default()
            }),
            ..Spec::default()
        })
    }
}
/// Builder for [`ExecConfig`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecConfigBuilder<S> {
    pub(crate) command: Vec<OsString>,
    pub(crate) env: Vec<OsString>,
    pub(crate) working_dir: PathBuf,
    pub(crate) user: User,
    pub(crate) no_new_privileges: bool,
    pub(crate) capabilities: LinuxCapabilities,
    pub(crate) rlimits: Vec<LinuxRlimit>,
    pub(crate) selinux_label: String,
    pub(crate) apparmor_profile: String,
    pub(crate) stdin: Stdio,
    pub(crate) stdout: Stdio,
    pub(crate) stderr: Stdio,
    pub(crate) pty: Option<PtyConfig>,
    pub(crate) state: PhantomData<S>,
}
impl ExecConfigBuilder<MissingCommand> {
    /// Set command arguments and advance to the buildable state.
    #[must_use]
    pub fn command<I, A>(self, args: I) -> ExecConfigBuilder<CommandSet>
    where
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        ExecConfigBuilder {
            command: args.into_iter().map(Into::into).collect(),
            env: self.env,
            working_dir: self.working_dir,
            user: self.user,
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities,
            rlimits: self.rlimits,
            selinux_label: self.selinux_label,
            apparmor_profile: self.apparmor_profile,
            stdin: self.stdin,
            stdout: self.stdout,
            stderr: self.stderr,
            pty: self.pty,
            state: PhantomData,
        }
    }
}
impl<S> ExecConfigBuilder<S> {
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
    /// Set the process `no_new_privileges` bit.
    #[must_use]
    pub const fn no_new_privileges(mut self, on: bool) -> Self {
        self.no_new_privileges = on;
        self
    }
    /// Set the Linux capability sets for the process.
    #[must_use]
    pub fn capabilities(mut self, capabilities: LinuxCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
    /// Append one POSIX resource limit.
    #[must_use]
    pub fn rlimit(mut self, rlimit: LinuxRlimit) -> Self {
        self.rlimits.push(rlimit);
        self
    }
    /// Append POSIX resource limits.
    #[must_use]
    pub fn rlimits<I>(mut self, rlimits: I) -> Self
    where
        I: IntoIterator<Item = LinuxRlimit>,
    {
        self.rlimits.extend(rlimits);
        self
    }
    /// Set the `SELinux` label for the process.
    #[must_use]
    pub fn selinux_label(mut self, label: impl Into<String>) -> Self {
        self.selinux_label = label.into();
        self
    }
    /// Set the `AppArmor` profile for the process.
    #[must_use]
    pub fn apparmor_profile(mut self, profile: impl Into<String>) -> Self {
        self.apparmor_profile = profile.into();
        self
    }
    /// Set stdin behavior for the exec'd process.
    #[must_use]
    pub const fn stdin(mut self, stdin: Stdio) -> Self {
        self.stdin = stdin;
        self
    }
}
impl ExecConfigBuilder<CommandSet> {
    /// Set stdout behavior for the exec'd process.
    #[must_use]
    pub const fn stdout(mut self, stdout: Stdio) -> Self {
        self.stdout = stdout;
        self
    }
    /// Set stderr behavior for the exec'd process.
    #[must_use]
    pub const fn stderr(mut self, stderr: Stdio) -> Self {
        self.stderr = stderr;
        self
    }
    /// Switch the exec configuration to pseudo-terminal mode.
    #[must_use]
    pub fn pty(self, config: impl Into<PtyConfig>) -> ExecConfigBuilder<CommandSetPty> {
        ExecConfigBuilder {
            command: self.command,
            env: self.env,
            working_dir: self.working_dir,
            user: self.user,
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities,
            rlimits: self.rlimits,
            selinux_label: self.selinux_label,
            apparmor_profile: self.apparmor_profile,
            stdin: self.stdin,
            stdout: Stdio::Null,
            stderr: Stdio::Null,
            pty: Some(config.into()),
            state: PhantomData,
        }
    }
    /// Build a stream-backed exec configuration.
    #[must_use]
    pub fn build(self) -> ExecConfig<Streams> {
        ExecConfig {
            command: self.command,
            env: self.env,
            working_dir: if self.working_dir.as_os_str().is_empty() {
                PathBuf::from("/")
            } else {
                self.working_dir
            },
            user: self.user,
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities,
            rlimits: self.rlimits,
            selinux_label: self.selinux_label,
            apparmor_profile: self.apparmor_profile,
            stdin: self.stdin,
            stdout: self.stdout,
            stderr: self.stderr,
            pty: None,
            state: PhantomData,
        }
    }
}
impl ExecConfigBuilder<CommandSetPty> {
    /// Build a pty-backed exec configuration.
    #[must_use]
    pub fn build(self) -> ExecConfig<Pty> {
        ExecConfig {
            command: self.command,
            env: self.env,
            working_dir: if self.working_dir.as_os_str().is_empty() {
                PathBuf::from("/")
            } else {
                self.working_dir
            },
            user: self.user,
            no_new_privileges: self.no_new_privileges,
            capabilities: self.capabilities,
            rlimits: self.rlimits,
            selinux_label: self.selinux_label,
            apparmor_profile: self.apparmor_profile,
            stdin: self.stdin,
            stdout: Stdio::Null,
            stderr: Stdio::Null,
            pty: self.pty.or(Some(PtyConfig::default())),
            state: PhantomData,
        }
    }
}
/// Live handle for a process exec'd into a container.
#[derive(Debug)]
pub struct Process<S = Streams> {
    pub(crate) id: ProcessId,
    pub(crate) pid: Option<i32>,
    pub(crate) pty: Option<Pty>,
    pub(crate) runtime: Arc<Mutex<ProcessRuntime>>,
    pub(crate) startup_timing: ExecStartupTiming,
    pub(crate) state: PhantomData<S>,
}

/// Host-side phase timing for starting an exec process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecStartupTiming {
    spec_build: Duration,
    stdio_prepare: Duration,
    request_encode: Duration,
    create_process_rpc: Duration,
    start_process_rpc: Duration,
}

impl ExecStartupTiming {
    /// Construct host-side phase timing for an exec process start.
    #[must_use]
    pub const fn new(
        spec_build: Duration,
        stdio_prepare: Duration,
        request_encode: Duration,
        create_process_rpc: Duration,
        start_process_rpc: Duration,
    ) -> Self {
        Self {
            spec_build,
            stdio_prepare,
            request_encode,
            create_process_rpc,
            start_process_rpc,
        }
    }

    /// Time spent building the OCI process spec.
    #[must_use]
    pub const fn spec_build(self) -> Duration {
        self.spec_build
    }

    /// Time spent preparing host stdio listeners/tasks.
    #[must_use]
    pub const fn stdio_prepare(self) -> Duration {
        self.stdio_prepare
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

    /// Time spent in the vminitd start-process RPC.
    #[must_use]
    pub const fn start_process_rpc(self) -> Duration {
        self.start_process_rpc
    }
}

impl<S: ContainerStdio> Process<S> {
    /// Return the process ID.
    #[must_use]
    pub const fn id(&self) -> &ProcessId {
        &self.id
    }
    /// Return the guest-side process PID, if known.
    #[must_use]
    pub const fn pid(&self) -> Option<i32> {
        self.pid
    }
    /// Return host-side phase timing for this exec process start.
    #[must_use]
    pub const fn startup_timing(&self) -> ExecStartupTiming {
        self.startup_timing
    }
    /// Return a signal-only handle for this process.
    #[must_use]
    pub fn kill_handle(&self) -> ProcessKillHandle {
        ProcessKillHandle {
            runtime: self.runtime.clone(),
        }
    }
    /// Take the process stdin handle, if stdin was configured as piped.
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
            Some(task) => await_stdin(task, "accept exec stdin").await.map(Some),
            None => Ok(None),
        }
    }
    /// Take the process stdout handle, if stdout was configured as piped.
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
            Some(task) => await_stdout(task, "accept exec stdout").await.map(Some),
            None => Ok(None),
        }
    }
    /// Take the process stderr handle, if stderr was configured as piped.
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
            Some(task) => await_stderr(task, "accept exec stderr").await.map(Some),
            None => Ok(None),
        }
    }
    /// Wait for the process to exit.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if the vminitd wait RPC fails.
    pub async fn wait(&mut self) -> Result<ExitStatus> {
        let mut runtime = self.runtime.lock().await;
        runtime.wait().await
    }
    /// Wait for the process and collect stdout and stderr.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if wait or stdio capture fails.
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
                tokio::try_join!(wait, read_pty_optional(pty, pty_task, "capture exec pty"))?;
            (status, stdout, Vec::new())
        } else {
            tokio::try_join!(
                wait,
                read_stdout_optional(stdout_task, "capture exec stdout"),
                read_stderr_optional(stderr_task, "capture exec stderr")
            )?
        };
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
    /// Send a signal to the process.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd rejects the kill RPC.
    pub async fn kill(&mut self, signal: Signal) -> Result<()> {
        let mut runtime = self.runtime.lock().await;
        runtime.kill(signal).await
    }
}
impl Process<Pty> {
    /// Return the process pty handle.
    ///
    /// # Panics
    ///
    /// Panics if the pty handle has already been consumed by [`Self::take_pty`].
    #[must_use]
    pub fn pty(&mut self) -> &mut Pty {
        self.pty
            .as_mut()
            .expect("Process<Pty> pty handle was already taken")
    }
    /// Take the process pty handle.
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
            Some(task) => await_pty(task, "accept exec pty").await.map(Some),
            None => Ok(None),
        }
    }
}
/// Signal-only handle for a process exec'd into a container.
///
/// This lets callers keep process control available while another task owns the
/// full [`Process`] handle for stdout, stderr, stdin, and wait orchestration.
#[derive(Clone, Debug)]
pub struct ProcessKillHandle {
    pub(crate) runtime: Arc<Mutex<ProcessRuntime>>,
}
impl ProcessKillHandle {
    /// Send a signal to the process.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd rejects the kill RPC.
    pub async fn kill(&self, signal: Signal) -> Result<()> {
        let mut runtime = self.runtime.lock().await;
        runtime.kill(signal).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_startup_timing_preserves_phase_durations() {
        let timing = ExecStartupTiming::new(
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
            Duration::from_millis(4),
            Duration::from_millis(5),
        );

        assert_eq!(timing.spec_build(), Duration::from_millis(1));
        assert_eq!(timing.stdio_prepare(), Duration::from_millis(2));
        assert_eq!(timing.request_encode(), Duration::from_millis(3));
        assert_eq!(timing.create_process_rpc(), Duration::from_millis(4));
        assert_eq!(timing.start_process_rpc(), Duration::from_millis(5));
    }
}
