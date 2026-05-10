//! Tests for vminitd-specific request shaping.

use std::net::Ipv4Addr;

use firkin_oci::{Process, Root, Spec, User};
use firkin_types::{ContainerId, NamespaceKind, ProcessId, VsockPort};
use firkin_vminitd_client::{
    ApplyOciLayer, ContainerBundle, ContainerStatistics, ContainerStatisticsQuery, CopyMetadata,
    CopyResponseEvent, CopyTransfer, FilesystemUsage, Fstrim, LinuxNamespace, NetworkConfig,
    ProcessCreate, ProcessStdio, RemovePath, RosettaSetup, SocketProxy, StatCategory, VMINITD_PORT,
    pb, stop_socket_proxy_request,
};
use serde_json::json;

#[test]
fn container_bundle_uses_the_vminitd_implicit_path() {
    let id = ContainerId::new("container-0").expect("container id");
    let bundle = ContainerBundle::for_id(&id);

    assert_eq!(bundle.path(), "/run/container/container-0");
    assert_eq!(bundle.rootfs_path(), "/run/container/container-0/rootfs");
    assert_eq!(
        bundle.config_json_path(),
        "/run/container/container-0/config.json"
    );
}

#[test]
fn vminitd_port_matches_the_spike_verified_rpc_port() {
    assert_eq!(VMINITD_PORT, VsockPort::new(1024));
}

#[test]
fn remove_path_request_records_recursive_allow_missing() {
    let request = RemovePath::recursive("/run/firkin/pod-store/pods/p/containers/c")
        .allow_missing(true)
        .into_request();

    assert_eq!(request.path, "/run/firkin/pod-store/pods/p/containers/c");
    assert!(request.recursive);
    assert!(request.allow_missing);
}

#[test]
fn fstrim_request_defaults_to_whole_mount() {
    let request = Fstrim::new("/run/firkin/pod-store").into_request();

    assert_eq!(request.path, "/run/firkin/pod-store");
    assert_eq!(request.minimum_bytes, 0);
}

#[test]
fn apply_oci_layer_request_names_archive_and_destination() {
    let request = ApplyOciLayer::new(
        "/run/firkin/layers/sha256-layer.tar.gz",
        "/run/firkin/pod-store/pods/p/templates/t/rootfs",
    )
    .into_request();

    assert_eq!(
        request.archive_path,
        "/run/firkin/layers/sha256-layer.tar.gz"
    );
    assert_eq!(
        request.destination,
        "/run/firkin/pod-store/pods/p/templates/t/rootfs"
    );
}

#[test]
fn filesystem_usage_request_records_path() {
    let request = FilesystemUsage::new("/run/firkin/pod-store").into_request();

    assert_eq!(request.path, "/run/firkin/pod-store");
}

#[test]
fn copy_transfer_builds_vminitd_copy_requests_and_events() {
    let copy_in =
        CopyTransfer::copy_in("/run/container/unit/rootfs/tmp/in.txt", VsockPort::new(42))
            .mode(0o600)
            .create_parents(true)
            .into_request();
    assert_eq!(
        copy_in,
        pb::CopyRequest {
            direction: pb::copy_request::Direction::CopyIn as i32,
            path: "/run/container/unit/rootfs/tmp/in.txt".into(),
            mode: 0o600,
            create_parents: true,
            vsock_port: 42,
            is_archive: false,
        }
    );

    let copy_out =
        CopyTransfer::copy_out("/run/container/unit/rootfs/tmp/out.txt", VsockPort::new(43))
            .into_request();
    assert_eq!(
        copy_out,
        pb::CopyRequest {
            direction: pb::copy_request::Direction::CopyOut as i32,
            path: "/run/container/unit/rootfs/tmp/out.txt".into(),
            mode: 0,
            create_parents: false,
            vsock_port: 43,
            is_archive: false,
        }
    );

    let metadata = CopyResponseEvent::try_from(pb::CopyResponse {
        status: pb::copy_response::Status::Metadata as i32,
        is_archive: false,
        total_size: 128,
        error: String::new(),
    })
    .expect("metadata event");
    assert_eq!(
        metadata,
        CopyResponseEvent::Metadata(CopyMetadata {
            is_archive: false,
            total_size: 128,
        })
    );
    assert_eq!(
        CopyResponseEvent::try_from(pb::CopyResponse {
            status: pb::copy_response::Status::Complete as i32,
            is_archive: false,
            total_size: 0,
            error: String::new(),
        })
        .expect("complete event"),
        CopyResponseEvent::Complete
    );
}

#[test]
fn socket_proxy_builds_vminitd_proxy_requests() {
    let into_guest = SocketProxy::into_guest("sock-0", VsockPort::new(44), "/run/sockets/sock-0")
        .permissions(Some(0o660))
        .into_request();
    assert_eq!(
        into_guest,
        pb::ProxyVsockRequest {
            id: "sock-0".into(),
            vsock_port: 44,
            guest_path: "/run/sockets/sock-0".into(),
            guest_socket_permissions: Some(0o660),
            action: pb::proxy_vsock_request::Action::Into as i32,
        }
    );

    let out_of_guest =
        SocketProxy::out_of_guest("sock-1", VsockPort::new(45), "/run/app.sock").into_request();
    assert_eq!(
        out_of_guest,
        pb::ProxyVsockRequest {
            id: "sock-1".into(),
            vsock_port: 45,
            guest_path: "/run/app.sock".into(),
            guest_socket_permissions: None,
            action: pb::proxy_vsock_request::Action::OutOf as i32,
        }
    );

    assert_eq!(
        stop_socket_proxy_request("sock-0"),
        pb::StopVsockProxyRequest {
            id: "sock-0".into(),
        }
    );
}

#[test]
fn container_statistics_query_and_mapping_preserve_requested_categories() {
    let categories = StatCategory::PROCESS | StatCategory::CPU | StatCategory::NETWORK;
    let request = ContainerStatisticsQuery::new(["unit"])
        .categories(categories)
        .into_request();
    assert_eq!(
        request,
        pb::ContainerStatisticsRequest {
            container_ids: vec!["unit".into()],
            categories: vec![
                pb::StatCategory::Process as i32,
                pb::StatCategory::Cpu as i32,
                pb::StatCategory::Network as i32,
            ],
        }
    );

    let stats = ContainerStatistics::from_proto(
        pb::ContainerStats {
            container_id: "unit".into(),
            process: Some(pb::ProcessStats {
                current: 2,
                limit: 64,
            }),
            cpu: Some(pb::CpuStats {
                usage_usec: 10,
                user_usec: 4,
                system_usec: 6,
                throttling_periods: 1,
                throttled_periods: 0,
                throttled_time_usec: 0,
            }),
            networks: vec![pb::NetworkStats {
                interface: "eth0".into(),
                received_packets: 3,
                transmitted_packets: 4,
                received_bytes: 5,
                transmitted_bytes: 6,
                received_errors: 0,
                transmitted_errors: 0,
            }],
            ..pb::ContainerStats::default()
        },
        categories,
    );

    assert_eq!(stats.id, "unit");
    assert_eq!(stats.process.unwrap().current, 2);
    assert_eq!(stats.cpu.unwrap().usage_usec, 10);
    assert_eq!(stats.networks.unwrap()[0].interface, "eth0");
    assert_eq!(stats.memory, None);
    assert_eq!(stats.block_io, None);
    assert_eq!(stats.memory_events, None);
}

#[test]
fn linux_namespace_unshare_keeps_the_strict_empty_path() {
    let namespace = LinuxNamespace::unshare(NamespaceKind::Pid);
    let json = serde_json::to_value(namespace).expect("namespace json");

    assert_eq!(json["type"], "pid");
    assert_eq!(json["path"], "");
}

#[test]
fn network_config_builds_the_s9_rpc_sequence() {
    let config = NetworkConfig::new(
        Ipv4Addr::new(192, 168, 70, 2),
        24,
        Ipv4Addr::new(192, 168, 70, 1),
        "/run/container/container-0/rootfs",
        ["192.168.70.1", "8.8.8.8"],
    );

    let requests = config.requests("eth0");

    assert_eq!(
        requests.loopback_link,
        pb::IpLinkSetRequest {
            interface: "lo".into(),
            up: true,
            mtu: None,
        }
    );
    assert_eq!(requests.address.ipv4_address, "192.168.70.2/24");
    assert_eq!(requests.interface_link.interface, "eth0");
    assert!(requests.interface_link.up);
    assert_eq!(requests.default_route.ipv4_gateway, "192.168.70.1");
    assert_eq!(
        requests.dns.nameservers,
        vec!["192.168.70.1".to_string(), "8.8.8.8".to_string()]
    );
    assert_eq!(requests.dns.location, "/run/container/container-0/rootfs");
}

#[test]
fn rosetta_setup_builds_the_s7_rpc_sequence() {
    let requests = RosettaSetup::amd64().requests();

    assert_eq!(
        requests.mkdir,
        pb::MkdirRequest {
            path: "/run/rosetta".into(),
            all: true,
            perms: 0o755,
        }
    );
    assert_eq!(
        requests.mount,
        pb::MountRequest {
            r#type: "virtiofs".into(),
            source: "rosetta".into(),
            destination: "/run/rosetta".into(),
            options: Vec::new(),
        }
    );
    assert_eq!(
        requests.setup_emulator,
        pb::SetupEmulatorRequest {
            binary_path: "/run/rosetta/rosetta".into(),
            name: "x86_64".into(),
            r#type: "M".into(),
            offset: String::new(),
            magic: "\\x7fELF\\x02\\x01\\x01\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x00\\x02\\x00\\x3e\\x00".into(),
            mask: "\\xff\\xff\\xff\\xff\\xff\\xfe\\xfe\\x00\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xff\\xfe\\xff\\xff\\xff".into(),
            flags: "CF".into(),
        }
    );
}

#[test]
fn bundle_builds_the_s4_mount_and_config_write_requests() {
    let id = ContainerId::new("container-0").expect("container id");
    let bundle = ContainerBundle::for_id(&id);

    assert_eq!(
        bundle.mkdir_rootfs_request(0o755),
        pb::MkdirRequest {
            path: "/run/container/container-0/rootfs".into(),
            all: true,
            perms: 0o755,
        }
    );

    assert_eq!(
        bundle.mount_rootfs_request("/dev/vdb", ["rw"]),
        pb::MountRequest {
            r#type: "ext4".into(),
            source: "/dev/vdb".into(),
            destination: "/run/container/container-0/rootfs".into(),
            options: vec!["rw".into()],
        }
    );

    let spec = demo_spec("/run/container/container-0/rootfs");
    let request = bundle.write_config_request(&spec).expect("config request");
    assert_eq!(request.path, "/run/container/container-0/config.json");
    let data: serde_json::Value = serde_json::from_slice(&request.data).unwrap();
    assert_eq!(data["ociVersion"], "1.1.0");
    assert_eq!(data["root"]["path"], "/run/container/container-0/rootfs");
    assert_eq!(request.mode, 0o644);
    assert_eq!(
        request.flags,
        Some(pb::write_file_request::WriteFileFlags {
            create_parent_dirs: true,
            append: false,
            create_if_missing: true,
        })
    );
}

#[test]
fn process_create_builds_the_process_centric_request() {
    let container_id = ContainerId::new("container-0").expect("container id");
    let process_id = ProcessId::new("init").expect("process id");
    let stdio = ProcessStdio::new()
        .stdout(VsockPort::new(10_000))
        .stderr(VsockPort::new(10_001));

    let request = ProcessCreate::new(
        process_id.clone(),
        container_id.clone(),
        demo_spec("/run/container/container-0/rootfs"),
    )
    .stdio(stdio)
    .oci_runtime_path("/sbin/runc")
    .options([1, 2, 3])
    .into_request()
    .expect("process request");

    let configuration: serde_json::Value = serde_json::from_slice(&request.configuration).unwrap();
    assert_eq!(
        configuration["process"]["args"],
        json!(["sh", "-c", "echo hi"])
    );
    assert_eq!(
        request,
        pb::CreateProcessRequest {
            id: "init".into(),
            container_id: Some("container-0".into()),
            stdin: None,
            stdout: Some(10_000),
            stderr: Some(10_001),
            oci_runtime_path: Some("/sbin/runc".into()),
            configuration: request.configuration.clone(),
            options: Some(vec![1, 2, 3]),
        }
    );

    assert_eq!(
        ProcessCreate::start_request(&process_id, Some(&container_id)),
        pb::StartProcessRequest {
            id: "init".into(),
            container_id: Some("container-0".into()),
        }
    );
    assert_eq!(
        ProcessCreate::wait_request(&process_id, Some(&container_id)),
        pb::WaitProcessRequest {
            id: "init".into(),
            container_id: Some("container-0".into()),
        }
    );
    assert_eq!(
        ProcessCreate::kill_request(&process_id, Some(&container_id), 15),
        pb::KillProcessRequest {
            id: "init".into(),
            container_id: Some("container-0".into()),
            signal: 15,
        }
    );
    assert_eq!(
        ProcessCreate::delete_request(&process_id, Some(&container_id)),
        pb::DeleteProcessRequest {
            id: "init".into(),
            container_id: Some("container-0".into()),
        }
    );
    assert_eq!(
        ProcessCreate::resize_request(&process_id, Some(&container_id), 24, 80),
        pb::ResizeProcessRequest {
            id: "init".into(),
            container_id: Some("container-0".into()),
            rows: 24,
            columns: 80,
        }
    );
}

fn demo_spec(rootfs: &str) -> Spec {
    Spec {
        version: "1.1.0".to_owned(),
        process: Some(Process {
            args: vec!["sh".to_owned(), "-c".to_owned(), "echo hi".to_owned()],
            user: User {
                uid: 1000,
                gid: 1000,
                ..User::default()
            },
            ..Process::default()
        }),
        root: Some(Root {
            path: rootfs.to_owned(),
            readonly: false,
        }),
        ..Spec::default()
    }
}
