//! Container builder typestate tests.

#[cfg(feature = "snapshot")]
use firkin_core::ContainerSnapshotState;
use firkin_core::{
    Capability, Container, CoreContainerFactory, EmptyDirMedium, EmptyDirVolume, Error, ExecConfig,
    ExitStatus, FileMount, KilledReason, LinuxCapabilities, LinuxRlimit, Mount, Output, PodBuilder,
    PodContainerSpec, PodRootfsSource, PodValidationError, RlimitKind, Rootfs, Seccomp,
    SeccompAction, SeccompArch, SeccompSyscallRule, Signal, SocketDirection, StatCategory, Stdio,
    UnixSocketConfig, User,
};
use firkin_ext4::Writer;
use firkin_oci::{Client, ImageConfig, Reference};
use firkin_types::VirtiofsTag;
use firkin_types::{BlockDeviceId, ContainerId, Hostname, Platform, Size};
use firkin_vmm::{
    BlankDiskImage, BootLog, DiskImageFormat, KernelImage, Network, Running, VirtualMachine,
    VmConfig, create_blank_disk_image,
};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static LIVE_COPY_PORT: AtomicU32 = AtomicU32::new(0x1000_3000);

fn pod_guest_path_rootfs(name: &str) -> PodRootfsSource {
    let path = format!("/run/firkin/pod-store/pods/p/rootfs/{name}");
    PodRootfsSource::guest_path(firkin_core::GuestPath::new(path).expect("guest path"))
}

#[test]
fn implicit_builder_accepts_full_rootfs_variants() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/echo", "hello"])
        .memory(Size::mib(256))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    assert_eq!(builder.id().as_str(), "worker");
    assert!(
        matches!(builder.rootfs(), Rootfs::Ext4Image(path) if path == std::path::Path::new("/tmp/rootfs.ext4"))
    );
}

#[test]
#[cfg(feature = "snapshot")]
fn implicit_builder_exposes_persistent_snapshot_restore_surface() {
    let staging = PathBuf::from("/tmp/firkin-worker-staging");
    let snapshot = PathBuf::from("/tmp/firkin-worker.vzstate");
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/sleep", "2147483647"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    std::mem::drop(builder.clone().spawn_with_staging_dir(&staging));
    std::mem::drop(builder.clone().restore_from_snapshot(
        &snapshot,
        &staging,
        vec![1, 2, 3],
        Vec::<String>::new(),
    ));
    let state = ContainerSnapshotState::new(&staging, vec![1, 2, 3], Vec::<String>::new());
    std::mem::drop(
        builder
            .clone()
            .restore_from_snapshot_state(&snapshot, &state),
    );
    std::mem::drop(builder.restore_from_snapshot_state_with_timings(&snapshot, &state));
}

#[test]
fn invalid_container_id_is_reported_at_builder_construction() {
    let error = Container::builder("bad/id").expect_err("invalid id");

    assert!(matches!(error, Error::InvalidContainerId(_)));
}

#[test]
fn block_device_rootfs_is_a_distinct_vm_rootfs() {
    let slot = std::num::NonZeroU32::new(7).expect("slot");
    let id = BlockDeviceId::from_slot(slot);
    let rootfs = Rootfs::block_device(id);

    assert_eq!(rootfs.as_block_device(), Some(id));
    assert_eq!(rootfs.as_guest_path(), None);
}

#[test]
fn guest_path_rootfs_is_a_distinct_vm_rootfs() {
    let path = firkin_core::GuestPath::new("/run/firkin/pods/p/rootfs/c").expect("guest path");
    let rootfs = firkin_core::VmRootfs::guest_path(path.clone());

    assert_eq!(rootfs.as_block_device(), None);
    assert_eq!(rootfs.as_guest_path(), Some(&path));
}

#[test]
fn guest_path_rejects_unsafe_paths() {
    for path in [
        "relative/rootfs",
        "/",
        "/run/firkin/../rootfs",
        "/bad\0path",
    ] {
        assert!(
            firkin_core::GuestPath::new(path).is_err(),
            "{path:?} should be rejected"
        );
    }
}

#[test]
fn pod_store_spec_defaults_to_ext4_firkin_store_path() {
    let slot = std::num::NonZeroU32::new(3).expect("slot");
    let id = BlockDeviceId::from_slot(slot);
    let spec = firkin_core::PodStoreSpec::ext4(id);

    assert_eq!(spec.block_device(), id);
    assert_eq!(spec.guest_mount().as_str(), "/run/firkin/pod-store");
    assert_eq!(spec.filesystem(), firkin_core::GuestFilesystem::Ext4);
}

#[test]
fn empty_dir_volume_validates_pod_volume_names() {
    let volume = EmptyDirVolume::disk("work").expect("disk emptyDir");

    assert_eq!(volume.name(), "work");
    assert_eq!(volume.medium(), EmptyDirMedium::Disk);
    assert_eq!(volume.size_limit(), None);
    assert!(EmptyDirVolume::disk("bad/name").is_err());
}

#[test]
fn pod_container_spec_records_rootfs_and_emptydir_mounts() {
    let rootfs = PodRootfsSource::guest_path(
        firkin_core::GuestPath::new("/run/firkin/pods/p/rootfs/app").expect("guest path"),
    );
    let spec = PodContainerSpec::new("app", rootfs.clone())
        .expect("pod container spec")
        .command(["/bin/sh", "-c", "cat /work/marker"])
        .env("ROLE", "agent")
        .empty_dir_mount("work", "/work")
        .expect("emptyDir mount")
        .empty_dir_mount_read_only("cache", "/cache")
        .expect("read-only emptyDir mount")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    assert_eq!(spec.id().as_str(), "app");
    assert_eq!(spec.rootfs(), &rootfs);
    assert_eq!(spec.command_args().len(), 3);
    assert_eq!(spec.env_vars()[0].0, "ROLE");
    assert_eq!(spec.env_vars()[0].1, "agent");
    assert_eq!(spec.empty_dir_mounts()[0].volume_name(), "work");
    assert_eq!(
        spec.empty_dir_mounts()[0].container_path(),
        std::path::Path::new("/work")
    );
    assert_eq!(spec.empty_dir_mounts()[1].volume_name(), "cache");
    assert!(spec.empty_dir_mounts()[1].read_only());
}

#[test]
fn pod_builder_records_initial_containers_and_volumes() {
    let slot = std::num::NonZeroU32::new(3).expect("slot");
    let id = BlockDeviceId::from_slot(slot);
    let pod_store = firkin_core::PodStoreSpec::ext4(id);
    let rootfs = PodRootfsSource::guest_path(
        firkin_core::GuestPath::new("/run/firkin/pods/p/rootfs/app").expect("guest path"),
    );
    let container = PodContainerSpec::new("app", rootfs).expect("container spec");
    let builder = PodBuilder::new("pod-a", VmConfig::default(), pod_store.clone())
        .expect("pod builder")
        .empty_dir(EmptyDirVolume::disk("work").expect("emptyDir"))
        .container(container);

    assert_eq!(builder.id().as_str(), "pod-a");
    assert_eq!(builder.pod_store(), &pod_store);
    assert_eq!(builder.empty_dirs().len(), 1);
    assert_eq!(builder.containers().len(), 1);
}

#[test]
fn pod_builder_validation_rejects_duplicate_container_ids() {
    let builder = PodBuilder::new(
        "pod-a",
        VmConfig::default(),
        firkin_core::PodStoreSpec::ext4(BlockDeviceId::from_slot(
            NonZeroU32::new(3).expect("slot"),
        )),
    )
    .expect("pod builder")
    .container(
        PodContainerSpec::new("agent", pod_guest_path_rootfs("agent-a")).expect("first container"),
    )
    .container(
        PodContainerSpec::new("agent", pod_guest_path_rootfs("agent-b")).expect("second container"),
    );

    let error = builder
        .validate()
        .expect_err("duplicate container rejected");

    assert!(matches!(
        error,
        Error::PodValidation(PodValidationError::DuplicateContainerName(name))
            if name.as_str() == "agent"
    ));
}

#[test]
fn pod_builder_validation_rejects_duplicate_empty_dirs() {
    let builder = PodBuilder::new(
        "pod-a",
        VmConfig::default(),
        firkin_core::PodStoreSpec::ext4(BlockDeviceId::from_slot(
            NonZeroU32::new(3).expect("slot"),
        )),
    )
    .expect("pod builder")
    .empty_dir(EmptyDirVolume::disk("work").expect("first emptyDir"))
    .empty_dir(EmptyDirVolume::disk("work").expect("second emptyDir"));

    let error = builder.validate().expect_err("duplicate emptyDir rejected");

    assert!(matches!(
        error,
        Error::PodValidation(PodValidationError::DuplicateEmptyDirName(name))
            if name.as_str() == "work"
    ));
}

#[test]
fn pod_builder_validation_rejects_unknown_empty_dir_mounts() {
    let builder = PodBuilder::new(
        "pod-a",
        VmConfig::default(),
        firkin_core::PodStoreSpec::ext4(BlockDeviceId::from_slot(
            NonZeroU32::new(3).expect("slot"),
        )),
    )
    .expect("pod builder")
    .container(
        PodContainerSpec::new("agent", pod_guest_path_rootfs("agent"))
            .expect("container")
            .empty_dir_mount("missing", "/work")
            .expect("volume mount"),
    );

    let error = builder.validate().expect_err("unknown emptyDir rejected");

    assert!(matches!(
        error,
        Error::PodValidation(PodValidationError::UnknownEmptyDirMount { container, volume_name })
            if container.as_str() == "agent" && volume_name.as_str() == "missing"
    ));
}

#[test]
fn image_config_populates_runtime_process_defaults() {
    let image_config = ImageConfig {
        user: Some("1000:1001".to_owned()),
        env: Some(vec![
            "PATH=/image/bin".to_owned(),
            "RUST_LOG=debug".to_owned(),
        ]),
        entrypoint: Some(vec!["/bin/sh".to_owned(), "-c".to_owned()]),
        cmd: Some(vec!["echo hi".to_owned()]),
        working_dir: Some("/work".to_owned()),
        ..ImageConfig::default()
    };
    let builder = Container::builder("worker")
        .expect("builder")
        .image_config(&image_config)
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");
    let process = spec.process.as_ref().expect("process");

    assert_eq!(process.args, vec!["/bin/sh", "-c", "echo hi"]);
    assert_eq!(
        process.env,
        vec!["PATH=/image/bin".to_owned(), "RUST_LOG=debug".to_owned()]
    );
    assert_eq!(process.cwd, "/work");
    assert_eq!(process.user.uid, 1000);
    assert_eq!(process.user.gid, 1001);
}

#[test]
fn builder_runtime_spec_preserves_public_user_variants() {
    let numeric = Container::builder("worker")
        .expect("builder")
        .user(User::numeric(1000, 1001).with_extra_groups(vec![10, 11]))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .runtime_spec()
        .expect("runtime spec")
        .process
        .expect("process")
        .user;

    assert_eq!(numeric.uid, 1000);
    assert_eq!(numeric.gid, 1001);
    assert_eq!(numeric.additional_gids, vec![10, 11]);
    assert_eq!(numeric.username, "");

    let named = Container::builder("worker")
        .expect("builder")
        .user(User::named("daemon"))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .runtime_spec()
        .expect("runtime spec")
        .process
        .expect("process")
        .user;

    assert_eq!(named.uid, 0);
    assert_eq!(named.gid, 0);
    assert!(named.additional_gids.is_empty());
    assert_eq!(named.username, "daemon");
}

#[test]
fn explicit_command_after_image_config_overrides_image_command() {
    let image_config = ImageConfig {
        entrypoint: Some(vec!["/from-image".to_owned()]),
        cmd: Some(vec!["arg".to_owned()]),
        ..ImageConfig::default()
    };
    let builder = Container::builder("worker")
        .expect("builder")
        .image_config(&image_config)
        .command(["/explicit"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");

    assert_eq!(spec.process.as_ref().unwrap().args, vec!["/explicit"]);
}

#[test]
fn ready_builder_generates_default_runtime_spec_for_vminitd() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/echo", "hello"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");

    assert_eq!(spec.version, "1.1.0");
    assert_eq!(spec.hostname, "worker");
    assert_eq!(
        spec.root.as_ref().unwrap().path,
        "/run/container/worker/rootfs"
    );
    assert!(!spec.root.as_ref().unwrap().readonly);
    assert_eq!(
        spec.mounts.iter().map(mount_shape).collect::<Vec<_>>(),
        vec![
            ("proc", "proc", "/proc"),
            ("sysfs", "sysfs", "/sys"),
            ("devtmpfs", "none", "/dev"),
            ("mqueue", "mqueue", "/dev/mqueue"),
            ("tmpfs", "tmpfs", "/dev/shm"),
            ("devpts", "devpts", "/dev/pts"),
            ("cgroup2", "none", "/sys/fs/cgroup"),
        ]
    );
    assert_eq!(spec.mounts[0].options, Vec::<String>::new());
    assert_eq!(spec.mounts[1].options, ["nosuid", "noexec", "nodev"]);
    assert_eq!(spec.mounts[2].options, ["nosuid", "mode=755"]);
    assert_eq!(spec.mounts[3].options, ["nosuid", "noexec", "nodev"]);
    assert_eq!(
        spec.mounts[4].options,
        ["nosuid", "noexec", "nodev", "mode=1777", "size=65536k"]
    );
    assert_eq!(
        spec.mounts[5].options,
        [
            "nosuid",
            "noexec",
            "newinstance",
            "gid=5",
            "mode=0620",
            "ptmxmode=0666"
        ]
    );
    assert_eq!(spec.mounts[6].options, ["nosuid", "noexec", "nodev"]);
    assert_eq!(
        spec.process.as_ref().unwrap().args,
        vec!["/bin/echo".to_owned(), "hello".to_owned()]
    );
    assert!(spec.process.as_ref().unwrap().capabilities.is_some());
    assert!(
        spec.process
            .as_ref()
            .unwrap()
            .capabilities
            .as_ref()
            .unwrap()
            .effective
            .as_ref()
            .unwrap()
            .contains(&"CAP_CHOWN".to_owned())
    );
    assert_eq!(
        spec.linux.as_ref().unwrap().cgroups_path,
        "/container/worker"
    );
    assert_eq!(
        spec.linux
            .as_ref()
            .unwrap()
            .resources
            .as_ref()
            .unwrap()
            .devices,
        Vec::new()
    );
}

#[test]
fn use_init_wraps_process_and_binds_vminitd() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/sh", "-c", "echo hello"])
        .use_init(true)
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");
    let init_mount = spec
        .mounts
        .iter()
        .find(|mount| mount.destination == "/.cz-init")
        .expect("init bind mount");

    assert_eq!(
        spec.process.as_ref().unwrap().args,
        vec!["/.cz-init", "--", "/bin/sh", "-c", "echo hello"]
    );
    assert_eq!(init_mount.kind, "bind");
    assert_eq!(init_mount.source, "/sbin/vminitd");
    assert_eq!(init_mount.options, ["bind", "ro"]);
}

#[test]
fn builder_runtime_spec_preserves_structured_seccomp_policy() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/true"])
        .seccomp(Seccomp {
            default_action: SeccompAction::Errno,
            default_errno_ret: Some(38),
            architectures: vec![SeccompArch::Aarch64],
            flags: Vec::new(),
            listener_path: String::new(),
            listener_metadata: String::new(),
            syscalls: vec![SeccompSyscallRule {
                names: vec!["read".to_owned(), "write".to_owned()],
                action: SeccompAction::Allow,
                errno_ret: None,
                args: Vec::new(),
            }],
        })
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");
    let seccomp = spec.linux.unwrap().seccomp.expect("seccomp");

    assert_eq!(
        serde_json::to_value(seccomp).unwrap(),
        serde_json::json!({
            "defaultAction": "SCMP_ACT_ERRNO",
            "defaultErrnoRet": 38,
            "architectures": ["SCMP_ARCH_AARCH64"],
            "flags": [],
            "listenerPath": "",
            "listenerMetadata": "",
            "syscalls": [{
                "names": ["read", "write"],
                "action": "SCMP_ACT_ALLOW",
                "args": []
            }]
        })
    );
}

#[test]
fn builder_runtime_spec_preserves_raw_seccomp_json() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/true"])
        .seccomp_profile_json(
            r#"{"defaultAction":"SCMP_ACT_ALLOW","architectures":["SCMP_ARCH_AARCH64"],"syscalls":[]}"#,
        )
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");
    let seccomp = spec.linux.unwrap().seccomp.expect("seccomp");

    assert_eq!(
        serde_json::to_value(seccomp).unwrap(),
        serde_json::json!({
            "defaultAction": "SCMP_ACT_ALLOW",
            "architectures": ["SCMP_ARCH_AARCH64"],
            "syscalls": []
        })
    );
}

#[test]
fn builder_runtime_spec_reports_invalid_raw_seccomp_json() {
    let error = Container::builder("worker")
        .expect("builder")
        .command(["/bin/true"])
        .seccomp_profile_json("{not json")
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .runtime_spec()
        .expect_err("invalid raw seccomp json");

    assert!(matches!(error, Error::InvalidSeccomp { .. }));
}

fn mount_shape(mount: &Mount) -> (&str, &str, &str) {
    (
        mount.kind.as_str(),
        mount.source.as_str(),
        mount.destination.as_str(),
    )
}

#[test]
fn builder_runtime_spec_preserves_and_sorts_custom_mounts() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/true"])
        .default_mounts(vec![Mount::proc("/proc")])
        .mount(Mount::tmpfs("/var/log/../cache").extra_option("mode=0700"))
        .mount(Mount::bind("/host", "/var").read_only())
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");

    assert_eq!(
        spec.mounts
            .iter()
            .map(|mount| mount.destination.as_str())
            .collect::<Vec<_>>(),
        vec!["/proc", "/var", "/var/cache"]
    );
    assert_eq!(spec.mounts[1].kind, "bind");
    assert_eq!(spec.mounts[1].options, ["bind", "rw", "rbind", "ro"]);
    assert_eq!(spec.mounts[2].kind, "tmpfs");
    assert_eq!(spec.mounts[2].options, ["mode=0700"]);
}

#[test]
fn builder_runtime_spec_binds_into_guest_socket_staging_path() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/true"])
        .default_mounts(Vec::new())
        .socket(UnixSocketConfig::into_guest(
            "api",
            "/tmp/host.sock",
            "/run/app/api.sock",
        ))
        .socket(UnixSocketConfig::new(
            "agent",
            "/run/app/agent.sock",
            "/tmp/agent.sock",
            SocketDirection::OutOf,
        ))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");

    assert_eq!(spec.mounts.len(), 1);
    assert_eq!(
        mount_shape(&spec.mounts[0]),
        (
            "bind",
            "/run/container/worker/sockets/api.sock",
            "/run/app/api.sock"
        )
    );
    assert_eq!(spec.mounts[0].options, ["bind"]);
}

#[test]
fn builder_runtime_spec_binds_single_file_mounts_from_holding_share() {
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("config.json");
    std::fs::write(&source, b"{}").unwrap();
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/true"])
        .default_mounts(Vec::new())
        .file_mount(FileMount::read_only(&source, "/etc/app/config.json"))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");

    assert_eq!(spec.mounts.len(), 1);
    assert_eq!(spec.mounts[0].kind, "none");
    assert!(spec.mounts[0].source.starts_with("/run/file-mounts/"));
    assert!(spec.mounts[0].source.ends_with("/config.json"));
    assert_eq!(spec.mounts[0].destination, "/etc/app/config.json");
    assert_eq!(spec.mounts[0].options, ["bind", "ro"]);
}

#[test]
fn builder_runtime_spec_accepts_block_writable_layer() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/true"])
        .writable_layer(Mount::block("ext4", "/tmp/upper.ext4", "/"))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");

    assert_eq!(
        spec.root.as_ref().unwrap().path,
        "/run/container/worker/rootfs"
    );
    assert!(
        !spec
            .mounts
            .iter()
            .any(|mount| mount.source == "/tmp/upper.ext4")
    );
}

#[test]
fn builder_runtime_spec_rejects_non_block_writable_layer() {
    let error = Container::builder("worker")
        .expect("builder")
        .writable_layer(Mount::tmpfs("/"))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"))
        .runtime_spec()
        .expect_err("non-block writable layer");

    assert_eq!(
        error,
        Error::InvalidWritableLayer {
            kind: "tmpfs".to_owned()
        }
    );
}

#[test]
fn builder_runtime_spec_preserves_process_policy_knobs() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/echo", "hello"])
        .hostname(Hostname::new("job-host").expect("hostname"))
        .sysctl("net.ipv4.ip_forward", "1")
        .no_new_privileges(true)
        .capabilities(LinuxCapabilities::single_set(vec![Capability::Chown]))
        .rlimit(LinuxRlimit::new(RlimitKind::OpenFiles, 2048, 1024))
        .selinux_label("system_u:system_r:container_t:s0")
        .apparmor_profile("firkin-profile")
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");
    let process = spec.process.as_ref().expect("process");
    let linux = spec.linux.as_ref().expect("linux");

    assert_eq!(spec.hostname, "job-host");
    assert_eq!(
        linux
            .sysctl
            .as_ref()
            .expect("sysctl")
            .get("net.ipv4.ip_forward"),
        Some(&"1".to_owned())
    );
    assert!(process.no_new_privileges);
    assert_eq!(
        process
            .capabilities
            .as_ref()
            .expect("capabilities")
            .effective,
        Some(vec!["CAP_CHOWN".to_owned()])
    );
    assert_eq!(process.rlimits[0].kind, "RLIMIT_NOFILE");
    assert_eq!(process.rlimits[0].soft, 1024);
    assert_eq!(process.rlimits[0].hard, 2048);
    assert_eq!(process.selinux_label, "system_u:system_r:container_t:s0");
    assert_eq!(process.apparmor_profile, "firkin-profile");
}

#[test]
fn process_policy_value_types_parse_domain_names() {
    assert_eq!(
        Capability::parse("CAP_NET_RAW").unwrap(),
        Capability::NetRaw
    );
    assert_eq!(
        Capability::parse("checkpoint_restore").unwrap(),
        Capability::CheckpointRestore
    );
    assert!(Capability::parse("CAP_NOT_REAL").is_err());
    assert_eq!(
        RlimitKind::parse("RLIMIT_NOFILE").unwrap(),
        RlimitKind::OpenFiles
    );
    assert_eq!(RlimitKind::parse("stack").unwrap(), RlimitKind::StackSize);
    assert!(RlimitKind::parse("RLIMIT_NOT_REAL").is_err());

    let empty = LinuxCapabilities::empty();
    assert!(empty.effective.is_empty());
    assert!(
        LinuxCapabilities::all()
            .effective
            .contains(&Capability::CheckpointRestore)
    );
}

#[test]
fn implicit_builder_accepts_vm_level_config_knobs() {
    let builder = Container::builder("worker")
        .expect("builder")
        .cpus(NonZeroU32::new(2).expect("cpus"))
        .memory(Size::mib(512))
        .virtiofs_share(VirtiofsTag::new("shared").expect("tag"), "/tmp")
        .nested_virtualization(true)
        .boot_log(BootLog::None)
        .kernel(KernelImage::from_file("/tmp/vmlinux"))
        .cmdline_extra("debug=1")
        .command(["/bin/echo", "hello"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    assert_eq!(builder.cpu_count().get(), 2);
    assert_eq!(builder.memory_size(), Size::mib(512));
}

#[test]
fn rosetta_builder_adds_rosetta_mount_to_runtime_spec() {
    let builder = Container::builder("rosetta")
        .expect("builder")
        .rosetta(true)
        .command(["/bin/uname", "-m"])
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().expect("runtime spec");

    assert!(spec.mounts.iter().any(|mount| {
        mount.kind == "virtiofs" && mount.source == "rosetta" && mount.destination == "/run/rosetta"
    }));
}

#[test]
fn exit_status_and_output_expose_process_outcome_shape() {
    let ok = ExitStatus::from_code(0);
    assert!(ok.success());
    assert_eq!(ok.code(), Some(0));
    assert_eq!(ok.signal(), None);
    assert_eq!(ok.killed_reason(), None);

    let killed = ExitStatus::from_signal(
        Signal::new(9),
        KilledReason::SignalFromHost {
            signal: Signal::new(9),
        },
    );
    assert!(!killed.success());
    assert_eq!(killed.code(), None);
    assert_eq!(killed.signal(), Some(Signal::new(9)));

    let output = Output {
        status: ok,
        stdout: b"hello\n".to_vec(),
        stderr: Vec::new(),
    };
    assert_eq!(output.stdout, b"hello\n");
}

#[test]
fn stdio_inherit_is_an_explicit_configuration() {
    assert_eq!(Stdio::inherit(), Stdio::Inherit);
}

#[tokio::test]
async fn spawn_terminal_enters_runtime_preparation() {
    let error = Container::builder(ContainerId::new("worker").expect("id"))
        .expect("builder")
        .rootfs(Rootfs::raw_block("/tmp/rootfs.img"))
        .spawn()
        .await
        .expect_err("missing rootfs");

    assert_eq!(error, missing_rootfs_preparation_error());
}

#[tokio::test]
async fn one_shot_terminals_enter_runtime_preparation() {
    let status_error = Container::builder("worker")
        .expect("builder")
        .rootfs(Rootfs::raw_block("/tmp/rootfs.img"))
        .status()
        .await
        .expect_err("missing rootfs");
    assert_eq!(status_error, missing_rootfs_preparation_error());

    let output_error = Container::builder("worker")
        .expect("builder")
        .rootfs(Rootfs::raw_block("/tmp/rootfs.img"))
        .output()
        .await
        .expect_err("missing rootfs");
    assert_eq!(output_error, missing_rootfs_preparation_error());
}

fn missing_rootfs_preparation_error() -> Error {
    if firkin_vminitd_bytes::embedded() {
        return Error::RuntimeArtifact {
            operation: "build VM config",
            reason: "invalid VM configuration: block_device: /tmp/rootfs.img not accessible".into(),
        };
    }

    Error::RuntimeArtifact {
        operation: "resolve init.block",
        reason: "runtime artifact preparation failed while load vminitd runtime ELFs: runtime-download builds do not embed vminitd/vmexec bytes yet".into(),
    }
}

async fn live_arm64_busybox_rootfs() -> Rootfs {
    if let Some(path) = std::env::var_os("FIRKIN_ARM64_BUSYBOX_ROOTFS") {
        return Rootfs::ext4_image(PathBuf::from(path));
    }
    live_busybox_rootfs(Platform::linux_arm64()).await
}

async fn live_amd64_busybox_rootfs() -> Rootfs {
    live_busybox_rootfs(Platform::linux_amd64()).await
}

async fn live_busybox_rootfs(platform: Platform) -> Rootfs {
    let image = Client::builder()
        .cache_dir(live_busybox_cache_dir())
        .platform(platform)
        .build()
        .expect("oci client")
        .pull(&Reference::parse("busybox").expect("reference"))
        .await
        .expect("busybox pull");
    Rootfs::oci_bundle(image)
}

fn live_busybox_cache_dir() -> PathBuf {
    std::env::var_os("FIRKIN_LIVE_BUSYBOX_CACHE")
        .map_or_else(|| firkin_live_cache_root().join("busybox"), PathBuf::from)
}

fn firkin_live_cache_root() -> PathBuf {
    std::env::var_os("FIRKIN_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".firkin").join("cache"))
        })
        .unwrap_or_else(|| PathBuf::from(".firkin").join("cache"))
        .join("live")
}

fn live_arm64_busybox_rootfs_path() -> Option<PathBuf> {
    std::env::var_os("FIRKIN_ARM64_BUSYBOX_ROOTFS").map(PathBuf::from)
}

async fn live_arm64_busybox_image_bundle() -> firkin_oci::ImageBundle {
    let image = Client::builder()
        .cache_dir(live_busybox_cache_dir())
        .platform(Platform::linux_arm64())
        .build()
        .expect("oci client")
        .pull(&Reference::parse("busybox").expect("reference"))
        .await
        .expect("busybox pull");
    assert!(
        !image.layers().is_empty(),
        "busybox image should have at least one layer"
    );
    image
}

fn write_empty_pod_store(path: impl AsRef<std::path::Path>) {
    Writer::new(path.as_ref(), Size::mib(192))
        .expect("empty pod-store writer")
        .write_dir("/run", 0o755)
        .expect("write /run")
        .write_dir("/run/firkin", 0o755)
        .expect("write /run/firkin")
        .finalize()
        .expect("finalize empty pod store");
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn live_init_block_path() -> PathBuf {
    firkin_ext4::init_block::synthesize(
        firkin_vminitd_bytes::VMINITD_AARCH64,
        firkin_vminitd_bytes::VMEXEC_AARCH64,
    )
    .expect("synthesize live init.block")
}

async fn live_vminitd_pod_store(
    name: &str,
) -> (
    VirtualMachine<Running>,
    firkin_core::MountedPodStore,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().expect("vminitd live tempdir");
    let pod_store_path = temp.path().join(format!("{name}.ext4"));
    write_empty_pod_store(&pod_store_path);

    let (vm_builder, pod_store_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(&pod_store_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot vminitd live VM");
    let mounted = firkin_core::mount_pod_store(&vm, &firkin_core::PodStoreSpec::ext4(pod_store_id))
        .await
        .expect("mount live pod store");
    (vm, mounted, temp)
}

async fn live_vminitd_client(
    vm: &VirtualMachine<Running>,
) -> firkin_vminitd_client::pb::sandbox_context_client::SandboxContextClient<
    tonic::transport::Channel,
> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let vm = vm.clone();
        match firkin_vminitd_client::connect_with_dialer(move |port| {
            let vm = vm.clone();
            async move {
                vm.dial_reserved_port(port).await.map_err(|error| {
                    firkin_vsock::Error::Io(std::io::Error::other(error.to_string()))
                })
            }
        })
        .await
        {
            Ok(client) => return client,
            Err(error) if std::time::Instant::now() < deadline => {
                eprintln!("waiting for vminitd: {error}");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(error) => panic!("connect vminitd: {error:?}"),
        }
    }
}

fn next_live_copy_port() -> firkin_types::VsockPort {
    firkin_types::VsockPort::new(LIVE_COPY_PORT.fetch_add(1, Ordering::SeqCst))
}

async fn copy_file_to_guest(
    vm: &VirtualMachine<Running>,
    client: &mut firkin_vminitd_client::pb::sandbox_context_client::SandboxContextClient<
        tonic::transport::Channel,
    >,
    host_path: &std::path::Path,
    guest_path: &str,
) {
    let port = next_live_copy_port();
    let listener = vm
        .listen_reserved_port(port)
        .expect("listen for guest copy-in");
    let request = firkin_vminitd_client::CopyTransfer::copy_in(guest_path, port)
        .create_parents(true)
        .into_request();
    let mut control = client.clone();
    let copy_task = tokio::spawn(async move {
        let mut stream = control
            .copy(tonic::Request::new(request))
            .await
            .expect("copy-in control")
            .into_inner();
        while let Some(response) = stream.message().await.expect("copy-in response") {
            let event =
                firkin_vminitd_client::CopyResponseEvent::try_from(response).expect("copy event");
            if matches!(event, firkin_vminitd_client::CopyResponseEvent::Complete) {
                return;
            }
        }
        panic!("copy-in stream ended without completion");
    });

    let (mut stream, _) = listener.accept().await.expect("accept copy-in stream");
    let mut file = tokio::fs::File::open(host_path)
        .await
        .expect("open host copy-in file");
    tokio::io::copy(&mut file, &mut stream)
        .await
        .expect("stream host file into guest");
    stream.shutdown().await.expect("finish copy-in stream");
    copy_task.await.expect("copy-in task");
}

async fn copy_file_from_guest(
    vm: &VirtualMachine<Running>,
    client: &mut firkin_vminitd_client::pb::sandbox_context_client::SandboxContextClient<
        tonic::transport::Channel,
    >,
    guest_path: &str,
) -> Result<Vec<u8>, tonic::Status> {
    let port = next_live_copy_port();
    let listener = vm
        .listen_reserved_port(port)
        .expect("listen for guest copy-out");
    let request = firkin_vminitd_client::CopyTransfer::copy_out(guest_path, port).into_request();
    let mut control = client.clone();
    let mut copy_task = tokio::spawn(async move {
        let mut stream = control
            .copy(tonic::Request::new(request))
            .await?
            .into_inner();
        while let Some(response) = stream.message().await? {
            let event = firkin_vminitd_client::CopyResponseEvent::try_from(response)
                .map_err(|error| tonic::Status::internal(error.to_string()))?;
            if matches!(event, firkin_vminitd_client::CopyResponseEvent::Complete) {
                return Ok::<(), tonic::Status>(());
            }
        }
        Err(tonic::Status::internal(
            "copy-out stream ended without completion",
        ))
    });

    let (mut stream, _) = tokio::select! {
        accepted = listener.accept() => accepted.expect("accept copy-out stream"),
        result = &mut copy_task => {
            result.expect("copy-out task")?;
            return Ok(Vec::new());
        }
    };
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("read guest copy-out stream");
    copy_task.await.expect("copy-out task")?;
    Ok(bytes)
}

fn write_tar_archive(path: &std::path::Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).expect("create tar archive");
    let mut builder = tar::Builder::new(file);
    for (name, content) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(u64::try_from(content.len()).expect("content length"));
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, *content)
            .expect("append tar entry");
    }
    builder.finish().expect("finish tar archive");
}

fn write_tar_archive_with_symlink_parent_escape(
    path: &std::path::Path,
    symlink_target: &str,
    escaped_path: &str,
) {
    let file = std::fs::File::create(path).expect("create tar archive");
    let mut builder = tar::Builder::new(file);
    let mut symlink = tar::Header::new_gnu();
    symlink.set_entry_type(tar::EntryType::Symlink);
    symlink.set_size(0);
    symlink.set_mode(0o777);
    symlink
        .set_link_name(symlink_target)
        .expect("set symlink target");
    symlink.set_cksum();
    builder
        .append_data(&mut symlink, "escape", std::io::empty())
        .expect("append symlink entry");

    let mut file_header = tar::Header::new_gnu();
    file_header.set_entry_type(tar::EntryType::Regular);
    file_header.set_size(7);
    file_header.set_mode(0o644);
    file_header.set_cksum();
    builder
        .append_data(&mut file_header, escaped_path, &b"escaped"[..])
        .expect("append escaped regular entry");
    builder.finish().expect("finish tar archive");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live vminitd RemovePath smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_vminitd_remove_path_removes_guest_directory() {
    let (vm, mounted, _temp) = live_vminitd_pod_store("remove-path").await;
    let mut client = live_vminitd_client(&vm).await;
    let target = format!("{}/remove-me/nested", mounted.guest_mount().as_str());
    client
        .mkdir(tonic::Request::new(
            firkin_vminitd_client::pb::MkdirRequest {
                path: target.clone(),
                all: true,
                perms: 0o755,
            },
        ))
        .await
        .expect("mkdir remove target");
    client
        .remove_path(tonic::Request::new(
            firkin_vminitd_client::RemovePath::recursive(format!(
                "{}/remove-me",
                mounted.guest_mount().as_str()
            ))
            .into_request(),
        ))
        .await
        .expect("remove path");
    let missing = client
        .remove_path(tonic::Request::new(
            firkin_vminitd_client::RemovePath::new(format!(
                "{}/remove-me",
                mounted.guest_mount().as_str()
            ))
            .into_request(),
        ))
        .await
        .expect_err("removed path is missing");
    vm.stop().await.expect("stop remove-path VM");

    assert_eq!(missing.code(), tonic::Code::NotFound);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live vminitd Fstrim smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_vminitd_fstrim_accepts_pod_store_mount() {
    let (vm, mounted, _temp) = live_vminitd_pod_store("fstrim").await;
    let mut client = live_vminitd_client(&vm).await;
    let _response = client
        .fstrim(tonic::Request::new(
            firkin_vminitd_client::Fstrim::new(mounted.guest_mount().as_str()).into_request(),
        ))
        .await
        .expect("fstrim pod store")
        .into_inner();
    let usage = client
        .filesystem_usage(tonic::Request::new(
            firkin_vminitd_client::FilesystemUsage::new(mounted.guest_mount().as_str())
                .into_request(),
        ))
        .await
        .expect("filesystem usage after fstrim")
        .into_inner();
    vm.stop().await.expect("stop fstrim VM");

    assert!(usage.total_blocks > 0);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live vminitd FilesystemUsage smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_vminitd_filesystem_usage_reports_pod_store() {
    let (vm, mounted, _temp) = live_vminitd_pod_store("filesystem-usage").await;
    let mut client = live_vminitd_client(&vm).await;
    let usage = client
        .filesystem_usage(tonic::Request::new(
            firkin_vminitd_client::FilesystemUsage::new(mounted.guest_mount().as_str())
                .into_request(),
        ))
        .await
        .expect("filesystem usage")
        .into_inner();
    vm.stop().await.expect("stop filesystem-usage VM");

    assert!(usage.block_size > 0);
    assert!(usage.total_blocks > 0);
    assert!(usage.available_blocks <= usage.total_blocks);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live vminitd ApplyOciLayer smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_vminitd_apply_oci_layer_applies_whiteouts() {
    let (vm, mounted, temp) = live_vminitd_pod_store("apply-oci-layer").await;
    let base_archive = temp.path().join("base.tar");
    let update_archive = temp.path().join("update.tar");
    write_tar_archive(
        &base_archive,
        &[
            ("kept", b"kept"),
            ("removed", b"removed"),
            ("dir/old", b"old"),
        ],
    );
    write_tar_archive(
        &update_archive,
        &[
            (".wh.removed", b""),
            ("dir/.wh..wh..opq", b""),
            ("dir/new", b"new"),
        ],
    );

    let mut client = live_vminitd_client(&vm).await;
    let base_guest = "/run/firkin/layers/base.tar";
    let update_guest = "/run/firkin/layers/update.tar";
    let destination = format!(
        "{}/pods/p/templates/t/rootfs",
        mounted.guest_mount().as_str()
    );
    client
        .mkdir(tonic::Request::new(
            firkin_vminitd_client::pb::MkdirRequest {
                path: destination.clone(),
                all: true,
                perms: 0o755,
            },
        ))
        .await
        .expect("mkdir layer destination");
    copy_file_to_guest(&vm, &mut client, &base_archive, base_guest).await;
    copy_file_to_guest(&vm, &mut client, &update_archive, update_guest).await;
    client
        .apply_oci_layer(tonic::Request::new(
            firkin_vminitd_client::ApplyOciLayer::new(base_guest, &destination).into_request(),
        ))
        .await
        .expect("apply base layer");
    client
        .apply_oci_layer(tonic::Request::new(
            firkin_vminitd_client::ApplyOciLayer::new(update_guest, &destination).into_request(),
        ))
        .await
        .expect("apply update layer");

    let kept = copy_file_from_guest(&vm, &mut client, &format!("{destination}/kept"))
        .await
        .expect("copy kept");
    let new = copy_file_from_guest(&vm, &mut client, &format!("{destination}/dir/new"))
        .await
        .expect("copy new");
    let removed = copy_file_from_guest(&vm, &mut client, &format!("{destination}/removed")).await;
    let old = copy_file_from_guest(&vm, &mut client, &format!("{destination}/dir/old")).await;
    vm.stop().await.expect("stop apply-oci-layer VM");

    assert_eq!(kept, b"kept");
    assert_eq!(new, b"new");
    assert!(removed.is_err());
    assert!(old.is_err());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live vminitd ApplyOciLayer symlink-parent smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_vminitd_apply_oci_layer_rejects_symlink_parent_escape() {
    let (vm, mounted, temp) = live_vminitd_pod_store("apply-oci-symlink-parent").await;
    let archive = temp.path().join("malicious.tar");
    let destination = format!(
        "{}/pods/p/templates/t/rootfs",
        mounted.guest_mount().as_str()
    );
    let escaped_marker = format!("{}/escaped-marker", mounted.guest_mount().as_str());
    write_tar_archive_with_symlink_parent_escape(
        &archive,
        mounted.guest_mount().as_str(),
        "escape/escaped-marker",
    );

    let mut client = live_vminitd_client(&vm).await;
    let archive_guest = "/run/firkin/layers/malicious.tar";
    client
        .mkdir(tonic::Request::new(
            firkin_vminitd_client::pb::MkdirRequest {
                path: destination.clone(),
                all: true,
                perms: 0o755,
            },
        ))
        .await
        .expect("mkdir layer destination");
    copy_file_to_guest(&vm, &mut client, &archive, archive_guest).await;
    let apply = client
        .apply_oci_layer(tonic::Request::new(
            firkin_vminitd_client::ApplyOciLayer::new(archive_guest, &destination).into_request(),
        ))
        .await;
    let escaped = copy_file_from_guest(&vm, &mut client, &escaped_marker).await;
    vm.stop().await.expect("stop apply-oci-symlink-parent VM");

    assert!(
        apply.is_err(),
        "symlink parent traversal should be rejected"
    );
    assert!(escaped.is_err(), "escaped marker must not be written");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_output_runs_in_implicit_vm() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let output = Container::builder("live-busybox")
        .expect("builder")
        .command(["/bin/echo", "hello from firkin"])
        .rootfs(rootfs)
        .output()
        .await
        .expect("container output");

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"hello from firkin\n");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM explicit-container smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_runs_on_existing_vm() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping explicit-VM live smoke");
        return;
    };
    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(rootfs_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot VM");

    let output = vm
        .container("live-on-vm")
        .expect("container builder")
        .command(["/bin/echo", "existing vm from firkin"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn on existing vm")
        .wait_with_output()
        .await
        .expect("container output");
    let _ = vm.clone().stop().await;

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"existing vm from firkin\n");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM multi-container substrate smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_two_busybox_containers_run_on_one_existing_vm() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping multi-container live smoke");
        return;
    };
    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(rootfs_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot VM");

    let first = vm
        .container("live-on-vm-a")
        .expect("first container builder")
        .command(["/bin/echo", "first existing vm container"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn first container on existing vm")
        .wait_with_output()
        .await
        .expect("first container output");
    let second = vm
        .container("live-on-vm-b")
        .expect("second container builder")
        .command(["/bin/echo", "second existing vm container"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn second container on existing vm")
        .wait_with_output()
        .await
        .expect("second container output");
    let _ = vm.clone().stop().await;

    assert!(first.status.success(), "first status={:?}", first.status);
    assert!(second.status.success(), "second status={:?}", second.status);
    assert_eq!(first.stdout, b"first existing vm container\n");
    assert_eq!(second.stdout, b"second existing vm container\n");
    assert!(first.stderr.is_empty());
    assert!(second.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM ASIF storage smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_asif_local_disk_image_is_guest_visible_and_writable() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping ASIF live smoke");
        return;
    };
    let temp = tempfile::tempdir().expect("ASIF smoke tempdir");
    let asif_path = temp.path().join("pod-store.asif");
    create_blank_disk_image(&BlankDiskImage::new(
        &asif_path,
        Size::mib(64),
        DiskImageFormat::Asif,
    ))
    .expect("create blank ASIF image");

    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(rootfs_path);
    let (vm_builder, _asif_id) = vm_builder.asif_disk_image(&asif_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot VM with ASIF data disk");

    let output_result = vm
        .container("live-asif-smoke")
        .expect("container builder")
        .command([
            "/bin/sh",
            "-c",
            "test -b /dev/vdc && printf firkin-asif-smoke | /bin/busybox dd of=/dev/vdc bs=17 count=1 conv=notrunc 2>/tmp/asif-write.err && /bin/busybox dd if=/dev/vdc bs=17 count=1 2>/tmp/asif-read.err",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn ASIF smoke container")
        .wait_with_output()
        .await;
    let stop_result = vm.clone().stop().await;
    let output = output_result.expect("ASIF smoke output");
    stop_result.expect("stop ASIF smoke VM");

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"firkin-asif-smoke");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM pod-store smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_pod_store_ext4_mounts_and_persists_guest_file() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping pod-store live smoke");
        return;
    };
    let temp = tempfile::tempdir().expect("pod-store tempdir");
    let pod_store_path = temp.path().join("pod-store.ext4");
    std::fs::copy(&rootfs_path, &pod_store_path).expect("copy pod-store ext4 image");

    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(&rootfs_path);
    let (vm_builder, pod_store_id) = vm_builder.block_device(&pod_store_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot pod-store VM");
    let mounted = firkin_core::mount_pod_store(&vm, &firkin_core::PodStoreSpec::ext4(pod_store_id))
        .await
        .expect("mount pod store");
    assert_eq!(mounted.guest_mount().as_str(), "/run/firkin/pod-store");

    let writer = vm
        .container("pod-store-writer")
        .expect("writer builder")
        .command([
            "/bin/sh",
            "-c",
            "printf pod-store-ok >/store/marker && /bin/busybox sync && cat /store/marker",
        ])
        .mount(Mount::bind("/run/firkin/pod-store", "/store"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn pod-store writer")
        .wait_with_output()
        .await
        .expect("writer output");
    vm.stop().await.expect("stop first pod-store VM");
    assert!(
        writer.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        writer.status,
        String::from_utf8_lossy(&writer.stdout),
        String::from_utf8_lossy(&writer.stderr)
    );
    assert_eq!(writer.stdout, b"pod-store-ok");

    let (vm_builder, restored_rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(rootfs_path);
    let (vm_builder, restored_store_id) = vm_builder.block_device(&pod_store_path);
    let restored = VirtualMachine::new(vm_builder.build().expect("restore vm config"))
        .boot()
        .await
        .expect("boot restored pod-store VM");
    firkin_core::mount_pod_store(
        &restored,
        &firkin_core::PodStoreSpec::ext4(restored_store_id),
    )
    .await
    .expect("mount restored pod store");
    let reader = restored
        .container("pod-store-reader")
        .expect("reader builder")
        .command(["/bin/cat", "/store/marker"])
        .mount(Mount::bind("/run/firkin/pod-store", "/store"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(restored_rootfs_id))
        .spawn()
        .await
        .expect("spawn pod-store reader")
        .wait_with_output()
        .await
        .expect("reader output");
    restored.stop().await.expect("stop restored pod-store VM");

    assert!(
        reader.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        reader.status,
        String::from_utf8_lossy(&reader.stdout),
        String::from_utf8_lossy(&reader.stderr)
    );
    assert_eq!(reader.stdout, b"pod-store-ok");
    assert!(reader.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM guest-path rootfs smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_on_vm_container_can_start_from_guest_path_rootfs() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping guest-path rootfs smoke");
        return;
    };
    let temp = tempfile::tempdir().expect("guest-path tempdir");
    let pod_store_path = temp.path().join("guest-path-rootfs.ext4");
    std::fs::copy(&rootfs_path, &pod_store_path).expect("copy guest-path ext4 image");

    let (vm_builder, pod_store_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(&pod_store_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot guest-path VM");
    let mounted = firkin_core::mount_pod_store(&vm, &firkin_core::PodStoreSpec::ext4(pod_store_id))
        .await
        .expect("mount guest-path rootfs store");
    let rootfs = firkin_core::VmRootfs::guest_path(mounted.guest_mount().clone());

    let output = vm
        .container("guest-path-rootfs")
        .expect("guest-path builder")
        .command(["/bin/echo", "guest-path-rootfs"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("spawn guest-path container")
        .wait_with_output()
        .await
        .expect("guest-path output");
    vm.stop().await.expect("stop guest-path VM");

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"guest-path-rootfs\n");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live pod-store rootfs materialization smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_pod_store_materializes_busybox_rootfs_and_starts_container() {
    let image = live_arm64_busybox_image_bundle().await;
    let temp = tempfile::tempdir().expect("materialized-rootfs tempdir");
    let pod_store_path = temp.path().join("materialized-pod-store.ext4");
    write_empty_pod_store(&pod_store_path);

    let (vm_builder, pod_store_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(&pod_store_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot materialized-rootfs VM");
    let mounted = firkin_core::mount_pod_store(&vm, &firkin_core::PodStoreSpec::ext4(pod_store_id))
        .await
        .expect("mount pod store");
    let container_id = ContainerId::new("materialized").expect("container id");
    let rootfs = firkin_core::materialize_rootfs_in_pod_store(
        &vm,
        &mounted,
        &container_id,
        &PodRootfsSource::oci_bundle(image),
    )
    .await
    .expect("materialize rootfs");

    let output = vm
        .container("materialized")
        .expect("container builder")
        .command([
            "/bin/sh",
            "-c",
            "test -x /bin/busybox && /bin/echo materialized-rootfs",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(firkin_core::VmRootfs::guest_path(rootfs.path().clone()))
        .spawn()
        .await
        .expect("spawn materialized-rootfs container")
        .wait_with_output()
        .await
        .expect("materialized-rootfs output");
    vm.stop().await.expect("stop materialized-rootfs VM");

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"materialized-rootfs\n");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live pod emptyDir smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_pod_two_busybox_containers_share_emptydir() {
    let image = live_arm64_busybox_image_bundle().await;
    let temp = tempfile::tempdir().expect("pod emptyDir tempdir");
    let pod_store_path = temp.path().join("pod-emptydir-store.ext4");
    write_empty_pod_store(&pod_store_path);

    let (vm_builder, pod_store_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(&pod_store_path);
    let pod_store = firkin_core::PodStoreSpec::ext4(pod_store_id);
    let mut pod = PodBuilder::new(
        "pod-emptydir",
        vm_builder.build().expect("vm config"),
        pod_store,
    )
    .expect("pod builder")
    .empty_dir(EmptyDirVolume::disk("work").expect("emptyDir"))
    .spawn()
    .await
    .expect("spawn pod");

    pod.add_container(
        PodContainerSpec::new("writer", PodRootfsSource::oci_bundle(image.clone()))
            .expect("writer spec")
            .command(["/bin/sh", "-c", "printf pod-emptydir-ok >/work/marker"])
            .empty_dir_mount("work", "/work")
            .expect("writer emptyDir")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .await
    .expect("add writer");
    let writer = pod.wait_container("writer").await.expect("writer output");
    assert!(writer.status.success(), "writer status={:?}", writer.status);

    pod.add_container(
        PodContainerSpec::new("reader", PodRootfsSource::oci_bundle(image))
            .expect("reader spec")
            .command(["/bin/cat", "/work/marker"])
            .empty_dir_mount("work", "/work")
            .expect("reader emptyDir")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .await
    .expect("add reader");
    let reader = pod.wait_container("reader").await.expect("reader output");
    pod.stop().await.expect("stop pod");

    assert!(
        reader.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        reader.status,
        String::from_utf8_lossy(&reader.stdout),
        String::from_utf8_lossy(&reader.stderr)
    );
    assert_eq!(reader.stdout, b"pod-emptydir-ok");
    assert!(reader.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live pod loopback smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_pod_two_busybox_containers_share_loopback() {
    let image = live_arm64_busybox_image_bundle().await;
    let temp = tempfile::tempdir().expect("pod loopback tempdir");
    let pod_store_path = temp.path().join("pod-loopback-store.ext4");
    write_empty_pod_store(&pod_store_path);

    let (vm_builder, pod_store_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(&pod_store_path);
    let mut pod = PodBuilder::new(
        "pod-loopback",
        vm_builder.build().expect("vm config"),
        firkin_core::PodStoreSpec::ext4(pod_store_id),
    )
    .expect("pod builder")
    .spawn()
    .await
    .expect("spawn pod");

    pod.add_container(
        PodContainerSpec::new("server", PodRootfsSource::oci_bundle(image.clone()))
            .expect("server spec")
            .command([
                "/bin/sh",
                "-c",
                "while true; do printf pod-loopback-ok | /bin/nc -l -p 18084 -s 127.0.0.1; done",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .await
    .expect("add server");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    pod.add_container(
        PodContainerSpec::new("client", PodRootfsSource::oci_bundle(image))
            .expect("client spec")
            .command(["/bin/nc", "127.0.0.1", "18084"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .await
    .expect("add client");
    let client = pod.wait_container("client").await.expect("client output");
    let _ = pod.remove_container("server").await;
    pod.stop().await.expect("stop pod");

    assert!(
        client.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        client.status,
        String::from_utf8_lossy(&client.stdout),
        String::from_utf8_lossy(&client.stderr)
    );
    assert_eq!(client.stdout, b"pod-loopback-ok");
    assert!(client.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live pod add/remove smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_pod_add_and_remove_container_without_vm_reboot() {
    let image = live_arm64_busybox_image_bundle().await;
    let temp = tempfile::tempdir().expect("pod add/remove tempdir");
    let pod_store_path = temp.path().join("pod-add-remove-store.ext4");
    write_empty_pod_store(&pod_store_path);

    let (vm_builder, pod_store_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(&pod_store_path);
    let mut pod = PodBuilder::new(
        "pod-add-remove",
        vm_builder.build().expect("vm config"),
        firkin_core::PodStoreSpec::ext4(pod_store_id),
    )
    .expect("pod builder")
    .empty_dir(EmptyDirVolume::disk("work").expect("emptyDir"))
    .container(
        PodContainerSpec::new("anchor", PodRootfsSource::oci_bundle(image.clone()))
            .expect("anchor spec")
            .command([
                "/bin/sh",
                "-c",
                "printf anchor-ready >/work/anchor; sleep 30",
            ])
            .empty_dir_mount("work", "/work")
            .expect("anchor emptyDir")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .spawn()
    .await
    .expect("spawn pod");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    pod.add_container(
        PodContainerSpec::new("reader", PodRootfsSource::oci_bundle(image))
            .expect("reader spec")
            .command(["/bin/cat", "/work/anchor"])
            .empty_dir_mount("work", "/work")
            .expect("reader emptyDir")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .await
    .expect("add reader");
    let reader = pod.wait_container("reader").await.expect("reader output");
    assert_eq!(reader.stdout, b"anchor-ready");

    let anchor_id = ContainerId::new("anchor").expect("anchor id");
    let anchor = pod.container_mut(&anchor_id).expect("anchor still tracked");
    let exec = anchor
        .exec(
            "anchor-check",
            ExecConfig::builder()
                .command(["/bin/cat", "/work/anchor"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .build(),
        )
        .await
        .expect("exec in anchor")
        .wait_with_output()
        .await
        .expect("anchor exec output");
    let _ = pod.remove_container("anchor").await;
    pod.stop().await.expect("stop pod");

    assert!(
        exec.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        exec.status,
        String::from_utf8_lossy(&exec.stdout),
        String::from_utf8_lossy(&exec.stderr)
    );
    assert_eq!(exec.stdout, b"anchor-ready");
    assert!(exec.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM shared-loopback substrate smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_two_busybox_containers_share_loopback_on_one_existing_vm() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping shared-loopback live smoke");
        return;
    };
    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(rootfs_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot VM");
    let caps = net_admin_caps();

    let mut server = vm
        .container("live-loopback-server")
        .expect("server container builder")
        .capabilities(caps.clone())
        .command([
            "/bin/sh",
            "-c",
            "/bin/busybox ip link set lo up && while true; do printf pod-smoke-ok | /bin/nc -l -p 18080 -s 127.0.0.1; done",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn loopback server on existing vm");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let client = vm
        .container("live-loopback-client")
        .expect("client container builder")
        .capabilities(caps)
        .command([
            "/bin/sh",
            "-c",
            "/bin/busybox ip link set lo up && /bin/nc 127.0.0.1 18080",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn loopback client on existing vm")
        .wait_with_output()
        .await
        .expect("client output");

    let _ = server.kill(Signal::new(9)).await;
    let _ = server.wait().await;
    let _ = vm.clone().stop().await;

    assert!(
        client.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        client.status,
        String::from_utf8_lossy(&client.stdout),
        String::from_utf8_lossy(&client.stderr)
    );
    assert_eq!(client.stdout, b"pod-smoke-ok");
    assert!(client.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM shared-loopback no-cap substrate smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_two_busybox_containers_share_loopback_without_net_admin() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping no-cap loopback live smoke");
        return;
    };
    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(rootfs_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot VM");

    let mut server = vm
        .container("live-loopback-no-cap-server")
        .expect("server container builder")
        .command([
            "/bin/sh",
            "-c",
            "while true; do printf pod-smoke-no-cap | /bin/nc -l -p 18083 -s 127.0.0.1; done",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn no-cap loopback server on existing vm");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let client = vm
        .container("live-loopback-no-cap-client")
        .expect("client container builder")
        .command(["/bin/nc", "127.0.0.1", "18083"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn no-cap loopback client on existing vm")
        .wait_with_output()
        .await
        .expect("client output");

    let _ = server.kill(Signal::new(9)).await;
    let _ = server.wait().await;
    let _ = vm.clone().stop().await;

    assert!(
        client.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        client.status,
        String::from_utf8_lossy(&client.stdout),
        String::from_utf8_lossy(&client.stderr)
    );
    assert_eq!(client.stdout, b"pod-smoke-no-cap");
    assert!(client.stderr.is_empty());
}

#[tokio::test]
#[cfg(feature = "snapshot")]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM multi-container snapshot substrate smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_two_busybox_container_listeners_survive_one_vm_snapshot_restore() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!(
            "warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping multi-container snapshot smoke"
        );
        return;
    };
    let snapshot_dir = tempfile::tempdir().expect("snapshot dir");
    let snapshot = snapshot_dir.path().join("multi-container.vzstate");
    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(&rootfs_path);
    let config = vm_builder.build().expect("vm config");
    let persisted_machine_identifier = config.machine_identifier().to_vec();
    let persisted_network_macs = config.network_macs().to_vec();
    let caps = net_admin_caps();

    let vm = VirtualMachine::new(config).boot().await.expect("boot VM");
    let mut first = vm
        .container("live-snapshot-server-a")
        .expect("first server builder")
        .capabilities(caps.clone())
        .command([
            "/bin/sh",
            "-c",
            "/bin/busybox ip link set lo up && while true; do printf marker-a | /bin/nc -l -p 18081 -s 127.0.0.1; done",
        ])
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn first listener");
    let mut second = vm
        .container("live-snapshot-server-b")
        .expect("second server builder")
        .capabilities(caps.clone())
        .command([
            "/bin/sh",
            "-c",
            "/bin/busybox ip link set lo up && while true; do printf marker-b | /bin/nc -l -p 18082 -s 127.0.0.1; done",
        ])
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn second listener");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    assert_eq!(
        query_loopback_marker(&vm, rootfs_id, caps.clone(), 18081).await,
        b"marker-a"
    );
    assert_eq!(
        query_loopback_marker(&vm, rootfs_id, caps.clone(), 18082).await,
        b"marker-b"
    );
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    vm.save_snapshot(&snapshot).await.expect("save snapshot");
    let _ = first.kill(Signal::new(9)).await;
    let _ = second.kill(Signal::new(9)).await;
    let _ = first.wait().await;
    let _ = second.wait().await;
    vm.clone().stop().await.expect("stop cold VM");

    let (restore_builder, restored_rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .machine_identifier(persisted_machine_identifier)
        .network_macs(persisted_network_macs)
        .block_device(rootfs_path);
    let restored = VirtualMachine::new(restore_builder.build().expect("restore vm config"))
        .boot_or_restore(&snapshot)
        .await
        .expect("restore VM");

    assert_eq!(
        query_loopback_marker(&restored, restored_rootfs_id, caps.clone(), 18081).await,
        b"marker-a"
    );
    assert_eq!(
        query_loopback_marker(&restored, restored_rootfs_id, caps, 18082).await,
        b"marker-b"
    );
    restored.stop().await.expect("stop restored VM");
}

fn net_admin_caps() -> LinuxCapabilities {
    let mut caps = LinuxCapabilities::default_oci();
    caps.bounding.push(Capability::NetAdmin);
    caps.effective.push(Capability::NetAdmin);
    caps.permitted.push(Capability::NetAdmin);
    caps
}

#[cfg(feature = "snapshot")]
async fn query_loopback_marker(
    vm: &VirtualMachine<firkin_vmm::Running>,
    rootfs_id: BlockDeviceId,
    caps: LinuxCapabilities,
    port: u16,
) -> Vec<u8> {
    let command = format!("/bin/nc 127.0.0.1 {port}");
    let output = vm
        .container(format!("live-snapshot-query-{port}"))
        .expect("query builder")
        .capabilities(caps)
        .command(["/bin/sh", "-c", command.as_str()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn query")
        .wait_with_output();
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), output)
        .await
        .expect("query timed out")
        .expect("query output");
    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM Arc-shared explicit-container smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_runs_on_arc_shared_existing_vm() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping Arc explicit-VM live smoke");
        return;
    };
    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(rootfs_path);
    let vm = Arc::new(
        VirtualMachine::new(vm_builder.build().expect("vm config"))
            .boot()
            .await
            .expect("boot VM"),
    );

    let output = vm
        .container_shared("live-on-arc-vm")
        .expect("container builder")
        .command(["/bin/echo", "arc existing vm from firkin"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(Rootfs::block_device(rootfs_id))
        .spawn()
        .await
        .expect("spawn on Arc existing vm")
        .wait_with_output()
        .await
        .expect("container output");
    let _ = (*vm).clone().stop().await;

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"arc existing vm from firkin\n");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM explicit-container pty smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_pty_runs_on_existing_vm() {
    let Some(rootfs_path) = live_arm64_busybox_rootfs_path() else {
        eprintln!("warn: FIRKIN_ARM64_BUSYBOX_ROOTFS not set; skipping explicit-VM pty live smoke");
        return;
    };
    let (vm_builder, rootfs_id) = VmConfig::builder()
        .memory(Size::mib(512))
        .rosetta(false)
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(live_init_block_path())
        .block_device(rootfs_path);
    let vm = VirtualMachine::new(vm_builder.build().expect("vm config"))
        .boot()
        .await
        .expect("boot VM");

    let mut container = vm
        .container("live-on-vm-pty")
        .expect("container builder")
        .command([
            "/bin/sh",
            "-c",
            "test -t 0 && test -t 1 && printf 'existing vm pty from firkin\\n'",
        ])
        .rootfs(Rootfs::block_device(rootfs_id))
        .pty((80, 24))
        .spawn()
        .await
        .expect("spawn pty on existing vm");
    assert!(container.pid().is_some_and(|pid| pid > 0));
    let mut output = Vec::new();
    container
        .pty()
        .read_to_end(&mut output)
        .await
        .expect("read pty");
    let status = container.wait().await.expect("wait init");
    let _ = vm.stop().await;

    assert!(status.success(), "status={status:?}");
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("existing vm pty from firkin"), "{output:?}");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM init stdio smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_spawn_exposes_init_stdout_and_stderr_handles() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-init-streams")
        .expect("builder")
        .command([
            "/bin/sh",
            "-c",
            "printf 'init stdout from firkin\\n'; printf 'init stderr from firkin\\n' >&2",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let mut stdout = container
        .take_stdout()
        .await
        .expect("take stdout")
        .expect("piped stdout");
    let mut stderr = container
        .take_stderr()
        .await
        .expect("take stderr")
        .expect("piped stderr");
    let (stdout, stderr) = tokio::try_join!(
        async move {
            let mut output = Vec::new();
            stdout.read_to_end(&mut output).await?;
            Ok::<_, std::io::Error>(output)
        },
        async move {
            let mut output = Vec::new();
            stderr.read_to_end(&mut output).await?;
            Ok::<_, std::io::Error>(output)
        }
    )
    .expect("read stdio");
    let status = container.wait().await.expect("wait init");

    assert!(status.success(), "status={status:?}");
    assert_eq!(stdout, b"init stdout from firkin\n");
    assert_eq!(stderr, b"init stderr from firkin\n");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM init inherit smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_spawn_inherits_init_stdout_and_stderr() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-init-inherit")
        .expect("builder")
        .command([
            "/bin/sh",
            "-c",
            "printf 'inherited init stdout from firkin\\n'; printf 'inherited init stderr from firkin\\n' >&2",
        ])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    assert!(
        container
            .take_stdout()
            .await
            .expect("take stdout")
            .is_none()
    );
    assert!(
        container
            .take_stderr()
            .await
            .expect("take stderr")
            .is_none()
    );
    let status = container.wait().await.expect("wait init");

    assert!(status.success(), "status={status:?}");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM init stdin smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_spawn_streams_stdin_to_init_process() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-init-stdin")
        .expect("builder")
        .command(["/bin/cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let mut stdin = container
        .take_stdin()
        .await
        .expect("take stdin")
        .expect("piped stdin");
    stdin
        .write_all(b"init stdin from firkin\n")
        .await
        .expect("write stdin");
    drop(stdin);
    let output = container
        .wait_with_output()
        .await
        .expect("container output");

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"init stdin from firkin\n");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM init pty smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_spawn_pty_exposes_terminal_stream() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-init-pty")
        .expect("builder")
        .command([
            "/bin/sh",
            "-c",
            "test -t 0 && test -t 1 && printf 'init pty from firkin\\n'",
        ])
        .rootfs(rootfs)
        .pty((80, 24))
        .spawn()
        .await
        .expect("container spawn");
    let mut output = Vec::new();
    container
        .pty()
        .read_to_end(&mut output)
        .await
        .expect("read pty");
    let status = container.wait().await.expect("wait init");

    assert!(status.success(), "status={status:?}");
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("init pty from firkin"), "{output:?}");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM exec smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_exec_runs_in_existing_container() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-exec")
        .expect("builder")
        .command(["/bin/sleep", "5"])
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let process = container
        .exec(
            "echo",
            ExecConfig::builder()
                .command(["/bin/echo", "exec from firkin"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .build(),
        )
        .await
        .expect("exec start");
    assert!(process.pid().is_some_and(|pid| pid > 0));
    let output = process.wait_with_output().await.expect("exec output");
    let _ = container.stop().await;

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"exec from firkin\n");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM exec stdin smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_exec_streams_stdin_to_process() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-exec-stdin")
        .expect("builder")
        .command(["/bin/sleep", "5"])
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let mut process = container
        .exec(
            "cat-stdin",
            ExecConfig::builder()
                .command(["/bin/cat"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .build(),
        )
        .await
        .expect("exec start");
    let mut stdin = process
        .take_stdin()
        .await
        .expect("take stdin")
        .expect("piped stdin");
    stdin
        .write_all(b"stdin from firkin\n")
        .await
        .expect("write stdin");
    drop(stdin);
    let output = process.wait_with_output().await.expect("exec output");
    let _ = container.stop().await;

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"stdin from firkin\n");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM exec stdio smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_exec_exposes_stdout_and_stderr_handles() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-exec-streams")
        .expect("builder")
        .command(["/bin/sleep", "5"])
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let mut process = container
        .exec(
            "stdio",
            ExecConfig::builder()
                .command([
                    "/bin/sh",
                    "-c",
                    "printf 'stdout from firkin\\n'; printf 'stderr from firkin\\n' >&2",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .build(),
        )
        .await
        .expect("exec start");
    let mut stdout = process
        .take_stdout()
        .await
        .expect("take stdout")
        .expect("piped stdout");
    let mut stderr = process
        .take_stderr()
        .await
        .expect("take stderr")
        .expect("piped stderr");
    let (stdout, stderr) = tokio::try_join!(
        async move {
            let mut output = Vec::new();
            stdout.read_to_end(&mut output).await?;
            Ok::<_, std::io::Error>(output)
        },
        async move {
            let mut output = Vec::new();
            stderr.read_to_end(&mut output).await?;
            Ok::<_, std::io::Error>(output)
        }
    )
    .expect("read stdio");
    let status = process.wait().await.expect("wait stdio process");
    let _ = container.stop().await;

    assert!(status.success(), "status={status:?}");
    assert_eq!(stdout, b"stdout from firkin\n");
    assert_eq!(stderr, b"stderr from firkin\n");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM exec inherit smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_exec_inherits_stdout_and_stderr() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-exec-inherit")
        .expect("builder")
        .command(["/bin/sleep", "5"])
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let mut process = container
        .exec(
            "inherit",
            ExecConfig::builder()
                .command([
                    "/bin/sh",
                    "-c",
                    "printf 'inherited exec stdout from firkin\\n'; printf 'inherited exec stderr from firkin\\n' >&2",
                ])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .build(),
        )
        .await
        .expect("exec start");
    assert!(process.take_stdout().await.expect("take stdout").is_none());
    assert!(process.take_stderr().await.expect("take stderr").is_none());
    let status = process.wait().await.expect("wait inherit process");
    let _ = container.stop().await;

    assert!(status.success(), "status={status:?}");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM exec pty smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_exec_pty_exposes_terminal_stream() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let mut container = Container::builder("live-exec-pty")
        .expect("builder")
        .command(["/bin/sleep", "5"])
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let mut process = container
        .exec(
            "pty-check",
            ExecConfig::builder()
                .command([
                    "/bin/sh",
                    "-c",
                    "test -t 0 && test -t 1 && printf 'pty from firkin\\n'",
                ])
                .pty((80, 24))
                .build(),
        )
        .await
        .expect("exec start");
    let mut output = Vec::new();
    process
        .pty()
        .read_to_end(&mut output)
        .await
        .expect("read pty");
    let status = process.wait().await.expect("wait pty process");
    let _ = container.stop().await;

    assert!(status.success(), "status={status:?}");
    let output = String::from_utf8_lossy(&output);
    assert!(output.contains("pty from firkin"), "{output:?}");
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM copy smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_copy_round_trips_file() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let host = tempfile::tempdir().expect("host dir");
    let source = host.path().join("source.txt");
    let destination = host.path().join("destination.txt");
    tokio::fs::write(&source, b"copy from firkin\n")
        .await
        .expect("write source");
    let mut container = Container::builder("live-copy")
        .expect("builder")
        .command(["/bin/sleep", "5"])
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    container
        .copy_in(&source, "/tmp/source.txt")
        .await
        .expect("copy in");
    let output = container
        .exec(
            "cat-copy",
            ExecConfig::builder()
                .command(["/bin/cat", "/tmp/source.txt"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .build(),
        )
        .await
        .expect("exec start")
        .wait_with_output()
        .await
        .expect("exec output");
    container
        .copy_out("/tmp/source.txt", &destination)
        .await
        .expect("copy out");
    let _ = container.stop().await;

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"copy from firkin\n");
    assert!(output.stderr.is_empty());
    assert_eq!(
        tokio::fs::read(&destination).await.expect("read copy out"),
        b"copy from firkin\n"
    );
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM directory copy smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_copy_round_trips_directory() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let host = tempfile::tempdir().expect("host dir");
    let source = host.path().join("source-dir");
    let nested = source.join("nested");
    let destination = host.path().join("destination-dir");
    tokio::fs::create_dir_all(&nested)
        .await
        .expect("create source dir");
    tokio::fs::write(nested.join("message.txt"), b"directory copy from firkin\n")
        .await
        .expect("write source");
    let mut container = Container::builder("live-copy-dir")
        .expect("builder")
        .command(["/bin/sleep", "5"])
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let result = async {
        container.copy_in(&source, "/tmp/source-dir").await?;
        let output = container
            .exec(
                "cat-copy-dir",
                ExecConfig::builder()
                    .command(["/bin/cat", "/tmp/source-dir/nested/message.txt"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .build(),
            )
            .await?
            .wait_with_output()
            .await?;
        assert!(
            output.status.success(),
            "status={:?}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"directory copy from firkin\n");
        assert!(output.stderr.is_empty());
        container.copy_out("/tmp/source-dir", &destination).await?;
        Ok::<_, Error>(())
    }
    .await;
    let _ = container.stop().await;

    result.expect("directory copy round trip");
    assert_eq!(
        tokio::fs::read(destination.join("nested/message.txt"))
            .await
            .expect("read copy out"),
        b"directory copy from firkin\n"
    );
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM statistics smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_statistics_returns_requested_categories() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let container = Container::builder("live-stats")
        .expect("builder")
        .command(["/bin/sleep", "5"])
        .rootfs(rootfs)
        .spawn()
        .await
        .expect("container spawn");
    let result = container
        .statistics(StatCategory::PROCESS | StatCategory::CPU | StatCategory::NETWORK)
        .await;
    let _ = container.stop().await;
    let stats = result.expect("container statistics");

    assert_eq!(stats.id, "live-stats");
    assert!(stats.process.is_some(), "{stats:?}");
    assert!(stats.cpu.is_some(), "{stats:?}");
    assert!(
        stats
            .networks
            .as_ref()
            .is_some_and(|networks| !networks.is_empty()),
        "{stats:?}"
    );
    assert_eq!(stats.memory, None);
    assert_eq!(stats.block_io, None);
    assert_eq!(stats.memory_events, None);
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM vmnet smoke; requires macOS Virtualization.framework entitlement"
)]
async fn live_busybox_vmnet_gets_guest_ip_and_gateway() {
    let rootfs = live_arm64_busybox_rootfs().await;

    let output = Container::builder("live-vmnet")
        .expect("builder")
        .networks([Network::vmnet_shared_subnet("192.168.127.0/24")])
        .command([
            "/bin/sh",
            "-c",
            "/bin/busybox ip -4 addr show eth0 && /bin/busybox ping -c 1 -W 2 192.168.127.1",
        ])
        .rootfs(rootfs)
        .output()
        .await
        .expect("container output");

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("192.168.127.2/24"), "{stdout}");
    assert!(stdout.contains("1 packets received"), "{stdout}");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
#[cfg_attr(
    not(feature = "real-vm"),
    ignore = "live VM Rosetta smoke; requires macOS Virtualization.framework entitlement and Rosetta for Linux"
)]
async fn live_amd64_busybox_runs_through_rosetta() {
    let rootfs = live_amd64_busybox_rootfs().await;

    let output = Container::builder("live-rosetta")
        .expect("builder")
        .command(["/bin/uname", "-m"])
        .rootfs(rootfs)
        .output()
        .await
        .expect("container output");

    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"x86_64\n");
    assert!(output.stderr.is_empty());
}
