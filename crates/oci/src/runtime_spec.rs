use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    #[serde(rename = "ociVersion")]
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks: Option<Hook>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process: Option<Process>,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub domainname: String,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<Root>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linux: Option<Linux>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(rename = "consoleSize", skip_serializing_if = "Option::is_none")]
    pub console_size: Option<BoxSize>,
    #[serde(default, rename = "selinuxLabel")]
    pub selinux_label: String,
    #[serde(default, rename = "noNewPrivileges")]
    pub no_new_privileges: bool,
    #[serde(default, rename = "commandLine")]
    pub command_line: String,
    #[serde(rename = "oomScoreAdj", skip_serializing_if = "Option::is_none")]
    pub oom_score_adj: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<LinuxCapabilities>,
    #[serde(default, rename = "apparmorProfile")]
    pub apparmor_profile: String,
    pub user: User,
    #[serde(default)]
    pub rlimits: Vec<PosixRlimit>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub terminal: bool,
}

impl Default for Process {
    fn default() -> Self {
        Self {
            cwd: "/".to_owned(),
            env: Vec::new(),
            console_size: None,
            selinux_label: String::new(),
            no_new_privileges: false,
            command_line: String::new(),
            oom_score_adj: None,
            capabilities: None,
            apparmor_profile: String::new(),
            user: User::default(),
            rlimits: Vec::new(),
            args: Vec::new(),
            terminal: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounding: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inheritable: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permitted: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambient: Option<Vec<String>>,
}

impl LinuxCapabilities {
    #[must_use]
    pub fn default_oci() -> Self {
        let caps = [
            "CAP_CHOWN",
            "CAP_DAC_OVERRIDE",
            "CAP_FSETID",
            "CAP_FOWNER",
            "CAP_MKNOD",
            "CAP_NET_RAW",
            "CAP_SETGID",
            "CAP_SETUID",
            "CAP_SETFCAP",
            "CAP_SETPCAP",
            "CAP_NET_BIND_SERVICE",
            "CAP_SYS_CHROOT",
            "CAP_KILL",
            "CAP_AUDIT_WRITE",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        Self {
            bounding: Some(caps.clone()),
            effective: Some(caps.clone()),
            inheritable: Some(Vec::new()),
            permitted: Some(caps),
            ambient: Some(Vec::new()),
        }
    }

    #[must_use]
    pub fn same_set(caps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let caps = caps.into_iter().map(Into::into).collect::<Vec<_>>();
        Self {
            bounding: Some(caps.clone()),
            effective: Some(caps.clone()),
            inheritable: Some(caps.clone()),
            permitted: Some(caps.clone()),
            ambient: Some(caps),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoxSize {
    pub height: u64,
    pub width: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub uid: u32,
    pub gid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub umask: Option<u32>,
    #[serde(default, rename = "additionalGids")]
    pub additional_gids: Vec<u32>,
    #[serde(default)]
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Root {
    pub path: String,
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub source: String,
    pub destination: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(rename = "uidMappings", skip_serializing_if = "Option::is_none")]
    pub uid_mappings: Option<Vec<LinuxIDMapping>>,
    #[serde(rename = "gidMappings", skip_serializing_if = "Option::is_none")]
    pub gid_mappings: Option<Vec<LinuxIDMapping>>,
}

impl Mount {
    #[must_use]
    pub fn virtiofs(tag: impl Into<String>, destination: impl Into<String>) -> Self {
        Self::custom("virtiofs", tag, destination)
    }

    #[must_use]
    pub fn bind(source: impl Into<String>, destination: impl Into<String>) -> Self {
        Self::custom("bind", source, destination)
            .extra_option("bind")
            .extra_option("rw")
            .extra_option("rbind")
    }

    #[must_use]
    pub fn block(
        format: impl Into<String>,
        source: impl Into<String>,
        destination: impl Into<String>,
    ) -> Self {
        Self::custom(format, source, destination)
    }

    #[must_use]
    pub fn tmpfs(destination: impl Into<String>) -> Self {
        Self::custom("tmpfs", "tmpfs", destination)
    }

    #[must_use]
    pub fn overlay(
        lower: impl IntoIterator<Item = impl Into<String>>,
        upper: impl Into<String>,
        work: impl Into<String>,
        destination: impl Into<String>,
    ) -> Self {
        let lower = lower.into_iter().map(Into::into).collect::<Vec<_>>();
        Self::custom("overlay", "overlay", destination)
            .extra_option(format!("lowerdir={}", lower.join(":")))
            .extra_option(format!("upperdir={}", upper.into()))
            .extra_option(format!("workdir={}", work.into()))
    }

    #[must_use]
    pub fn proc(destination: impl Into<String>) -> Self {
        Self::custom("proc", "proc", destination)
    }

    #[must_use]
    pub fn sysfs(destination: impl Into<String>) -> Self {
        Self::custom("sysfs", "sysfs", destination).default_secure_options()
    }

    #[must_use]
    pub fn devtmpfs(destination: impl Into<String>) -> Self {
        Self::custom("devtmpfs", "none", destination)
            .no_suid()
            .extra_option("mode=755")
    }

    #[must_use]
    pub fn devpts(destination: impl Into<String>) -> Self {
        Self::custom("devpts", "devpts", destination)
            .no_suid()
            .no_exec()
            .extra_option("newinstance")
            .extra_option("gid=5")
            .extra_option("mode=0620")
            .extra_option("ptmxmode=0666")
    }

    #[must_use]
    pub fn mqueue(destination: impl Into<String>) -> Self {
        Self::custom("mqueue", "mqueue", destination).default_secure_options()
    }

    #[must_use]
    pub fn cgroup2(destination: impl Into<String>) -> Self {
        Self::custom("cgroup2", "none", destination).default_secure_options()
    }

    #[must_use]
    pub fn custom(
        kind: impl Into<String>,
        source: impl Into<String>,
        destination: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            source: source.into(),
            destination: destination.into(),
            options: Vec::new(),
            uid_mappings: None,
            gid_mappings: None,
        }
    }

    #[must_use]
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::proc("/proc"),
            Self::sysfs("/sys"),
            Self::devtmpfs("/dev"),
            Self::mqueue("/dev/mqueue"),
            Self::tmpfs("/dev/shm")
                .default_secure_options()
                .extra_option("mode=1777")
                .extra_option("size=65536k"),
            Self::cgroup2("/sys/fs/cgroup"),
            Self::devpts("/dev/pts"),
        ]
    }

    #[must_use]
    pub fn oci_defaults() -> Vec<Self> {
        vec![
            Self::proc("/proc"),
            Self::tmpfs("/dev")
                .no_suid()
                .extra_option("mode=755")
                .extra_option("size=65536k"),
            Self::devpts("/dev/pts"),
            Self::sysfs("/sys"),
            Self::mqueue("/dev/mqueue"),
            Self::tmpfs("/dev/shm")
                .default_secure_options()
                .extra_option("mode=1777")
                .extra_option("size=65536k"),
            Self::cgroup2("/sys/fs/cgroup"),
        ]
    }

    #[must_use]
    pub fn read_only(self) -> Self {
        self.extra_option("ro")
    }

    #[must_use]
    pub fn no_suid(self) -> Self {
        self.extra_option("nosuid")
    }

    #[must_use]
    pub fn no_exec(self) -> Self {
        self.extra_option("noexec")
    }

    #[must_use]
    pub fn no_dev(self) -> Self {
        self.extra_option("nodev")
    }

    #[must_use]
    pub fn relatime(self) -> Self {
        self.extra_option("relatime")
    }

    #[must_use]
    pub fn noatime(self) -> Self {
        self.extra_option("noatime")
    }

    #[must_use]
    pub fn extra_option(mut self, option: impl Into<String>) -> Self {
        let option = option.into();
        if !self.options.iter().any(|existing| existing == &option) {
            self.options.push(option);
        }
        self
    }

    #[must_use]
    fn default_secure_options(self) -> Self {
        self.no_suid().no_exec().no_dev()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hook {
    pub path: String,
    pub args: Vec<String>,
    pub env: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hooks {
    pub prestart: Vec<Hook>,
    #[serde(rename = "createRuntime")]
    pub create_runtime: Vec<Hook>,
    #[serde(rename = "createContainer")]
    pub create_container: Vec<Hook>,
    #[serde(rename = "startContainer")]
    pub start_container: Vec<Hook>,
    pub poststart: Vec<Hook>,
    pub poststop: Vec<Hook>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Linux {
    #[serde(default, rename = "uidMappings")]
    pub uid_mappings: Vec<LinuxIDMapping>,
    #[serde(default, rename = "gidMappings")]
    pub gid_mappings: Vec<LinuxIDMapping>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sysctl: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<LinuxResources>,
    #[serde(default, rename = "cgroupsPath")]
    pub cgroups_path: String,
    #[serde(default)]
    pub namespaces: Vec<LinuxNamespace>,
    #[serde(default)]
    pub devices: Vec<LinuxDevice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seccomp: Option<LinuxSeccomp>,
    #[serde(default, rename = "rootfsPropagation")]
    pub rootfs_propagation: String,
    #[serde(default, rename = "maskedPaths")]
    pub masked_paths: Vec<String>,
    #[serde(default, rename = "readonlyPaths")]
    pub readonly_paths: Vec<String>,
    #[serde(default, rename = "mountLabel")]
    pub mount_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub personality: Option<LinuxPersonality>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxNamespace {
    #[serde(rename = "type")]
    pub kind: LinuxNamespaceType,
    #[serde(default)]
    pub path: String,
}

impl LinuxNamespace {
    #[must_use]
    pub fn unshare(kind: LinuxNamespaceType) -> Self {
        Self {
            kind,
            path: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinuxNamespaceType {
    Pid,
    Network,
    Uts,
    Mount,
    Ipc,
    User,
    Cgroup,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxIDMapping {
    #[serde(rename = "containerID")]
    pub container_id: u32,
    #[serde(rename = "hostID")]
    pub host_id: u32,
    pub size: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PosixRlimit {
    #[serde(rename = "type")]
    pub kind: String,
    pub hard: u64,
    pub soft: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxResources {
    #[serde(default)]
    pub devices: Vec<LinuxDeviceCgroup>,
    #[serde(default, rename = "hugepageLimits")]
    pub hugepage_limits: Vec<LinuxHugepageLimit>,
    #[serde(default)]
    pub unified: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxDeviceCgroup {
    #[serde(default)]
    pub allow: bool,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor: Option<i64>,
    #[serde(default)]
    pub access: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxHugepageLimit {
    #[serde(default)]
    pub pagesize: String,
    #[serde(default)]
    pub limit: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxDevice {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LinuxSeccomp {
    Profile(LinuxSeccompProfile),
    Raw(serde_json::Value),
}

impl From<LinuxSeccompProfile> for LinuxSeccomp {
    fn from(value: LinuxSeccompProfile) -> Self {
        Self::Profile(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxSeccompProfile {
    #[serde(rename = "defaultAction")]
    pub default_action: LinuxSeccompAction,
    #[serde(rename = "defaultErrnoRet", skip_serializing_if = "Option::is_none")]
    pub default_errno_ret: Option<u32>,
    #[serde(default)]
    pub architectures: Vec<LinuxSeccompArch>,
    #[serde(default)]
    pub flags: Vec<LinuxSeccompFlag>,
    #[serde(default, rename = "listenerPath")]
    pub listener_path: String,
    #[serde(default, rename = "listenerMetadata")]
    pub listener_metadata: String,
    #[serde(default)]
    pub syscalls: Vec<LinuxSyscall>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinuxSeccompFlag {
    #[serde(rename = "SECCOMP_FILTER_FLAG_LOG")]
    Log,
    #[serde(rename = "SECCOMP_FILTER_FLAG_SPEC_ALLOW")]
    SpecAllow,
    #[serde(rename = "SECCOMP_FILTER_FLAG_WAIT_KILLABLE_RECV")]
    WaitKillableRecv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinuxSeccompArch {
    #[serde(rename = "SCMP_ARCH_X86")]
    X86,
    #[serde(rename = "SCMP_ARCH_X86_64")]
    X86_64,
    #[serde(rename = "SCMP_ARCH_X32")]
    X32,
    #[serde(rename = "SCMP_ARCH_ARM")]
    Arm,
    #[serde(rename = "SCMP_ARCH_AARCH64")]
    Aarch64,
    #[serde(rename = "SCMP_ARCH_MIPS")]
    Mips,
    #[serde(rename = "SCMP_ARCH_MIPS64")]
    Mips64,
    #[serde(rename = "SCMP_ARCH_MIPS64N32")]
    Mips64N32,
    #[serde(rename = "SCMP_ARCH_MIPSEL")]
    MipsEl,
    #[serde(rename = "SCMP_ARCH_MIPSEL64")]
    MipsEl64,
    #[serde(rename = "SCMP_ARCH_MIPSEL64N32")]
    MipsEl64N32,
    #[serde(rename = "SCMP_ARCH_PPC")]
    Ppc,
    #[serde(rename = "SCMP_ARCH_PPC64")]
    Ppc64,
    #[serde(rename = "SCMP_ARCH_PPC64LE")]
    Ppc64Le,
    #[serde(rename = "SCMP_ARCH_S390")]
    S390,
    #[serde(rename = "SCMP_ARCH_S390X")]
    S390X,
    #[serde(rename = "SCMP_ARCH_PARISC")]
    Parisc,
    #[serde(rename = "SCMP_ARCH_PARISC64")]
    Parisc64,
    #[serde(rename = "SCMP_ARCH_RISCV64")]
    Riscv64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinuxSeccompAction {
    #[serde(rename = "SCMP_ACT_KILL")]
    Kill,
    #[serde(rename = "SCMP_ACT_KILL_PROCESS")]
    KillProcess,
    #[serde(rename = "SCMP_ACT_KILL_THREAD")]
    KillThread,
    #[serde(rename = "SCMP_ACT_TRAP")]
    Trap,
    #[serde(rename = "SCMP_ACT_ERRNO")]
    Errno,
    #[serde(rename = "SCMP_ACT_TRACE")]
    Trace,
    #[serde(rename = "SCMP_ACT_ALLOW")]
    Allow,
    #[serde(rename = "SCMP_ACT_LOG")]
    Log,
    #[serde(rename = "SCMP_ACT_NOTIFY")]
    Notify,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinuxSeccompOperator {
    #[serde(rename = "SCMP_CMP_NE")]
    NotEqual,
    #[serde(rename = "SCMP_CMP_LT")]
    LessThan,
    #[serde(rename = "SCMP_CMP_LE")]
    LessThanOrEqual,
    #[serde(rename = "SCMP_CMP_EQ")]
    EqualTo,
    #[serde(rename = "SCMP_CMP_GE")]
    GreaterThanOrEqual,
    #[serde(rename = "SCMP_CMP_GT")]
    GreaterThan,
    #[serde(rename = "SCMP_CMP_MASKED_EQ")]
    MaskedEqual,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxSeccompArg {
    pub index: u32,
    pub value: u64,
    #[serde(rename = "valueTwo", skip_serializing_if = "Option::is_none")]
    pub value_two: Option<u64>,
    pub op: LinuxSeccompOperator,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxSyscall {
    pub names: Vec<String>,
    pub action: LinuxSeccompAction,
    #[serde(rename = "errnoRet", skip_serializing_if = "Option::is_none")]
    pub errno_ret: Option<u32>,
    #[serde(default)]
    pub args: Vec<LinuxSeccompArg>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxPersonality {}
