#![allow(missing_docs)]

use firkin_oci::{
    ImageConfig, Linux, LinuxCapabilities, LinuxNamespace, LinuxNamespaceType, LinuxResources,
    LinuxSeccompAction, LinuxSeccompArch, LinuxSeccompArg, LinuxSeccompOperator,
    LinuxSeccompProfile, LinuxSyscall, Mount, Process, Root, Spec, User,
};
use serde_json::json;

#[test]
fn minimal_spec_decode_matches_swift_defaults() {
    let decoded: Spec = serde_json::from_value(json!({
        "ociVersion": "1.2.3"
    }))
    .unwrap();

    assert_eq!(decoded.version, "1.2.3");
    assert!(decoded.hooks.is_none());
    assert!(decoded.process.is_none());
    assert_eq!(decoded.hostname, "");
    assert_eq!(decoded.domainname, "");
    assert!(decoded.mounts.is_empty());
    assert!(decoded.annotations.is_none());
    assert!(decoded.root.is_none());
    assert!(decoded.linux.is_none());
}

#[test]
fn minimal_child_specs_decode_with_swift_defaults() {
    let process: Process = serde_json::from_value(json!({
        "cwd": "/work",
        "user": {
            "uid": 10,
            "gid": 11
        }
    }))
    .unwrap();
    assert_eq!(process.cwd, "/work");
    assert_eq!(process.env, Vec::<String>::new());
    assert_eq!(process.selinux_label, "");
    assert!(!process.no_new_privileges);
    assert_eq!(process.command_line, "");
    assert!(process.oom_score_adj.is_none());
    assert!(process.capabilities.is_none());
    assert_eq!(process.apparmor_profile, "");
    assert_eq!(process.user.uid, 10);
    assert_eq!(process.user.gid, 11);
    assert!(process.rlimits.is_empty());
    assert!(!process.terminal);

    let user: User = serde_json::from_value(json!({ "uid": 10, "gid": 11 })).unwrap();
    assert_eq!(user.uid, 10);
    assert_eq!(user.gid, 11);
    assert!(user.umask.is_none());
    assert!(user.additional_gids.is_empty());
    assert_eq!(user.username, "");

    let root: Root = serde_json::from_value(json!({ "path": "/rootfs" })).unwrap();
    assert_eq!(root.path, "/rootfs");
    assert!(!root.readonly);

    let mount: Mount = serde_json::from_value(json!({ "destination": "/proc" })).unwrap();
    assert_eq!(mount.kind, "");
    assert_eq!(mount.source, "");
    assert_eq!(mount.destination, "/proc");
    assert!(mount.options.is_empty());
    assert!(mount.uid_mappings.is_none());
    assert!(mount.gid_mappings.is_none());
}

#[test]
fn vminitd_runtime_spec_encoding_keeps_strict_empty_fields() {
    let spec = Spec {
        version: "1.1.0".to_owned(),
        process: Some(Process {
            args: vec!["/bin/echo".to_owned(), "hi".to_owned()],
            capabilities: Some(LinuxCapabilities::same_set(["CAP_CHOWN", "CAP_NET_RAW"])),
            ..Process::default()
        }),
        root: Some(Root {
            path: "/run/container/demo/rootfs".to_owned(),
            readonly: true,
        }),
        linux: Some(Linux {
            resources: Some(LinuxResources::default()),
            namespaces: vec![
                LinuxNamespace::unshare(LinuxNamespaceType::Pid),
                LinuxNamespace::unshare(LinuxNamespaceType::Network),
                LinuxNamespace::unshare(LinuxNamespaceType::Mount),
            ],
            ..Linux::default()
        }),
        ..Spec::default()
    };

    let encoded = serde_json::to_value(&spec).unwrap();
    assert_eq!(encoded["ociVersion"], "1.1.0");
    assert_eq!(
        encoded["linux"]["namespaces"][0],
        json!({
            "type": "pid",
            "path": ""
        })
    );
    assert_eq!(
        encoded["linux"]["namespaces"][1],
        json!({
            "type": "network",
            "path": ""
        })
    );
    assert_eq!(
        encoded["linux"]["resources"],
        json!({
            "devices": [],
            "hugepageLimits": [],
            "unified": {}
        })
    );
    assert_eq!(
        encoded["process"]["capabilities"],
        json!({
            "bounding": ["CAP_CHOWN", "CAP_NET_RAW"],
            "effective": ["CAP_CHOWN", "CAP_NET_RAW"],
            "inheritable": ["CAP_CHOWN", "CAP_NET_RAW"],
            "permitted": ["CAP_CHOWN", "CAP_NET_RAW"],
            "ambient": ["CAP_CHOWN", "CAP_NET_RAW"]
        })
    );
}

#[test]
fn linux_seccomp_profile_encodes_swift_oci_shape() {
    let profile = LinuxSeccompProfile {
        default_action: LinuxSeccompAction::Errno,
        default_errno_ret: Some(38),
        architectures: vec![LinuxSeccompArch::Aarch64],
        flags: Vec::new(),
        listener_path: String::new(),
        listener_metadata: String::new(),
        syscalls: vec![LinuxSyscall {
            names: vec!["read".to_owned(), "write".to_owned()],
            action: LinuxSeccompAction::Allow,
            errno_ret: None,
            args: vec![LinuxSeccompArg {
                index: 0,
                value: 1,
                value_two: Some(0xff),
                op: LinuxSeccompOperator::MaskedEqual,
            }],
        }],
    };

    assert_eq!(
        serde_json::to_value(profile).unwrap(),
        json!({
            "defaultAction": "SCMP_ACT_ERRNO",
            "defaultErrnoRet": 38,
            "architectures": ["SCMP_ARCH_AARCH64"],
            "flags": [],
            "listenerPath": "",
            "listenerMetadata": "",
            "syscalls": [{
                "names": ["read", "write"],
                "action": "SCMP_ACT_ALLOW",
                "args": [{
                    "index": 0,
                    "value": 1,
                    "valueTwo": 255,
                    "op": "SCMP_CMP_MASKED_EQ"
                }]
            }]
        })
    );
}

#[test]
fn image_config_decodes_swift_oci_config_keys() {
    let config: ImageConfig = serde_json::from_value(json!({
        "User": "1000:1001",
        "Env": ["PATH=/usr/bin", "RUST_LOG=debug"],
        "Entrypoint": ["/bin/sh", "-c"],
        "Cmd": ["echo hi"],
        "WorkingDir": "/work",
        "Labels": {
            "org.opencontainers.image.title": "demo"
        },
        "StopSignal": "SIGTERM"
    }))
    .unwrap();

    assert_eq!(config.user.as_deref(), Some("1000:1001"));
    assert_eq!(
        config.command_args(),
        vec!["/bin/sh".to_owned(), "-c".to_owned(), "echo hi".to_owned()]
    );
    assert_eq!(
        config.env.as_ref().unwrap(),
        &vec!["PATH=/usr/bin".to_owned(), "RUST_LOG=debug".to_owned()]
    );
    assert_eq!(config.working_dir.as_deref(), Some("/work"));
    assert_eq!(
        config.labels.as_ref().unwrap()["org.opencontainers.image.title"],
        "demo"
    );
    assert_eq!(config.stop_signal.as_deref(), Some("SIGTERM"));
}

#[test]
fn default_oci_capabilities_match_swift_runtime_defaults() {
    let caps = LinuxCapabilities::default_oci();
    let expected = vec![
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

    assert_eq!(caps.bounding.as_ref().unwrap(), &expected);
    assert_eq!(caps.effective.as_ref().unwrap(), &expected);
    assert_eq!(caps.permitted.as_ref().unwrap(), &expected);
    assert_eq!(caps.inheritable.as_ref().unwrap(), &Vec::<String>::new());
    assert_eq!(caps.ambient.as_ref().unwrap(), &Vec::<String>::new());
}

#[test]
fn mount_defaults_match_swift_linux_container_defaults() {
    let mounts = Mount::defaults();

    assert_eq!(
        mounts
            .iter()
            .map(|mount| (
                mount.kind.as_str(),
                mount.source.as_str(),
                mount.destination.as_str(),
                mount.options.as_slice()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("proc", "proc", "/proc", &[][..]),
            (
                "sysfs",
                "sysfs",
                "/sys",
                &["nosuid".to_owned(), "noexec".to_owned(), "nodev".to_owned()][..]
            ),
            (
                "devtmpfs",
                "none",
                "/dev",
                &["nosuid".to_owned(), "mode=755".to_owned()][..]
            ),
            (
                "mqueue",
                "mqueue",
                "/dev/mqueue",
                &["nosuid".to_owned(), "noexec".to_owned(), "nodev".to_owned()][..]
            ),
            (
                "tmpfs",
                "tmpfs",
                "/dev/shm",
                &[
                    "nosuid".to_owned(),
                    "noexec".to_owned(),
                    "nodev".to_owned(),
                    "mode=1777".to_owned(),
                    "size=65536k".to_owned()
                ][..]
            ),
            (
                "cgroup2",
                "none",
                "/sys/fs/cgroup",
                &["nosuid".to_owned(), "noexec".to_owned(), "nodev".to_owned()][..]
            ),
            (
                "devpts",
                "devpts",
                "/dev/pts",
                &[
                    "nosuid".to_owned(),
                    "noexec".to_owned(),
                    "newinstance".to_owned(),
                    "gid=5".to_owned(),
                    "mode=0620".to_owned(),
                    "ptmxmode=0666".to_owned()
                ][..]
            ),
        ]
    );
}

#[test]
fn mount_constructors_deduplicate_options() {
    let mount = Mount::bind("/host", "/guest")
        .read_only()
        .read_only()
        .no_suid()
        .extra_option("ro");

    assert_eq!(mount.kind, "bind");
    assert_eq!(mount.source, "/host");
    assert_eq!(mount.destination, "/guest");
    assert_eq!(mount.options, ["bind", "rw", "rbind", "ro", "nosuid"]);
}
