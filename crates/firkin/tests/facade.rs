//! Facade re-export tests.

use firkin::e2b::{
    ConnectRequest, ConnectedSandbox, LocalSandboxRegistry, LocalTemplateRegistry,
    SandboxCreateRequest, SandboxRoutes, SandboxRuntimeConfig, SandboxState as E2bSandboxState,
    TemplateBuildRequest,
};
use firkin::{
    BlankDiskImage, BootLog, Capability, Container, DiskImageFormat, DnsConfig, EmptyDirMedium,
    EmptyDirVolume, ExecConfig, FileMount, GuestFilesystem, GuestPath, Hostname, HostsConfig,
    HostsEntry, KernelImage, LinuxCapabilities, LinuxRlimit, MaterializedRootfs, Mount,
    NetworkPolicyRule, PodBuilder, PodContainerSpec, PodRootfsSource, PodStoreSpec, PodVolumeMount,
    PortSandboxHost, RlimitKind, Rootfs, RuntimeCapability, SandboxNetworkPolicy, Seccomp,
    SeccompAction, Size, UnixSocketConfig, User, VirtiofsTag, VsockPort,
    apple_local_runtime_capabilities, create_blank_disk_image, materialize_rootfs_in_pod_store,
};
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroU32;

#[test]
fn facade_exports_single_node_runtime_surface() {
    let config = firkin::runtime::single_node::SingleNodeConfig::new(
        "/tmp/firkin-single-node",
        "cube.localhost",
    );
    assert_eq!(config.domain(), "cube.localhost");
    assert_eq!(config.minimum_free_disk(), Size::gib(10));
}

#[tokio::test]
async fn facade_exports_sandbox_public_surface() {
    use firkin::sandbox::apple_vz::AppleVzBackend;
    use firkin::sandbox::{DataPlaneSpec, Runtime, SandboxSpec, TemplateSpec};

    let backend = AppleVzBackend::from_config(firkin::sandbox::apple_vz::SingleNodeConfig::new(
        "/tmp/firkin-sandbox-facade",
        "cube.localhost",
    ));
    let runtime = Runtime::build(backend).await.expect("runtime");
    let template = runtime
        .templates()
        .prepare(TemplateSpec::oci("example").data_plane(DataPlaneSpec::none()))
        .await
        .expect("template");
    let spec = SandboxSpec::from_template(&template);

    assert_eq!(template.id().as_str(), "example");
    assert!(matches!(spec, SandboxSpec::Template { .. }));
}

#[test]
fn facade_exports_primary_builder_surface() {
    let builder = Container::builder("worker")
        .expect("builder")
        .memory(Size::mib(512))
        .rootfs(Rootfs::raw_block("/tmp/rootfs.img"));

    assert_eq!(builder.id().as_str(), "worker");
    assert_eq!(builder.memory_size(), Size::mib(512));

    let exec = ExecConfig::builder()
        .command(["/bin/echo", "hello"])
        .build();
    assert!(format!("{exec:?}").contains("/bin/echo"));

    let image = BlankDiskImage::asif("/tmp/pod-store.asif", Size::mib(64));
    assert_eq!(image.format(), DiskImageFormat::Asif);
    let _: fn(&BlankDiskImage) -> firkin::vmm::Result<()> = create_blank_disk_image;

    let guest_path = GuestPath::new("/run/firkin/pod-store").expect("guest path");
    assert_eq!(guest_path.as_str(), "/run/firkin/pod-store");
    let store = PodStoreSpec::ext4(firkin::types::BlockDeviceId::from_slot(
        NonZeroU32::new(1).expect("slot"),
    ));
    assert_eq!(store.filesystem(), GuestFilesystem::Ext4);

    let empty_dir = EmptyDirVolume::disk("work").expect("emptyDir");
    assert_eq!(empty_dir.medium(), EmptyDirMedium::Disk);
    let rootfs = GuestPath::new("/run/firkin/pod-store/pods/p/rootfs/c").expect("rootfs");
    let materialized = MaterializedRootfs::new(rootfs.clone(), Some("sha256:test".to_owned()));
    assert_eq!(materialized.path(), &rootfs);
    let container = PodContainerSpec::new("agent", PodRootfsSource::guest_path(rootfs.clone()))
        .expect("pod container")
        .empty_dir_mount("work", "/work")
        .expect("emptyDir mount");
    assert_eq!(
        container.empty_dir_mounts()[0].container_path(),
        std::path::Path::new("/work")
    );
    let mount = PodVolumeMount::read_write("work", "/work").expect("volume mount");
    assert_eq!(mount.volume_name(), "work");
    let pod = PodBuilder::new(
        "pod",
        firkin::vmm::VmConfig::builder().build().unwrap(),
        store,
    )
    .unwrap()
    .empty_dir(empty_dir)
    .container(container);
    assert_eq!(pod.containers().len(), 1);
    let exported_fn = materialize_rootfs_in_pod_store;
    assert_eq!(std::mem::size_of_val(&exported_fn), 0);
}

#[test]
fn prelude_exports_primary_builder_surface() {
    use firkin::prelude::*;

    let builder = Container::builder("worker")
        .expect("builder")
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    assert_eq!(builder.id().as_str(), "worker");

    let _: RuntimeCapabilities = apple_local_runtime_capabilities();
    let image = BlankDiskImage::raw("/tmp/pod-store.raw", Size::mib(64));
    assert_eq!(image.format(), DiskImageFormat::Raw);
    let _: fn(&BlankDiskImage) -> firkin::vmm::Result<()> = create_blank_disk_image;
    let exported_mount_fn = mount_pod_store;
    let exported_materialize_fn = materialize_rootfs_in_pod_store;
    assert_eq!(std::mem::size_of_val(&exported_mount_fn), 0);
    assert_eq!(std::mem::size_of_val(&exported_materialize_fn), 0);
    let empty_dir = EmptyDirVolume::disk("work").expect("emptyDir");
    assert_eq!(empty_dir.name(), "work");
}

#[test]
fn facade_exports_apple_local_runtime_capabilities() {
    let capabilities = apple_local_runtime_capabilities();

    assert_eq!(capabilities.backend(), "apple-vz");
    assert!(
        capabilities
            .supported()
            .contains(&RuntimeCapability::supported(
                "create-linux-container",
                None
            ))
    );
    assert!(capabilities.supports("create-linux-container"));
    assert!(
        capabilities
            .require_supported("create-linux-container")
            .is_ok()
    );

    let unsupported_names = capabilities
        .unsupported()
        .iter()
        .map(|capability| capability.name())
        .collect::<Vec<_>>();
    assert_eq!(
        unsupported_names,
        vec![
            "e2b-envd-compatible-api",
            "snapshot-backed-pause-resume-connect",
            "e2b-network-policy",
            "domain-host-proxy",
            "template-build-service",
        ]
    );

    for capability in capabilities.unsupported() {
        assert!(!capability.is_supported());
        assert!(capability.reason().is_some_and(|reason| !reason.is_empty()));
    }

    let error = capabilities
        .require_supported("domain-host-proxy")
        .expect_err("unsupported capability");
    assert_eq!(error.name(), "domain-host-proxy");
    assert!(error.reason().contains("{port}-{sandboxID}.domain proxy"));
    assert!(error.to_string().contains("domain-host-proxy"));
}

#[test]
fn facade_exports_process_policy_types() {
    let builder = Container::builder("worker")
        .expect("builder")
        .user(User::from(1000))
        .capabilities(LinuxCapabilities::single_set(vec![Capability::Chown]))
        .rlimit(LinuxRlimit::new(RlimitKind::OpenFiles, 2048, 1024))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let process = builder.runtime_spec().unwrap().process.unwrap();

    assert_eq!(
        process.capabilities.unwrap().effective,
        Some(vec!["CAP_CHOWN".to_owned()])
    );
    assert_eq!(process.user.uid, 1000);
    assert_eq!(process.user.gid, 1000);
    assert_eq!(process.rlimits[0].kind, "RLIMIT_NOFILE");
}

#[test]
fn facade_exports_mount_surface() {
    let builder = Container::builder("worker")
        .expect("builder")
        .default_mounts(vec![Mount::proc("/proc")])
        .mount(Mount::tmpfs("/scratch"))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().unwrap();

    assert_eq!(
        spec.mounts
            .iter()
            .map(|mount| mount.destination.as_str())
            .collect::<Vec<_>>(),
        vec!["/proc", "/scratch"]
    );
}

#[test]
fn facade_exports_use_init_surface() {
    let builder = Container::builder("worker")
        .expect("builder")
        .command(["/bin/true"])
        .use_init(true)
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    assert_eq!(
        builder.runtime_spec().unwrap().process.unwrap().args,
        vec!["/.cz-init", "--", "/bin/true"]
    );
}

#[test]
fn facade_exports_seccomp_surface() {
    let builder = Container::builder("worker")
        .expect("builder")
        .seccomp(Seccomp {
            default_action: SeccompAction::Allow,
            default_errno_ret: None,
            architectures: Vec::new(),
            flags: Vec::new(),
            listener_path: String::new(),
            listener_metadata: String::new(),
            syscalls: Vec::new(),
        })
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    assert!(
        builder
            .runtime_spec()
            .unwrap()
            .linux
            .unwrap()
            .seccomp
            .is_some()
    );
}

#[test]
fn facade_exports_dns_and_hosts_surface() {
    let builder = Container::builder("worker")
        .expect("builder")
        .dns(DnsConfig {
            nameservers: vec![IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))],
            domain: None,
            search: Vec::new(),
            options: Vec::new(),
        })
        .hosts(HostsConfig {
            entries: vec![HostsEntry {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                hostnames: vec!["localhost".to_owned()],
                comment: None,
            }],
            comment: None,
        })
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    assert_eq!(builder.id().as_str(), "worker");
}

#[test]
fn facade_exports_socket_relay_surface() {
    let builder = Container::builder("worker")
        .expect("builder")
        .default_mounts(Vec::new())
        .socket(UnixSocketConfig::into_guest(
            "api",
            "/tmp/api.sock",
            "/run/api.sock",
        ))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().unwrap();

    assert_eq!(
        spec.mounts[0].source,
        "/run/container/worker/sockets/api.sock"
    );
}

#[test]
fn facade_exports_file_mount_surface() {
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("config.json");
    std::fs::write(&source, b"{}").unwrap();
    let builder = Container::builder("worker")
        .expect("builder")
        .default_mounts(Vec::new())
        .file_mount(FileMount::read_only(&source, "/etc/app/config.json"))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    let spec = builder.runtime_spec().unwrap();

    assert_eq!(spec.mounts[0].destination, "/etc/app/config.json");
}

#[test]
fn facade_exports_implicit_vm_config_types() {
    let builder = Container::builder("worker")
        .expect("builder")
        .cpus(NonZeroU32::new(2).unwrap())
        .virtiofs_share(VirtiofsTag::new("shared").unwrap(), "/tmp")
        .boot_log(BootLog::None)
        .kernel(KernelImage::from_file("/tmp/vmlinux"))
        .rootfs(Rootfs::ext4_image("/tmp/rootfs.ext4"));

    assert_eq!(builder.cpu_count().get(), 2);
}

#[test]
fn facade_exports_container_vsock_surface() {
    async fn calls_dial_vsock(container: &Container) {
        let _ = container.dial_vsock(VsockPort::new(2222)).await;
    }

    let _ = calls_dial_vsock;
}

#[test]
fn facade_exports_proxy_host_route_type() {
    let domain = Hostname::new("cube.localhost").unwrap();
    let route = PortSandboxHost::parse_for_domain("49983-sbx.cube.localhost", &domain).unwrap();

    assert_eq!(route.to_string(), "49983-sbx.cube.localhost");
}

#[test]
fn facade_exports_e2b_network_policy_shape() {
    let rule = NetworkPolicyRule::new("api.example.com").unwrap();
    let policy = SandboxNetworkPolicy::new(Some(true), [rule], [], Some(true), None);

    assert_eq!(policy.allow_internet_access(), Some(true));
    assert_eq!(policy.allow_public_traffic(), Some(true));
    assert!(policy.requires_policy_engine());
}

#[test]
fn facade_exports_e2b_control_plane_contract_types() {
    let create = SandboxCreateRequest::default();
    let connected = ConnectedSandbox {
        sandbox_id: "sbx_123".to_owned(),
        envd_version: "0.5.7".to_owned(),
        envd_access_token: None,
        traffic_access_token: None,
        domain: Some("cube.localhost".to_owned()),
    };

    assert_eq!(create.template_id, "base");
    assert_eq!(connected.sandbox_id, "sbx_123");
    assert_eq!(ConnectRequest { timeout: 300 }.timeout, 300);
    assert_eq!(E2bSandboxState::Running, E2bSandboxState::Running);
    assert_eq!(SandboxRoutes::pause("sbx_123"), "/sandboxes/sbx_123/pause");

    let mut registry = LocalSandboxRegistry::new();
    registry
        .create(
            create,
            SandboxRuntimeConfig {
                sandbox_id: "sbx_123".to_owned(),
                domain: "cube.localhost".to_owned(),
                envd_version: "0.5.7".to_owned(),
                envd_access_token: None,
                traffic_access_token: None,
                started_at: "2026-05-03T12:00:00Z".to_owned(),
                end_at: "2026-05-03T12:05:00Z".to_owned(),
                cpu_count: 2,
                memory_mb: 1024,
            },
        )
        .unwrap();
    assert_eq!(
        registry.get("sbx_123").unwrap().state,
        E2bSandboxState::Running
    );

    let mut templates = LocalTemplateRegistry::new("2026-05-03T12:00:00Z");
    let requested = templates.request_build(TemplateBuildRequest {
        name: Some("agent-template".to_owned()),
        ..TemplateBuildRequest::default()
    });
    assert_eq!(requested.template_id, "tpl_1");
}
