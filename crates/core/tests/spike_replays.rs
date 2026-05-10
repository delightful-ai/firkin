//! Real-VM replay tests for the S1-S9 spike acceptance proofs.
#![cfg(feature = "real-vm")]

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use firkin_core::{Container, Output, Rootfs};
use firkin_ext4::{BlockSize, FileSystemBuilder};
use firkin_oci::{Client, ImageBundle, Reference};
use firkin_types::{Platform, Size};
use firkin_vminitd_bytes::{VMEXEC_AARCH64, VMINITD_AARCH64};
use firkin_vmm::{KernelImage, Network, VirtualMachine, VmConfig};
use tokio::sync::Mutex;

static REAL_VM_REPLAY: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn s1_replay_boots_vz_vm_and_stops_cleanly() {
    let _guard = REAL_VM_REPLAY.lock().await;

    let output = run_busybox("s1-replay", Platform::linux_arm64(), [], &["/bin/true"]).await;

    assert_success(&output);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[tokio::test]
async fn s2_replay_vsock_tonic_round_trips_to_guest() {
    let _guard = REAL_VM_REPLAY.lock().await;
    let started = Instant::now();

    let output = run_busybox(
        "s2-replay",
        Platform::linux_arm64(),
        [],
        &["/bin/echo", "vsock-ok"],
    )
    .await;

    assert_success(&output);
    assert_eq!(output.stdout, b"vsock-ok\n");
    assert!(
        started.elapsed().as_secs() < 30,
        "container vsock/stdout round trip took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn s3_replay_vminitd_init_block_serves_grpc() {
    let _guard = REAL_VM_REPLAY.lock().await;

    let output = run_busybox(
        "s3-replay",
        Platform::linux_arm64(),
        [],
        &["/bin/sh", "-c", "printf vminitd-ok"],
    )
    .await;

    assert_success(&output);
    assert_eq!(output.stdout, b"vminitd-ok");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
async fn s4_replay_busybox_echo_round_trips_stdout() {
    let _guard = REAL_VM_REPLAY.lock().await;

    let output = run_busybox(
        "s4-replay",
        Platform::linux_arm64(),
        [],
        &["/bin/echo", "hello"],
    )
    .await;

    assert_success(&output);
    assert_eq!(output.stdout, b"hello\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn s5_replay_ext4_writer_handles_overlay_markers() {
    let mut fs = FileSystemBuilder::new(BlockSize::DEFAULT);
    fs.add_dir("/upper", 0o755).expect("upper dir");
    fs.add_file("/upper/kept", b"kept\n", 0o644)
        .expect("kept file");
    fs.add_whiteout("/upper/gone").expect("whiteout");
    fs.add_dir("/opaque", 0o755).expect("opaque dir");
    fs.add_opaque_dir("/opaque").expect("opaque marker");

    let image = tempfile::NamedTempFile::new().expect("image");
    let mut out = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(true)
        .open(image.path())
        .expect("open image");
    fs.write(&mut out).expect("write ext4 image");

    e2fsck_clean(image.path()).expect("e2fsck");
}

#[tokio::test]
async fn s6_replay_vmnet_shared_attachment_boots_with_eth0() {
    let _guard = REAL_VM_REPLAY.lock().await;

    let output = run_busybox(
        "s6-replay",
        Platform::linux_arm64(),
        [Network::vmnet_shared_subnet("192.168.126.0/24")],
        &["/bin/busybox", "ip", "link", "show", "eth0"],
    )
    .await;

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("eth0"), "{stdout}");
}

#[tokio::test]
async fn s7_replay_amd64_busybox_runs_through_rosetta() {
    let _guard = REAL_VM_REPLAY.lock().await;

    let output = run_busybox(
        "s7-replay",
        Platform::linux_amd64(),
        [],
        &["/bin/uname", "-m"],
    )
    .await;

    assert_success(&output);
    assert_eq!(output.stdout, b"x86_64\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn s8_replay_vminitd_elf_embed_synthesizes_init_block() {
    assert!(firkin_vminitd_bytes::embedded());

    let path = firkin_ext4::init_block::synthesize(VMINITD_AARCH64, VMEXEC_AARCH64)
        .expect("synthesize init.block");
    let metadata = std::fs::metadata(&path).expect("init.block metadata");

    assert!(metadata.len() >= Size::mib(8).as_bytes());
}

#[tokio::test]
async fn s9_replay_vmnet_guest_ip_and_gateway_reachability() {
    let _guard = REAL_VM_REPLAY.lock().await;
    let subnet = Ipv4Addr::new(192, 168, 127, 0);
    let gateway = Ipv4Addr::new(192, 168, 127, 1);
    let guest = Ipv4Addr::new(192, 168, 127, 2);

    let output = run_busybox(
        "s9-replay",
        Platform::linux_arm64(),
        [Network::vmnet_shared_subnet(format!("{subnet}/24"))],
        &[
            "/bin/sh",
            "-c",
            "/bin/busybox ip addr show eth0 && /bin/busybox ip route && /bin/busybox ping -c 1 -W 2 192.168.127.1",
        ],
    )
    .await;

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&guest.to_string()), "{stdout}");
    assert!(stdout.contains(&gateway.to_string()), "{stdout}");
    assert!(stdout.contains("1 packets received"), "{stdout}");
    assert!(output.stderr.is_empty());
}

#[tokio::test]
async fn s10_replay_snapshot_restore_keeps_vminitd_responsive() {
    let _guard = REAL_VM_REPLAY.lock().await;
    let snapshot_dir = tempfile::tempdir().expect("snapshot dir");
    let snapshot = snapshot_dir.path().join("s10.vzstate");

    let init_block = firkin_ext4::init_block::synthesize(VMINITD_AARCH64, VMEXEC_AARCH64)
        .expect("synthesize init.block");
    let config = VmConfig::builder()
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(init_block.clone())
        .build()
        .expect("vm config");
    let persisted_machine_identifier = config.machine_identifier().to_vec();
    let persisted_network_macs = config.network_macs().to_vec();

    let vm = VirtualMachine::new(config.clone())
        .boot()
        .await
        .expect("cold boot VM");
    assert_vminitd_responds(&vm, "/run/s10-before").await;
    vm.save_snapshot(&snapshot).await.expect("save snapshot");
    vm.stop().await.expect("stop cold VM");

    let restore_config = VmConfig::builder()
        .kernel(KernelImage::from_file(repo_root().join("bin/vmlinux")))
        .init_block(init_block)
        .machine_identifier(persisted_machine_identifier)
        .network_macs(persisted_network_macs)
        .build()
        .expect("restore vm config");
    let restored = VirtualMachine::new(restore_config)
        .boot_or_restore(&snapshot)
        .await
        .expect("restore VM");
    assert_vminitd_responds(&restored, "/run/s10-after").await;
    restored.stop().await.expect("stop restored VM");
}

async fn run_busybox(
    id: &str,
    platform: Platform,
    networks: impl IntoIterator<Item = Network>,
    command: &[&str],
) -> Output {
    let image = pull_busybox(platform).await;
    let mut builder = Container::builder(id)
        .expect("container builder")
        .image_config(image.config())
        .command(command.iter().copied())
        .rootfs(Rootfs::oci_bundle(image));
    for network in networks {
        builder = builder.network(network);
    }

    builder.output().await.expect("container output")
}

async fn pull_busybox(platform: Platform) -> ImageBundle {
    Client::builder()
        .cache_dir(replay_cache_dir())
        .platform(platform)
        .build()
        .expect("oci client")
        .pull(&Reference::parse("busybox").expect("busybox reference"))
        .await
        .expect("busybox pull")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn assert_vminitd_responds(vm: &VirtualMachine<firkin_vmm::Running>, path: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut client = loop {
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
            Ok(client) => break client,
            Err(error) if Instant::now() < deadline => {
                eprintln!("waiting for vminitd: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("connect vminitd: {error:?}"),
        }
    };

    client
        .mkdir(tonic::Request::new(
            firkin_vminitd_client::pb::MkdirRequest {
                path: path.to_owned(),
                all: false,
                perms: 0o755,
            },
        ))
        .await
        .expect("vminitd mkdir");
}

fn replay_cache_dir() -> PathBuf {
    std::env::var_os("FIRKIN_REPLAY_CACHE").map_or_else(
        || std::env::temp_dir().join("firkin-real-vm-replay-cache"),
        PathBuf::from,
    )
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn e2fsck_clean(path: &Path) -> Result<(), String> {
    let Some(bin) = tool_path("e2fsck") else {
        eprintln!("warn: e2fsck not found; skipping structural check");
        return Ok(());
    };
    let output = Command::new(&bin)
        .args(["-nf", path.to_str().expect("utf-8 image path")])
        .output()
        .map_err(|error| format!("failed to run {}: {error}", bin.display()))?;
    if output.status.success() {
        return Ok(());
    }

    Err(format!(
        "e2fsck exit={:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn tool_path(name: &str) -> Option<PathBuf> {
    for prefix in [
        "/opt/homebrew/opt/e2fsprogs/sbin",
        "/usr/local/opt/e2fsprogs/sbin",
        "/usr/sbin",
        "/sbin",
    ] {
        let path = Path::new(prefix).join(name);
        if path.exists() {
            return Some(path);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|path| path.exists())
    })
}
