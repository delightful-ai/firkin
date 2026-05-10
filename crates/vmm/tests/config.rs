//! VM configuration validation tests.

use std::io::{Read, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::process::Command;
use std::time::Duration;

use firkin_types::{Size, VirtiofsTag, VmId, VsockPort};
use firkin_vmm::{
    BlankDiskImage, DiskImageConversion, DiskImageFormat, Error, HostArch, KernelImage, Network,
    Preflight, Running, VirtualMachine, VmConfig, VmPhase, VmStatistics, VsockListener,
    convert_disk_image, create_blank_disk_image, preflight, signing,
};

#[test]
fn default_config_matches_the_vm_surface_spec() {
    let config = VmConfig::default();

    assert_eq!(config.cpus(), NonZeroU32::new(4).expect("nonzero"));
    assert_eq!(config.memory(), Size::gib(1));
    assert_eq!(config.networks(), &[Network::Nat]);
    assert_eq!(config.network_macs().len(), 1);
    assert_eq!(config.init_block(), None);
    assert!(!config.rosetta_enabled());
    assert!(!config.nested_virtualization());
}

#[test]
fn rosetta_is_explicit_opt_in() {
    let config = VmConfig::builder().rosetta(true).build().expect("config");

    assert!(config.rosetta_enabled());
}

#[test]
fn build_rejects_memory_below_the_vz_floor() {
    let error = VmConfig::builder()
        .memory(Size::mib(127))
        .build()
        .expect_err("invalid memory");

    assert_eq!(
        error,
        Error::InvalidConfig {
            reason: "memory must be >= 128 MiB".into()
        }
    );
}

#[test]
fn build_rejects_duplicate_virtiofs_tags() {
    let tag = VirtiofsTag::new("shared").expect("tag");
    let error = VmConfig::builder()
        .virtiofs_share(tag.clone(), "/tmp/a")
        .virtiofs_share(tag, "/tmp/b")
        .build()
        .expect_err("duplicate tag");

    assert_eq!(
        error,
        Error::InvalidConfig {
            reason: "virtiofs tag shared used twice".into()
        }
    );
}

#[test]
fn block_device_returns_typed_handles_in_declaration_order() {
    let first = tempfile::NamedTempFile::new().expect("first");
    let second = tempfile::NamedTempFile::new().expect("second");

    let (builder, first_id) = VmConfig::builder().block_device(first.path());
    let (builder, second_id) = builder.block_device(second.path());
    let config = builder.build().expect("config");

    assert_ne!(first_id, second_id);
    assert_eq!(first_id.slot().get(), 1);
    assert_eq!(second_id.slot().get(), 2);
    assert_eq!(config.block_devices().len(), 2);
}

#[test]
fn block_device_records_raw_writable_local_disk_image() {
    let image = tempfile::NamedTempFile::new().expect("image");

    let (builder, _id) = VmConfig::builder().block_device(image.path());
    let config = builder.build().expect("config");
    let device = &config.block_devices()[0];

    assert_eq!(device.path(), image.path());
    assert_eq!(device.disk_image_format(), DiskImageFormat::Raw);
    assert!(!device.read_only());
}

#[test]
fn asif_disk_image_records_asif_format() {
    let image = tempfile::Builder::new()
        .suffix(".asif")
        .tempfile()
        .expect("asif image");

    let (builder, id) = VmConfig::builder().asif_disk_image(image.path());
    let config = builder.build().expect("config");
    let device = &config.block_devices()[0];

    assert_eq!(id.slot().get(), 1);
    assert_eq!(device.path(), image.path());
    assert_eq!(device.disk_image_format(), DiskImageFormat::Asif);
    assert!(!device.read_only());
}

#[test]
fn readonly_asif_disk_image_records_readonly_flag() {
    let image = tempfile::Builder::new()
        .suffix(".asif")
        .tempfile()
        .expect("asif image");

    let (builder, _id) = VmConfig::builder().readonly_asif_disk_image(image.path());
    let config = builder.build().expect("config");
    let device = &config.block_devices()[0];

    assert_eq!(device.disk_image_format(), DiskImageFormat::Asif);
    assert!(device.read_only());
}

#[test]
fn create_blank_raw_disk_image_uses_exact_size() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("pod-store.raw");

    create_blank_disk_image(&BlankDiskImage::new(
        &path,
        Size::mib(2),
        DiskImageFormat::Raw,
    ))
    .expect("create raw image");

    assert_eq!(
        std::fs::metadata(path).expect("raw metadata").len(),
        Size::mib(2).as_bytes()
    );
}

#[test]
fn asif_disk_image_conversion_records_paths_and_format() {
    let conversion = DiskImageConversion::asif("/tmp/source.raw", "/tmp/dest.asif");

    assert_eq!(conversion.source(), std::path::Path::new("/tmp/source.raw"));
    assert_eq!(
        conversion.destination(),
        std::path::Path::new("/tmp/dest.asif")
    );
    assert_eq!(conversion.format(), DiskImageFormat::Asif);
}

#[test]
#[cfg(target_os = "macos")]
fn convert_raw_disk_image_to_asif_preserves_block_contents() {
    if !diskutil_supports_asif() {
        eprintln!("warn: diskutil does not advertise ASIF support; skipping ASIF conversion probe");
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    let source = dir.path().join("source.raw");
    let asif = dir.path().join("converted.asif");
    let marker = b"firkin-asif-conversion";

    {
        let mut raw = std::fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&source)
            .expect("raw source");
        raw.set_len(Size::mib(16).as_bytes())
            .expect("size raw source");
        let mut sector = [0u8; 512];
        sector[..marker.len()].copy_from_slice(marker);
        raw.seek(SeekFrom::Start(0)).expect("seek raw");
        raw.write_all(&sector).expect("write marker sector");
        raw.flush().expect("flush raw");
    }

    convert_disk_image(&DiskImageConversion::asif(&source, &asif)).expect("convert to asif");

    let attached = AttachedDisk::attach(&asif);
    let raw_device = format!("/dev/r{}", attached.device);
    let mut readback = [0u8; 512];
    std::fs::File::open(&raw_device)
        .unwrap_or_else(|error| panic!("open {raw_device}: {error}"))
        .read_exact(&mut readback)
        .unwrap_or_else(|error| panic!("read {raw_device}: {error}"));

    assert_eq!(&readback[..marker.len()], marker);
}

#[cfg(target_os = "macos")]
fn diskutil_supports_asif() -> bool {
    let Ok(output) = Command::new("diskutil")
        .args(["image", "create", "from", "--help"])
        .output()
    else {
        return false;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout.contains("ASIF") || stderr.contains("ASIF")
}

#[cfg(target_os = "macos")]
struct AttachedDisk {
    device: String,
}

#[cfg(target_os = "macos")]
impl AttachedDisk {
    fn attach(path: &std::path::Path) -> Self {
        let output = Command::new("diskutil")
            .args(["image", "attach", "--plist", "--noMount"])
            .arg(path)
            .output()
            .expect("run diskutil image attach");
        assert!(
            output.status.success(),
            "diskutil image attach failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let plist = tempfile::NamedTempFile::new().expect("attach plist");
        std::fs::write(plist.path(), &output.stdout).expect("write attach plist");
        let device = Command::new("plutil")
            .args(["-extract", "system-entities.0.dev-entry", "raw", "-o", "-"])
            .arg(plist.path())
            .output()
            .expect("run plutil");
        assert!(
            device.status.success(),
            "plutil failed: stdout={} stderr={}",
            String::from_utf8_lossy(&device.stdout),
            String::from_utf8_lossy(&device.stderr)
        );

        Self {
            device: String::from_utf8(device.stdout)
                .expect("utf8 device")
                .trim()
                .to_owned(),
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for AttachedDisk {
    fn drop(&mut self) {
        let _ = Command::new("diskutil")
            .args(["eject", self.device.as_str()])
            .output();
    }
}

#[test]
fn build_rejects_missing_block_device_path() {
    let (builder, _id) = VmConfig::builder().block_device("/definitely/missing/firkin.img");
    let error = builder.build().expect_err("missing block device");

    assert_eq!(
        error,
        Error::InvalidConfig {
            reason: "block_device: /definitely/missing/firkin.img not accessible".into()
        }
    );
}

#[test]
fn init_block_path_is_validated_when_explicit() {
    let init_block = tempfile::NamedTempFile::new().expect("init block");
    let config = VmConfig::builder()
        .init_block(init_block.path())
        .build()
        .expect("config");

    assert_eq!(config.init_block(), Some(init_block.path()));

    let error = VmConfig::builder()
        .init_block("/definitely/missing/init.block")
        .build()
        .expect_err("missing init block");
    assert_eq!(
        error,
        Error::InvalidConfig {
            reason: "init_block: /definitely/missing/init.block not accessible".into()
        }
    );
}

#[test]
fn explicit_kernel_path_is_validated() {
    let kernel = tempfile::NamedTempFile::new().expect("kernel");
    let config = VmConfig::builder()
        .kernel(KernelImage::from_file(kernel.path()))
        .build()
        .expect("config");

    assert_eq!(config.kernel(), &KernelImage::from_file(kernel.path()));

    let error = VmConfig::builder()
        .kernel(KernelImage::from_file("/definitely/missing/vmlinux"))
        .build()
        .expect_err("missing kernel");
    assert_eq!(
        error,
        Error::InvalidConfig {
            reason: "kernel: /definitely/missing/vmlinux not accessible".into()
        }
    );
}

#[test]
fn build_rejects_too_many_networks() {
    let mut builder = VmConfig::builder().networks([]);
    for _ in 0..9 {
        builder = builder.network(Network::Nat);
    }

    let error = builder.build().expect_err("too many networks");
    assert_eq!(
        error,
        Error::InvalidConfig {
            reason: "at most 8 network attachments are supported".into()
        }
    );
}

#[test]
fn build_rejects_invalid_vmnet_subnet() {
    let error = VmConfig::builder()
        .networks([Network::vmnet_shared_subnet("192.168.64.0/31")])
        .build()
        .expect_err("bad subnet");

    assert_eq!(
        error,
        Error::InvalidConfig {
            reason:
                "vmnet subnet `192.168.64.0/31` must leave room for gateway and guest addresses"
                    .into()
        }
    );
}

#[test]
fn network_macs_are_persistable_and_validated() {
    let config = VmConfig::builder()
        .networks([
            Network::Nat,
            Network::vmnet_shared_subnet("192.168.64.0/24"),
        ])
        .network_macs(["02:00:00:00:00:10", "02:00:00:00:00:11"])
        .build()
        .expect("config");

    assert_eq!(
        config.network_macs(),
        ["02:00:00:00:00:10", "02:00:00:00:00:11"]
    );

    let count_error = VmConfig::builder()
        .networks([Network::Nat, Network::Nat])
        .network_macs(["02:00:00:00:00:10"])
        .build()
        .expect_err("mac count");
    assert_eq!(
        count_error,
        Error::InvalidConfig {
            reason: "network MAC count 1 does not match network count 2".into()
        }
    );

    let format_error = VmConfig::builder()
        .network_macs(["not-a-mac"])
        .build()
        .expect_err("mac format");
    assert_eq!(
        format_error,
        Error::InvalidConfig {
            reason: "network MAC address `not-a-mac` is invalid".into()
        }
    );
}

#[test]
fn not_booted_virtual_machine_wraps_config_without_resources() {
    let config = VmConfig::default();
    let vm = VirtualMachine::new(config.clone());

    assert_eq!(vm.config(), &config);
}

#[test]
fn boot_future_is_send_for_multithreaded_library_consumers() {
    fn assert_send<T: Send>(_value: T) {}

    let vm = VirtualMachine::new(VmConfig::default());
    assert_send(vm.boot());
}

#[test]
fn preflight_reports_public_capability_shape() {
    let info = preflight().expect("preflight");

    assert!(matches!(
        info.architecture(),
        HostArch::Arm64 | HostArch::X86_64
    ));
    assert!(!info.macos_version().is_empty());

    let current_exe = std::env::current_exe().expect("current exe");
    let signing = signing::codesign_check(&current_exe).expect("codesign check");
    assert_eq!(info.codesigned(), Some(signing.codesigned()));
    assert_eq!(
        info.has_virtualization_entitlement(),
        Some(signing.has_virtualization_entitlement())
    );

    let explicit = Preflight::new(
        "26.0".to_owned(),
        HostArch::Arm64,
        true,
        true,
        Some(true),
        Some(true),
    );
    assert_eq!(explicit.macos_version(), "26.0");
    assert!(explicit.nested_virtualization_supported());
    assert!(explicit.rosetta_available());
}

#[tokio::test]
async fn boot_requires_core_to_resolve_kernel_and_init_block() {
    let vm = VirtualMachine::new(VmConfig::default());
    let error = vm.boot().await.expect_err("unresolved boot inputs");

    assert_eq!(
        error,
        Error::InvalidConfig {
            reason: "VZ boot requires a resolved kernel file".into()
        }
    );
}

#[allow(dead_code)]
fn running_virtual_machine_surface_matches_the_vm_spec(vm: &VirtualMachine<Running>) {
    let _: &VmId = vm.id();
    let _: NonZeroU32 = vm.cpus();
    let _: Size = vm.memory();
    let _: &VmConfig = vm.config();
    let _: bool = vm.is_paused();
    let _: VmPhase = vm.state();
    let _: Result<VsockListener, Error> = vm.listen(VsockPort::new(2048));
    let _: VmStatistics = VmStatistics::new(vm.cpus(), vm.memory(), vm.state());
    std::mem::drop(vm.dial(VsockPort::new(2048)));
    std::mem::drop(vm.pause());
    std::mem::drop(vm.resume());
    std::mem::drop(vm.statistics());
}

#[allow(dead_code)]
fn running_virtual_machine_owned_surface_matches_the_vm_spec(vm: VirtualMachine<Running>) {
    std::mem::drop(vm.stop_with_grace(Duration::from_secs(1)));
}
