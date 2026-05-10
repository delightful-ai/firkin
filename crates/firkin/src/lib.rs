#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Facade crate for the firkin Rust containerization library.
use std::fmt;
/// A named runtime capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeCapability {
    name: &'static str,
    supported: bool,
    reason: Option<&'static str>,
}
impl RuntimeCapability {
    /// Construct a supported capability.
    #[must_use]
    pub const fn supported(name: &'static str, reason: Option<&'static str>) -> Self {
        Self {
            name,
            supported: true,
            reason,
        }
    }
    /// Construct an unsupported capability.
    #[must_use]
    pub const fn unsupported(name: &'static str, reason: &'static str) -> Self {
        Self {
            name,
            supported: false,
            reason: Some(reason),
        }
    }
    /// Return the stable capability name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }
    /// Return whether the capability is supported.
    #[must_use]
    pub const fn is_supported(self) -> bool {
        self.supported
    }
    /// Return the optional explanatory reason.
    #[must_use]
    pub const fn reason(self) -> Option<&'static str> {
        self.reason
    }
}
/// Error returned when a required runtime capability is unavailable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedRuntimeCapability {
    name: &'static str,
    reason: &'static str,
}
impl UnsupportedRuntimeCapability {
    /// Construct an unsupported-capability error.
    #[must_use]
    pub const fn new(name: &'static str, reason: &'static str) -> Self {
        Self { name, reason }
    }
    /// Return the capability name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
    /// Return the reason the capability is unavailable.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}
impl fmt::Display for UnsupportedRuntimeCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "runtime capability `{}` is not supported: {}",
            self.name, self.reason
        )
    }
}
impl std::error::Error for UnsupportedRuntimeCapability {}
/// Capability set for a runtime backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeCapabilities {
    backend: &'static str,
    supported: &'static [RuntimeCapability],
    unsupported: &'static [RuntimeCapability],
}
impl RuntimeCapabilities {
    /// Construct a runtime capability set.
    #[must_use]
    pub const fn new(
        backend: &'static str,
        supported: &'static [RuntimeCapability],
        unsupported: &'static [RuntimeCapability],
    ) -> Self {
        Self {
            backend,
            supported,
            unsupported,
        }
    }
    /// Return the backend name.
    #[must_use]
    pub const fn backend(self) -> &'static str {
        self.backend
    }
    /// Return capabilities implemented by the backend.
    #[must_use]
    pub const fn supported(self) -> &'static [RuntimeCapability] {
        self.supported
    }
    /// Return capabilities intentionally not implemented by the backend.
    #[must_use]
    pub const fn unsupported(self) -> &'static [RuntimeCapability] {
        self.unsupported
    }
    /// Return whether the backend reports `name` as supported.
    #[must_use]
    pub fn supports(self, name: &str) -> bool {
        self.supported
            .iter()
            .any(|capability| capability.name == name)
    }
    /// Require a capability and return a hard error when it is unsupported.
    ///
    /// Unknown names are treated as unsupported. The caller should use stable
    /// capability names from this crate or from the target control-plane spec.
    ///
    /// # Errors
    ///
    /// Returns [`UnsupportedRuntimeCapability`] when `name` is listed as
    /// unsupported or not advertised by the backend.
    pub fn require_supported(self, name: &'static str) -> Result<(), UnsupportedRuntimeCapability> {
        if self.supports(name) {
            return Ok(());
        }
        let reason = self
            .unsupported
            .iter()
            .find(|capability| capability.name == name)
            .and_then(|capability| capability.reason)
            .unwrap_or("capability is not advertised by this backend");
        Err(UnsupportedRuntimeCapability::new(name, reason))
    }
}
/// Capabilities implemented by the Apple/VZ local runtime layer.
pub const APPLE_LOCAL_RUNTIME_SUPPORTED_CAPABILITIES: &[RuntimeCapability] = &[
    RuntimeCapability::supported("create-linux-container", None),
    RuntimeCapability::supported("delete-linux-container", None),
    RuntimeCapability::supported("exec-process", None),
    RuntimeCapability::supported("copy-in", None),
    RuntimeCapability::supported("copy-out", None),
    RuntimeCapability::supported("pod-vm", None),
    RuntimeCapability::supported("pod-emptydir", None),
    RuntimeCapability::supported("pod-guest-path-rootfs", None),
    RuntimeCapability::supported("basic-container-metrics", None),
    RuntimeCapability::supported(
        "vmnet-network-attach",
        Some("Available on macOS 26 and newer"),
    ),
    RuntimeCapability::supported(
        "rosetta-linux-amd64",
        Some("Requires Rosetta preflight success"),
    ),
];
/// E2B/Cube capabilities not implemented by the Apple/VZ local runtime layer.
pub const APPLE_LOCAL_RUNTIME_UNSUPPORTED_CAPABILITIES: &[RuntimeCapability] = &[
    RuntimeCapability::unsupported(
        "e2b-envd-compatible-api",
        "vminitd is not envd; a guest envd or deliberate envd bridge is required",
    ),
    RuntimeCapability::unsupported(
        "snapshot-backed-pause-resume-connect",
        "VZ save/restore exists below this layer, but the E2B-compatible sandbox snapshot lifecycle is not implemented",
    ),
    RuntimeCapability::unsupported(
        "e2b-network-policy",
        "Current APIs attach/configure interfaces but do not enforce allow/deny/mask policy",
    ),
    RuntimeCapability::unsupported(
        "domain-host-proxy",
        "The crate exposes vsock/socket relay primitives, not the E2B {port}-{sandboxID}.domain proxy",
    ),
    RuntimeCapability::unsupported(
        "template-build-service",
        "Image and rootfs operations exist, but the E2B template builder/control-plane service is external",
    ),
];
/// Return the Apple/VZ local runtime capability set.
#[must_use]
pub const fn apple_local_runtime_capabilities() -> RuntimeCapabilities {
    RuntimeCapabilities::new(
        "apple-vz",
        APPLE_LOCAL_RUNTIME_SUPPORTED_CAPABILITIES,
        APPLE_LOCAL_RUNTIME_UNSUPPORTED_CAPABILITIES,
    )
}
/// Container orchestration surface.
pub mod core {
    pub use firkin_core::*;
}
/// EXT4 image writer.
pub mod ext4 {
    pub use firkin_ext4::*;
}
/// E2B-compatible control-plane wire types.
pub mod e2b {
    #[allow(unused_imports, ambiguous_glob_reexports)]
    pub use {firkin_e2b_contract::*, firkin_e2b_server::*, firkin_e2b_wire::*};
}
/// Neutral envd protocol and data-plane contracts.
pub mod envd {
    pub use firkin_envd::*;
}
/// OCI image pulling and bundle primitives.
pub mod oci {
    pub use firkin_oci::*;
}
/// Public sandbox orchestration API.
pub mod sandbox {
    pub use firkin_sandbox::*;

    /// Local Apple/VZ-backed sandbox backend.
    pub mod apple_vz {
        pub use firkin_single_node::{AppleVzBackend, SingleNodeConfig};
    }
}
/// Production substrate control models.
pub mod substrate {
    #[allow(unused_imports, ambiguous_glob_reexports)]
    pub use {
        firkin_admission::*, firkin_artifacts::*, firkin_evidence::*, firkin_hygiene::*,
        firkin_template::*,
    };
}
/// Trace and benchmark sample primitives.
pub mod trace {
    pub use firkin_trace::*;
}
/// Benchmark execution and benchmark-side evidence writers.
pub mod benchmark {
    pub use firkin_benchmark::*;
}
/// Benchmark evidence reports, metric catalogs, and gates.
pub mod evidence {
    pub use firkin_evidence::*;
}
/// Production runtime composition.
pub mod runtime {
    pub use firkin_runtime::*;

    #[allow(missing_docs)]
    pub mod single_node {
        #[allow(unused_imports, ambiguous_glob_reexports)]
        pub use firkin_single_node::*;
    }
}
/// Template build execution.
pub mod template {
    pub use firkin_template::*;
}
/// Shared value types.
pub mod types {
    pub use firkin_types::*;
}
/// vminitd client helpers.
pub mod vminitd_client {
    pub use firkin_vminitd_client::*;
}
/// Pinned vminitd ELF bytes.
pub mod vminitd_bytes {
    pub use firkin_vminitd_bytes::*;
}
/// Virtual machine primitives.
pub mod vmm {
    pub use firkin_vmm::*;
}
/// Vsock stream primitives.
pub mod vsock {
    pub use firkin_vsock::*;
}
/// Common imports for application code.
pub mod prelude {
    pub use crate::{
        RuntimeCapabilities, RuntimeCapability, UnsupportedRuntimeCapability,
        apple_local_runtime_capabilities,
    };
    pub use firkin_core::{
        Capability, ChildStderr, ChildStdin, ChildStdout, Container, ContainerBuilder,
        ContainerStatistics, CoreContainerFactory, DnsConfig, EmptyDirMedium, EmptyDirVolume,
        ExecConfig, FileMount, GuestFilesystem, GuestPath, HostsConfig, HostsEntry, ImplicitVm,
        Init, InvalidCapability, InvalidRlimit, LinuxCapabilities, LinuxRlimit, MaterializedRootfs,
        Mount, MountedPodStore, OnVm, Pod, PodBuilder, PodContainerSpec, PodId, PodRootfsSource,
        PodStoreSpec, PodVolumeMount, Process, PtyConfig, Ready, ReadyPty, RlimitKind, Rootfs,
        Seccomp, SeccompAction, SeccompArch, SeccompArgRule, SeccompFlag, SeccompOp,
        SeccompSyscallRule, SocketDirection, StatCategory, Stdio, Streams, UnixSocketConfig, User,
        VmRootfs, materialize_rootfs_in_pod_store, mount_pod_store,
    };
    pub use firkin_sandbox::prelude::*;
    pub use firkin_types::{
        ContainerId, Hostname, InvalidNetworkPolicyRule, InvalidPortSandboxHost, NetworkPolicyRule,
        Platform, PortSandboxHost, ProcessId, SandboxNetworkPolicy, Size, VirtiofsTag, VsockPort,
    };
    pub use firkin_vmm::{
        BlankDiskImage, BootLog, DiskImageFormat, KernelImage, Network, VirtualMachine, VmConfig,
        create_blank_disk_image,
    };
    pub use firkin_vsock::VsockStream;
}
pub use firkin_core::{
    BlockIoDevice, BlockIoStatistics, Capability, ChildStderr, ChildStdin, ChildStdout, Container,
    ContainerBuilder, ContainerStatistics, ContainerStatisticsQuery, CoreContainerFactory,
    CpuStatistics, DnsConfig, EmptyDirMedium, EmptyDirVolume, ExecConfig, FileMount,
    GuestFilesystem, GuestPath, HostsConfig, HostsEntry, ImplicitVm, Init, InvalidCapability,
    InvalidRlimit, LinuxCapabilities, LinuxRlimit, MaterializedRootfs, MemoryEventStatistics,
    MemoryStatistics, Mount, MountedPodStore, NetworkStatistics, OnVm, OnVmArc, Pod, PodBuilder,
    PodContainerSpec, PodId, PodRootfsSource, PodStoreSpec, PodVolumeMount, Process,
    ProcessKillHandle, ProcessStatistics, Pty, PtyConfig, Ready, ReadyPty, RlimitKind, Rootfs,
    Seccomp, SeccompAction, SeccompArch, SeccompArgRule, SeccompFlag, SeccompOp,
    SeccompSyscallRule, SocketDirection, StatCategory, Stdio, Streams, UnixSocketConfig, User,
    VmRootfs, materialize_rootfs_in_pod_store, mount_pod_store,
};
pub use firkin_types::{
    ContainerId, Hostname, InvalidNetworkPolicyRule, InvalidPortSandboxHost, NetworkPolicyRule,
    Platform, PortSandboxHost, ProcessId, SandboxNetworkPolicy, Size, VirtiofsTag, VsockPort,
};
pub use firkin_vmm::{
    BlankDiskImage, BootLog, DiskImageFormat, KernelImage, Network, VirtualMachine, VmConfig,
    create_blank_disk_image,
};
pub use firkin_vsock::{VsockListener, VsockPeer, VsockStream};
