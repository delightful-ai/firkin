#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! User-facing container orchestration surface.
#[cfg(test)]
#[allow(unused_imports)]
use firkin_oci::{
    ImageBundle, ImageConfig, LinuxCapabilities as OciLinuxCapabilities,
    PosixRlimit as OciPosixRlimit, Process as OciProcess, User as OciUser,
};
#[allow(unused_imports)]
pub use firkin_oci::{
    LinuxSeccompAction as SeccompAction, LinuxSeccompArch as SeccompArch,
    LinuxSeccompArg as SeccompArgRule, LinuxSeccompFlag as SeccompFlag,
    LinuxSeccompOperator as SeccompOp, LinuxSeccompProfile as Seccomp,
    LinuxSyscall as SeccompSyscallRule, Mount,
};
#[cfg(test)]
#[allow(unused_imports)]
use firkin_types::{ContainerId, VirtiofsTag};
#[allow(unused_imports)]
use firkin_types::{Size, VsockPort};
#[cfg(test)]
#[allow(unused_imports)]
use firkin_vmm::{BootLog, KernelImage};
#[allow(unused_imports)]
use firkin_vmm::{Running, VirtualMachine};
pub use pod::*;
#[allow(unused_imports)]
use std::ffi::OsString;
#[cfg(test)]
#[allow(unused_imports)]
use std::io as io_import;
#[cfg(test)]
#[allow(unused_imports)]
use std::net::IpAddr;
#[cfg(test)]
#[allow(unused_imports)]
use std::num::NonZeroU32;
#[allow(unused_imports)]
use std::os::unix::fs::PermissionsExt;
#[cfg(test)]
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[cfg(all(feature = "snapshot", target_os = "macos"))]
#[cfg(test)]
#[allow(unused_imports)]
use std::process::Command;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(crate) mod pod;
pub(crate) mod sealed {
    pub trait Sealed {}
}
impl sealed::Sealed for VirtualMachine<Running> {}
pub(crate) fn os_strings_to_strings(
    values: &[OsString],
    error: impl Fn(usize) -> Error,
) -> Result<Vec<String>> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| value.clone().into_string().map_err(|_| error(index)))
        .collect()
}
pub(crate) fn prepare_fixed_stdin(
    vm: &VirtualMachine<Running>,
    config: Stdio,
    port: VsockPort,
    operation: &'static str,
) -> Result<FixedStreamTask<ChildStdin>> {
    match config {
        Stdio::Null => Ok((false, None)),
        Stdio::Piped => {
            let listener = vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error(operation))?;
            Ok((true, Some(tokio::spawn(accept_child_stdin(listener)))))
        }
        Stdio::Inherit => {
            let listener = vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error(operation))?;
            detach_stdio_relay(tokio::spawn(relay_inherited_stdin(listener)));
            Ok((true, None))
        }
    }
}
pub(crate) fn prepare_fixed_stdout(
    vm: &VirtualMachine<Running>,
    config: Stdio,
    port: VsockPort,
    operation: &'static str,
) -> Result<FixedStreamTask<ChildStdout>> {
    match config {
        Stdio::Null => Ok((false, None)),
        Stdio::Piped => {
            let listener = vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error(operation))?;
            Ok((true, Some(tokio::spawn(accept_child_stdout(listener)))))
        }
        Stdio::Inherit => {
            let listener = vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error(operation))?;
            detach_stdio_relay(tokio::spawn(relay_inherited_stdout(listener)));
            Ok((true, None))
        }
    }
}
pub(crate) fn prepare_fixed_stderr(
    vm: &VirtualMachine<Running>,
    config: Stdio,
    port: VsockPort,
    operation: &'static str,
) -> Result<FixedStreamTask<ChildStderr>> {
    match config {
        Stdio::Null => Ok((false, None)),
        Stdio::Piped => {
            let listener = vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error(operation))?;
            Ok((true, Some(tokio::spawn(accept_child_stderr(listener)))))
        }
        Stdio::Inherit => {
            let listener = vm
                .listen_reserved_port(port)
                .map_err(runtime_vmm_error(operation))?;
            detach_stdio_relay(tokio::spawn(relay_inherited_stderr(listener)));
            Ok((true, None))
        }
    }
}
pub(crate) async fn await_stdin(
    task: tokio::task::JoinHandle<Result<ChildStdin>>,
    operation: &'static str,
) -> Result<ChildStdin> {
    tokio::time::timeout(STDIO_CAPTURE_TIMEOUT, task)
        .await
        .map_err(|_| Error::RuntimeOperation {
            operation,
            reason: format!("timed out after {}s", STDIO_CAPTURE_TIMEOUT.as_secs()),
        })?
        .map_err(|error| Error::RuntimeOperation {
            operation,
            reason: error.to_string(),
        })?
}
pub(crate) async fn await_stdout(
    task: tokio::task::JoinHandle<Result<ChildStdout>>,
    operation: &'static str,
) -> Result<ChildStdout> {
    tokio::time::timeout(STDIO_CAPTURE_TIMEOUT, task)
        .await
        .map_err(|_| Error::RuntimeOperation {
            operation,
            reason: format!("timed out after {}s", STDIO_CAPTURE_TIMEOUT.as_secs()),
        })?
        .map_err(|error| Error::RuntimeOperation {
            operation,
            reason: error.to_string(),
        })?
}
pub(crate) async fn await_stderr(
    task: tokio::task::JoinHandle<Result<ChildStderr>>,
    operation: &'static str,
) -> Result<ChildStderr> {
    tokio::time::timeout(STDIO_CAPTURE_TIMEOUT, task)
        .await
        .map_err(|_| Error::RuntimeOperation {
            operation,
            reason: format!("timed out after {}s", STDIO_CAPTURE_TIMEOUT.as_secs()),
        })?
        .map_err(|error| Error::RuntimeOperation {
            operation,
            reason: error.to_string(),
        })?
}
pub(crate) async fn read_stdout_optional(
    task: Option<tokio::task::JoinHandle<Result<ChildStdout>>>,
    operation: &'static str,
) -> Result<Vec<u8>> {
    let Some(task) = task else {
        return Ok(Vec::new());
    };
    let mut stdout = await_stdout(task, operation).await?;
    let mut output = Vec::new();
    stdout
        .read_to_end(&mut output)
        .await
        .map_err(io_runtime_error(operation))?;
    Ok(output)
}
pub(crate) async fn read_stderr_optional(
    task: Option<tokio::task::JoinHandle<Result<ChildStderr>>>,
    operation: &'static str,
) -> Result<Vec<u8>> {
    let Some(task) = task else {
        return Ok(Vec::new());
    };
    let mut stderr = await_stderr(task, operation).await?;
    let mut output = Vec::new();
    stderr
        .read_to_end(&mut output)
        .await
        .map_err(io_runtime_error(operation))?;
    Ok(output)
}
pub(crate) async fn read_pty_optional(
    pty: Option<Pty>,
    task: Option<tokio::task::JoinHandle<Result<Pty>>>,
    operation: &'static str,
) -> Result<Vec<u8>> {
    let mut pty = if let Some(pty) = pty {
        pty
    } else {
        let Some(task) = task else {
            return Ok(Vec::new());
        };
        await_pty(task, operation).await?
    };
    let mut output = Vec::new();
    pty.read_to_end(&mut output)
        .await
        .map_err(io_runtime_error(operation))?;
    Ok(output)
}
pub(crate) async fn await_pty(
    task: tokio::task::JoinHandle<Result<Pty>>,
    operation: &'static str,
) -> Result<Pty> {
    tokio::time::timeout(STDIO_CAPTURE_TIMEOUT, task)
        .await
        .map_err(|_| Error::RuntimeOperation {
            operation,
            reason: format!("timed out after {}s", STDIO_CAPTURE_TIMEOUT.as_secs()),
        })?
        .map_err(|error| Error::RuntimeOperation {
            operation,
            reason: error.to_string(),
        })?
}
pub(crate) fn runtime_rpc_error(operation: &'static str) -> impl Fn(tonic::Status) -> Error {
    move |error| Error::RuntimeOperation {
        operation,
        reason: error.to_string(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use firkin_ext4::LayerCompression;
    use firkin_oci::{Digest, ImageBundle, ImageConfig, Layer, MediaType, Reference};
    use firkin_types::Platform;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    #[test]
    fn vmnet_create_failure_is_transient_boot_error() {
        let error = firkin_vmm::Error::UnclassifiedVz {
            reason: "vmnet_network_create failed: status=1001 (VMNET_FAILURE)".to_owned(),
        };
        assert!(is_transient_vmnet_boot_error(&error));
    }
    #[test]
    fn vmnet_sharing_busy_is_transient_boot_error() {
        let error = firkin_vmm::Error::UnclassifiedVz {
            reason: "vmnet_network_create failed: status=1009 (VMNET_SHARING_SERVICE_BUSY)"
                .to_owned(),
        };
        assert!(is_transient_vmnet_boot_error(&error));
    }
    #[test]
    fn unrelated_vz_error_is_not_transient_boot_error() {
        let error = firkin_vmm::Error::UnclassifiedVz {
            reason:
                "vmnet_network_configuration_create failed: status=1003 (VMNET_INVALID_ARGUMENT)"
                    .to_owned(),
        };
        assert!(!is_transient_vmnet_boot_error(&error));
    }
    #[test]
    fn stages_oci_bundle_rootfs_to_ext4_image() {
        let layer = tempfile::NamedTempFile::new().unwrap();
        write_layer(layer.path(), "bin/hello", b"hello\n");
        let bundle_root = tempfile::tempdir().unwrap();
        let bundle = ImageBundle::new(
            bundle_root.path(),
            Reference::parse("busybox").unwrap(),
            Digest::new("sha256:manifest"),
            Platform::linux_arm64(),
            ImageConfig::default(),
            vec![
                Layer::new(
                    layer.path(),
                    Digest::new("sha256:layer"),
                    Digest::new("sha256:diff"),
                    Size::bytes(100),
                    MediaType::new(MediaType::TAR),
                )
                .unwrap(),
            ],
        );
        let builder = Container::builder("worker")
            .unwrap()
            .rootfs(Rootfs::oci_bundle(bundle));
        let dest = tempfile::NamedTempFile::new().unwrap();
        let staged = builder.stage_rootfs_to(dest.path()).unwrap();
        assert_eq!(staged.path(), dest.path());
        assert_debugfs_cat(dest.path(), "/bin/hello", "hello\n");
    }
    #[cfg(feature = "snapshot")]
    #[test]
    fn restored_rootfs_copy_reports_stage_method_and_bytes() {
        let source = tempfile::NamedTempFile::new().unwrap();
        let dest_dir = tempfile::tempdir().unwrap();
        let dest = dest_dir.path().join("rootfs.ext4");
        std::fs::write(source.path(), b"restored-rootfs").unwrap();
        let report = copy_restored_rootfs(source.path(), &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"restored-rootfs");
        assert_eq!(report.source_bytes(), "restored-rootfs".len() as u64);
        assert!(report.elapsed() >= Duration::ZERO);
        #[cfg(target_os = "macos")]
        assert_eq!(report.method(), RestoredRootfsStageMethod::Clone);
    }
    #[cfg(feature = "snapshot")]
    #[test]
    fn restored_rootfs_from_snapshot_state_uses_persisted_staged_rootfs() {
        let source_staging = tempfile::tempdir().unwrap();
        let source_rootfs = source_staging.path().join("worker-rootfs.ext4");
        let dest = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(&source_rootfs, b"persisted-rootfs").unwrap();
        let state =
            ContainerSnapshotState::new(source_staging.path(), vec![1, 2, 3], Vec::<String>::new());
        let builder = Container::builder("worker")
            .unwrap()
            .rootfs(Rootfs::ext4_image("/tmp/not-the-persisted-rootfs.ext4"));
        let (rootfs, _report) = builder
            .stage_restored_rootfs_from_snapshot_state_to(&state, dest.path())
            .unwrap();
        assert_eq!(rootfs.path(), dest.path());
        assert_eq!(std::fs::read(dest.path()).unwrap(), b"persisted-rootfs");
    }
    #[cfg(feature = "snapshot")]
    #[test]
    fn repeated_resolve_init_block_uses_process_cache() {
        let first = resolve_init_block().unwrap();
        let started = std::time::Instant::now();
        let second = resolve_init_block().unwrap();
        assert_eq!(first.path(), second.path());
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "second init.block resolve should not rehash embedded runtime bytes"
        );
    }
    #[test]
    fn exec_config_generates_process_spec_for_existing_container() {
        let container_id = ContainerId::new("unit").unwrap();
        let spec = ExecConfig::builder()
            .env("A", "B")
            .working_dir("/tmp")
            .command(["/bin/echo", "exec"])
            .build()
            .runtime_spec(&container_id, false)
            .unwrap();
        let process = spec.process.expect("process");
        assert_eq!(process.args, ["/bin/echo", "exec"]);
        assert_eq!(process.env, ["A=B"]);
        assert_eq!(process.cwd, "/tmp");
        assert_eq!(spec.root.expect("root").path, "/run/container/unit/rootfs");
        assert_eq!(spec.hostname, "unit");
    }
    #[test]
    fn exec_config_preserves_process_policy_knobs() {
        let container_id = ContainerId::new("unit").unwrap();
        let spec = ExecConfig::builder()
            .command(["/bin/echo", "exec"])
            .no_new_privileges(true)
            .capabilities(LinuxCapabilities::single_set(vec![Capability::Chown]))
            .rlimit(LinuxRlimit::new(RlimitKind::OpenFiles, 2048, 1024))
            .selinux_label("system_u:system_r:container_t:s0")
            .apparmor_profile("firkin-profile")
            .build()
            .runtime_spec(&container_id, false)
            .unwrap();
        let process = spec.process.expect("process");
        assert!(process.no_new_privileges);
        assert_eq!(
            process.capabilities.expect("capabilities").effective,
            Some(vec!["CAP_CHOWN".to_owned()])
        );
        assert_eq!(process.rlimits[0].kind, "RLIMIT_NOFILE");
        assert_eq!(process.rlimits[0].soft, 1024);
        assert_eq!(process.rlimits[0].hard, 2048);
        assert_eq!(process.selinux_label, "system_u:system_r:container_t:s0");
        assert_eq!(process.apparmor_profile, "firkin-profile");
    }
    #[test]
    fn materializer_rewrites_hardlinks_as_relative_symlinks() {
        let layer = tempfile::NamedTempFile::new().unwrap();
        write_hardlink_layer(layer.path());
        let rewritten =
            crate::pod::rewrite_materialization_layer_archive(layer.path(), LayerCompression::None)
                .unwrap();
        let file = std::fs::File::open(rewritten.path()).unwrap();
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        let mut saw_busybox = false;
        let mut sh_target = None;
        let mut env_target = None;
        for entry in archive.entries().unwrap() {
            let entry = entry.unwrap();
            let path = entry.path().unwrap().into_owned();
            if path == Path::new("bin/[") {
                assert!(entry.header().entry_type().is_file());
                saw_busybox = true;
            } else if path == Path::new("bin/sh") {
                assert!(entry.header().entry_type().is_symlink());
                sh_target = Some(entry.link_name().unwrap().unwrap().into_owned());
            } else if path == Path::new("usr/bin/env") {
                assert!(entry.header().entry_type().is_symlink());
                env_target = Some(entry.link_name().unwrap().unwrap().into_owned());
            }
        }
        assert!(saw_busybox);
        assert_eq!(sh_target.as_deref(), Some(Path::new("[")));
        assert_eq!(env_target.as_deref(), Some(Path::new("../../bin/[")));
    }
    #[test]
    fn process_id_inputs_are_validated_before_exec() {
        let error = "bad/id".into_process_id().expect_err("invalid process id");
        assert!(error.to_string().contains("forbidden characters"));
    }
    #[test]
    fn prepares_implicit_vm_with_kernel_init_block_and_rootfs_device() {
        if !firkin_vminitd_bytes::embedded() {
            return;
        }
        let rootfs = tempfile::NamedTempFile::new().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let builder = Container::builder("boot-plan")
            .unwrap()
            .memory(Size::mib(512))
            .rootfs(Rootfs::raw_block(rootfs.path()));
        let prepared = builder.prepare_implicit_vm_in(staging.path()).unwrap();
        assert_eq!(prepared.config().memory(), Size::mib(512));
        assert_eq!(
            prepared.config().kernel(),
            &KernelImage::from_file(resolve_kernel().unwrap())
        );
        assert_eq!(
            prepared.config().init_block(),
            Some(prepared.init_block().path())
        );
        assert!(prepared.init_block().path().exists());
        assert_eq!(prepared.rootfs().path(), rootfs.path());
        assert_eq!(prepared.rootfs_device().slot().get(), 1);
        assert_eq!(prepared.writable_layer_device(), None);
        assert_eq!(prepared.config().block_devices()[0].path(), rootfs.path());
        assert!(!prepared.config().rosetta_enabled());
    }
    #[test]
    fn prepares_implicit_vm_with_writable_layer_block_device() {
        if !firkin_vminitd_bytes::embedded() {
            return;
        }
        let rootfs = tempfile::NamedTempFile::new().unwrap();
        let writable = tempfile::NamedTempFile::new().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let builder = Container::builder("writable-plan")
            .unwrap()
            .writable_layer(Mount::block(
                "ext4",
                writable.path().display().to_string(),
                "/",
            ))
            .rootfs(Rootfs::raw_block(rootfs.path()));
        let prepared = builder.prepare_implicit_vm_in(staging.path()).unwrap();
        let config = prepared.config();
        assert_eq!(prepared.rootfs_device().slot().get(), 1);
        assert_eq!(prepared.writable_layer_device().unwrap().slot().get(), 2);
        assert_eq!(config.block_devices()[0].path(), rootfs.path());
        assert_eq!(config.block_devices()[1].path(), writable.path());
    }
    #[cfg(feature = "snapshot")]
    #[test]
    fn restored_implicit_vm_uses_private_staged_rootfs_path() {
        if !firkin_vminitd_bytes::embedded() {
            return;
        }
        let rootfs = tempfile::NamedTempFile::new().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let builder = Container::builder("restore-plan")
            .unwrap()
            .memory(Size::mib(512))
            .rootfs(Rootfs::raw_block(rootfs.path()));
        let prepared = builder
            .prepare_restored_implicit_vm_in(
                staging.path(),
                vec![1, 2, 3],
                vec!["02:00:00:00:00:01".to_owned()],
            )
            .unwrap();
        let staged = staging.path().join("restore-plan-rootfs.ext4");
        assert_eq!(prepared.rootfs().path(), staged);
        assert_eq!(prepared.config().block_devices()[0].path(), staged);
    }
    #[test]
    fn amd64_oci_bundle_requests_rosetta_for_implicit_vm() {
        if !firkin_vminitd_bytes::embedded() {
            return;
        }
        let layer = tempfile::NamedTempFile::new().unwrap();
        write_layer(layer.path(), "bin/uname", b"fake amd64 uname\n");
        let bundle_root = tempfile::tempdir().unwrap();
        let bundle = ImageBundle::new(
            bundle_root.path(),
            Reference::parse("busybox").unwrap(),
            Digest::new("sha256:manifest"),
            Platform::linux_amd64(),
            ImageConfig::default(),
            vec![
                Layer::new(
                    layer.path(),
                    Digest::new("sha256:layer"),
                    Digest::new("sha256:diff"),
                    Size::bytes(100),
                    MediaType::new(MediaType::TAR),
                )
                .unwrap(),
            ],
        );
        let staging = tempfile::tempdir().unwrap();
        let builder = Container::builder("rosetta-plan")
            .unwrap()
            .rootfs(Rootfs::oci_bundle(bundle));
        let prepared = builder.prepare_implicit_vm_in(staging.path()).unwrap();
        assert!(prepared.config().rosetta_enabled());
    }
    #[test]
    fn explicit_rosetta_request_enables_implicit_vm() {
        if !firkin_vminitd_bytes::embedded() {
            return;
        }
        let rootfs = tempfile::NamedTempFile::new().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let builder = Container::builder("explicit-rosetta")
            .unwrap()
            .rosetta(true)
            .rootfs(Rootfs::raw_block(rootfs.path()));
        let prepared = builder.prepare_implicit_vm_in(staging.path()).unwrap();
        assert!(prepared.config().rosetta_enabled());
    }
    #[test]
    fn prepares_implicit_vm_with_vm_level_knobs() {
        if !firkin_vminitd_bytes::embedded() {
            return;
        }
        let rootfs = tempfile::NamedTempFile::new().unwrap();
        let kernel = tempfile::NamedTempFile::new().unwrap();
        let boot_log = tempfile::NamedTempFile::new().unwrap();
        let share = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let builder = Container::builder("vm-knobs")
            .unwrap()
            .cpus(NonZeroU32::new(2).unwrap())
            .memory(Size::mib(768))
            .virtiofs_share(VirtiofsTag::new("shared").unwrap(), share.path())
            .nested_virtualization(true)
            .boot_log(BootLog::File(boot_log.path().to_path_buf()))
            .kernel(KernelImage::from_file(kernel.path()))
            .cmdline_extra("debug=1")
            .rootfs(Rootfs::raw_block(rootfs.path()));
        let prepared = builder.prepare_implicit_vm_in(staging.path()).unwrap();
        let config = prepared.config();
        assert_eq!(config.cpus().get(), 2);
        assert_eq!(config.memory(), Size::mib(768));
        assert_eq!(config.virtiofs_shares().len(), 1);
        assert_eq!(config.virtiofs_shares()[0].tag().as_str(), "shared");
        assert_eq!(config.virtiofs_shares()[0].host_path(), share.path());
        assert!(config.nested_virtualization());
        assert_eq!(
            config.boot_log(),
            &BootLog::File(boot_log.path().to_path_buf())
        );
        assert_eq!(config.kernel(), &KernelImage::from_file(kernel.path()));
        assert_eq!(config.cmdline_extra(), ["debug=1"]);
    }
    #[test]
    fn prepares_implicit_vm_with_file_mount_parent_share() {
        if !firkin_vminitd_bytes::embedded() {
            return;
        }
        let rootfs = tempfile::NamedTempFile::new().unwrap();
        let source_dir = tempfile::tempdir().unwrap();
        let source = source_dir.path().join("config.json");
        std::fs::write(&source, b"{}").unwrap();
        let staging = tempfile::tempdir().unwrap();
        let builder = Container::builder("file-mount-plan")
            .unwrap()
            .file_mount(FileMount::read_only(&source, "/etc/app/config.json"))
            .rootfs(Rootfs::raw_block(rootfs.path()));
        let prepared = builder.prepare_implicit_vm_in(staging.path()).unwrap();
        let shares = prepared.config().virtiofs_shares();
        let expected_parent = source_dir.path().canonicalize().unwrap();
        assert_eq!(shares.len(), 1);
        assert_eq!(shares[0].host_path(), expected_parent);
        assert_eq!(shares[0].tag().as_str().len(), 36);
    }
    #[test]
    fn dns_and_hosts_configs_build_vminitd_requests() {
        let dns = DnsConfig {
            nameservers: vec![
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
            ],
            domain: Some("example.test".to_owned()),
            search: vec!["svc.local".to_owned()],
            options: vec!["ndots:2".to_owned()],
        };
        let hosts = HostsConfig {
            entries: vec![HostsEntry {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                hostnames: vec!["localhost".to_owned(), "worker".to_owned()],
                comment: Some("local".to_owned()),
            }],
            comment: Some("managed by firkin".to_owned()),
        };
        let dns_request = configure_dns_request(&dns, "/run/container/worker/rootfs");
        let hosts_request = configure_hosts_request(&hosts, "/run/container/worker/rootfs");
        assert_eq!(dns_request.location, "/run/container/worker/rootfs");
        assert_eq!(dns_request.nameservers, ["1.1.1.1", "::1"]);
        assert_eq!(dns_request.domain.as_deref(), Some("example.test"));
        assert_eq!(dns_request.search_domains, ["svc.local"]);
        assert_eq!(dns_request.options, ["ndots:2"]);
        assert_eq!(hosts_request.location, "/run/container/worker/rootfs");
        assert_eq!(
            hosts_request.entries[0].hostnames,
            ["localhost".to_owned(), "worker".to_owned()]
        );
        assert_eq!(hosts_request.entries[0].comment.as_deref(), Some("local"));
        assert_eq!(hosts_request.comment.as_deref(), Some("managed by firkin"));
    }
    #[tokio::test]
    async fn relay_stdio_stream_copies_bytes_and_closes_writer() {
        let (mut source, relay_reader) = tokio::io::duplex(64);
        let (relay_writer, mut sink) = tokio::io::duplex(64);
        let relay = tokio::spawn(relay_stdio_stream(
            relay_reader,
            relay_writer,
            "test relay stdio",
        ));
        source.write_all(b"inherited stdio\n").await.unwrap();
        drop(source);
        let mut output = Vec::new();
        sink.read_to_end(&mut output).await.unwrap();
        let copied = relay.await.unwrap().unwrap();
        assert_eq!(copied, u64::try_from(b"inherited stdio\n".len()).unwrap());
        assert_eq!(output, b"inherited stdio\n");
    }
    fn write_layer(path: &Path, file_path: &str, content: &[u8]) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .unwrap();
        let mut tar = tar::Builder::new(file);
        let mut dir = tar::Header::new_gnu();
        dir.set_entry_type(tar::EntryType::Directory);
        dir.set_path("bin").unwrap();
        dir.set_mode(0o755);
        dir.set_size(0);
        dir.set_cksum();
        tar.append(&dir, std::io::empty()).unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_path(file_path).unwrap();
        header.set_mode(0o755);
        header.set_size(u64::try_from(content.len()).unwrap());
        header.set_cksum();
        tar.append(&header, content).unwrap();
        tar.finish().unwrap();
    }
    fn write_hardlink_layer(path: &Path) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .unwrap();
        let mut tar = tar::Builder::new(file);
        append_dir(&mut tar, "bin");
        append_dir(&mut tar, "usr");
        append_dir(&mut tar, "usr/bin");
        let content = b"busybox\n";
        let mut file_header = tar::Header::new_gnu();
        file_header.set_entry_type(tar::EntryType::Regular);
        file_header.set_path("bin/[").unwrap();
        file_header.set_mode(0o755);
        file_header.set_size(u64::try_from(content.len()).unwrap());
        file_header.set_cksum();
        tar.append(&file_header, content.as_slice()).unwrap();
        append_hardlink(&mut tar, "bin/sh", "bin/[");
        append_hardlink(&mut tar, "usr/bin/env", "bin/[");
        tar.finish().unwrap();
    }
    fn append_dir<W: io_import::Write>(tar: &mut tar::Builder<W>, path: &str) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_path(path).unwrap();
        header.set_mode(0o755);
        header.set_size(0);
        header.set_cksum();
        tar.append(&header, std::io::empty()).unwrap();
    }
    fn append_hardlink<W: io_import::Write>(tar: &mut tar::Builder<W>, path: &str, target: &str) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Link);
        header.set_path(path).unwrap();
        header.set_link_name(target).unwrap();
        header.set_mode(0o755);
        header.set_size(0);
        header.set_cksum();
        tar.append(&header, std::io::empty()).unwrap();
    }
    fn assert_debugfs_cat(image: &Path, guest_path: &str, expected: &str) {
        let Some(debugfs) = debugfs_binary() else {
            eprintln!("warn: debugfs not found on PATH; skipping staged rootfs content check");
            return;
        };
        let out = Command::new(debugfs)
            .args(["-R", &format!("cat {guest_path}"), image.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    }
    fn debugfs_binary() -> Option<PathBuf> {
        for candidate in [
            "/opt/homebrew/opt/e2fsprogs/sbin/debugfs",
            "/usr/local/opt/e2fsprogs/sbin/debugfs",
            "/usr/sbin/debugfs",
            "/sbin/debugfs",
        ] {
            if Path::new(candidate).exists() {
                return Some(PathBuf::from(candidate));
            }
        }
        None
    }
}
pub mod builder;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use builder::*;
pub mod error;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use error::*;
pub mod ids;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use ids::*;
pub mod io;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use io::*;
pub mod process;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use process::*;
pub mod rootfs;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use rootfs::*;
pub mod runtime;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use runtime::*;
pub mod snapshot;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use snapshot::*;
pub mod vm_attach;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use vm_attach::*;
