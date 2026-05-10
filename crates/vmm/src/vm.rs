//! vm — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::config::VmConfig;
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use crate::network::NetworkInterface;
#[cfg(target_vendor = "apple")]
#[allow(unused_imports)]
use crate::vz;
#[allow(unused_imports)]
pub use firkin_types::{BlockDeviceId, Size, VirtiofsTag, VmId, VsockPort};
#[allow(unused_imports)]
pub use firkin_vsock::{VsockListener, VsockPeer, VsockStream};
#[allow(unused_imports)]
use std::marker::PhantomData;
#[allow(unused_imports)]
use std::num::NonZeroU32;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::time::Duration;
/// Runtime VM phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmPhase {
    /// The VM is running.
    Running,
    /// The VM is paused.
    Paused,
    /// The VM is stopping.
    Stopping,
}
/// Point-in-time VM statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmStatistics {
    pub(crate) cpus: NonZeroU32,
    pub(crate) memory: Size,
    pub(crate) phase: VmPhase,
}
impl VmStatistics {
    /// Construct statistics from sampled VM state.
    #[must_use]
    pub const fn new(cpus: NonZeroU32, memory: Size, phase: VmPhase) -> Self {
        Self {
            cpus,
            memory,
            phase,
        }
    }
    /// Return configured CPU count.
    #[must_use]
    pub const fn cpus(self) -> NonZeroU32 {
        self.cpus
    }
    /// Return configured memory.
    #[must_use]
    pub const fn memory(self) -> Size {
        self.memory
    }
    /// Return sampled VM phase.
    #[must_use]
    pub const fn phase(self) -> VmPhase {
        self.phase
    }
}
/// Marker for a VM that has not booted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotBooted;
/// Marker for a running VM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Running;
/// Virtual machine handle parameterized by boot state.
#[derive(Clone, Debug)]
pub struct VirtualMachine<S = NotBooted> {
    pub(crate) id: VmId,
    pub(crate) config: VmConfig,
    pub(crate) phase: Option<VmPhase>,
    pub(crate) network_interfaces: Vec<NetworkInterface>,
    #[cfg(target_vendor = "apple")]
    #[allow(dead_code)]
    pub(crate) runtime: Option<vz::VmRuntime>,
    pub(crate) state: PhantomData<S>,
}
impl VirtualMachine<NotBooted> {
    /// Construct a VM handle from a validated config.
    #[must_use]
    pub fn new(config: VmConfig) -> Self {
        Self {
            id: VmId::new(),
            config,
            phase: None,
            network_interfaces: Vec::new(),
            #[cfg(target_vendor = "apple")]
            runtime: None,
            state: PhantomData,
        }
    }
    /// Boot the VM and return a running handle.
    ///
    /// # Errors
    ///
    /// Returns Virtualization.framework configuration or start errors on Apple
    /// hosts. Returns [`Error::RuntimeNotImplemented`] only when this build has
    /// no runtime backend.
    pub async fn boot(self) -> Result<VirtualMachine<Running>> {
        #[cfg(target_vendor = "apple")]
        let (runtime, network_interfaces) = {
            let start = vz::prepare_start(&self.config)?;
            let network_interfaces = start.network_interfaces().to_vec();
            (Some(vz::start(start).await?), network_interfaces)
        };
        Ok(VirtualMachine {
            id: self.id,
            config: self.config,
            phase: Some(VmPhase::Running),
            #[cfg(target_vendor = "apple")]
            network_interfaces,
            #[cfg(not(target_vendor = "apple"))]
            network_interfaces: Vec::new(),
            #[cfg(target_vendor = "apple")]
            runtime,
            state: PhantomData,
        })
    }
    /// Boot the VM, restoring from `snapshot_path` when the file exists.
    ///
    /// # Errors
    ///
    /// Returns Virtualization.framework restore, resume, configuration, or start
    /// errors on Apple hosts. Returns [`Error::RuntimeNotImplemented`] only when
    /// this build has no runtime backend.
    #[cfg(feature = "snapshot")]
    pub async fn boot_or_restore(
        self,
        snapshot_path: impl AsRef<Path>,
    ) -> Result<VirtualMachine<Running>> {
        let snapshot_path = snapshot_path.as_ref();
        if !snapshot_path.exists() {
            return self.boot().await;
        }
        #[cfg(target_vendor = "apple")]
        let (runtime, network_interfaces) = {
            let prepared = vz::prepare_start(&self.config)?;
            let network_interfaces = prepared.network_interfaces().to_vec();
            (
                Some(vz::restore(prepared, snapshot_path).await?),
                network_interfaces,
            )
        };
        Ok(VirtualMachine {
            id: self.id,
            config: self.config,
            phase: Some(VmPhase::Running),
            #[cfg(target_vendor = "apple")]
            network_interfaces,
            #[cfg(not(target_vendor = "apple"))]
            network_interfaces: Vec::new(),
            #[cfg(target_vendor = "apple")]
            runtime,
            state: PhantomData,
        })
    }
}
impl VirtualMachine<Running> {
    /// Return configured CPU count.
    #[must_use]
    pub const fn cpus(&self) -> NonZeroU32 {
        self.config.cpus()
    }
    /// Return configured memory.
    #[must_use]
    pub const fn memory(&self) -> Size {
        self.config.memory()
    }
    /// Return guest-visible network interfaces assigned during VM boot.
    #[must_use]
    pub fn network_interfaces(&self) -> &[NetworkInterface] {
        &self.network_interfaces
    }
    /// Return whether the VM is currently paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        matches!(self.state(), VmPhase::Paused)
    }
    /// Return the current VM phase.
    #[must_use]
    pub fn state(&self) -> VmPhase {
        #[cfg(target_vendor = "apple")]
        if let Some(runtime) = &self.runtime {
            return runtime.phase_blocking();
        }
        match self.phase {
            Some(phase) => phase,
            None => VmPhase::Stopping,
        }
    }
    /// Dial an arbitrary guest-side vsock port.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ReservedPort`] for library-reserved ports and
    /// [`Error::RuntimeNotImplemented`] when this handle has no runtime backend.
    pub async fn dial(&self, port: VsockPort) -> Result<VsockStream> {
        reject_reserved_port(port)?;
        self.dial_reserved_port(port).await
    }
    /// Dial a guest-side port reserved for firkin's own runtime wiring.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RuntimeNotImplemented`] when this handle has no runtime
    /// backend.
    #[doc(hidden)]
    pub async fn dial_reserved_port(&self, port: VsockPort) -> Result<VsockStream> {
        #[cfg(target_vendor = "apple")]
        if let Some(runtime) = &self.runtime {
            return runtime.dial(port).await;
        }
        let _ = self.id();
        runtime_not_implemented().await
    }
    /// Listen on a host-side vsock port for guest-initiated connections.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ReservedPort`] for library-reserved ports and
    /// [`Error::RuntimeNotImplemented`] when this handle has no runtime backend.
    pub fn listen(&self, port: VsockPort) -> Result<VsockListener> {
        reject_reserved_port(port)?;
        self.listen_reserved_port(port)
    }
    /// Listen on a host-side port reserved for firkin's own runtime wiring.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RuntimeNotImplemented`] when this handle has no runtime
    /// backend.
    #[doc(hidden)]
    pub fn listen_reserved_port(&self, port: VsockPort) -> Result<VsockListener> {
        #[cfg(target_vendor = "apple")]
        if let Some(runtime) = &self.runtime {
            return Ok(runtime.listen(port));
        }
        let _ = self.id();
        Err(Error::RuntimeNotImplemented)
    }
    /// Pause VM execution.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RuntimeNotImplemented`] when this handle has no runtime
    /// backend.
    pub async fn pause(&self) -> Result<()> {
        #[cfg(target_vendor = "apple")]
        if let Some(runtime) = &self.runtime {
            return runtime.pause().await;
        }
        let _ = self.state();
        runtime_not_implemented().await
    }
    /// Resume VM execution.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RuntimeNotImplemented`] when this handle has no runtime
    /// backend.
    pub async fn resume(&self) -> Result<()> {
        #[cfg(target_vendor = "apple")]
        if let Some(runtime) = &self.runtime {
            return runtime.resume().await;
        }
        let _ = self.state();
        runtime_not_implemented().await
    }
    /// Save this VM's current machine state to `path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RuntimeNotImplemented`] when this handle has no runtime
    /// backend.
    #[cfg(feature = "snapshot")]
    pub async fn save_snapshot(&self, path: impl AsRef<Path>) -> Result<()> {
        #[cfg(target_vendor = "apple")]
        if let Some(runtime) = &self.runtime {
            return runtime.save_snapshot(path.as_ref()).await;
        }
        let _ = self.state();
        let _ = path.as_ref();
        runtime_not_implemented().await
    }
    /// Return point-in-time VM statistics.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RuntimeNotImplemented`] when this handle has no runtime
    /// backend.
    pub async fn statistics(&self) -> Result<VmStatistics> {
        #[cfg(target_vendor = "apple")]
        if self.runtime.is_some() {
            return Ok(VmStatistics::new(self.cpus(), self.memory(), self.state()));
        }
        let _ = self.state();
        runtime_not_implemented().await
    }
    /// Stop the VM with the default grace period.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RuntimeNotImplemented`] when this handle has no runtime
    /// backend.
    pub async fn stop(self) -> Result<()> {
        #[cfg(target_vendor = "apple")]
        if let Some(runtime) = self.runtime {
            return runtime.stop().await;
        }
        runtime_not_implemented().await
    }
    /// Stop the VM with a caller-selected grace period.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RuntimeNotImplemented`] when this handle has no runtime
    /// backend.
    pub async fn stop_with_grace(self, grace: Duration) -> Result<()> {
        #[cfg(target_vendor = "apple")]
        if let Some(runtime) = self.runtime {
            let _ = runtime.request_stop().await;
            tokio::time::sleep(grace).await;
            return runtime.stop().await;
        }
        let _ = grace;
        runtime_not_implemented().await
    }
}
impl<S> VirtualMachine<S> {
    /// Return the VM identity.
    #[must_use]
    pub const fn id(&self) -> &VmId {
        &self.id
    }
    /// Return the VM configuration.
    #[must_use]
    pub const fn config(&self) -> &VmConfig {
        &self.config
    }
}
fn reject_reserved_port(port: VsockPort) -> Result<()> {
    if port.get() == 1024 {
        return Err(Error::ReservedPort {
            port,
            reason: "1024 is reserved for vminitd gRPC",
        });
    }
    if (0x1000_0000..=0x2000_0000).contains(&port.get()) {
        return Err(Error::ReservedPort {
            port,
            reason: "0x1000_0000-0x2000_0000 is reserved for library stdio/relay allocation",
        });
    }
    Ok(())
}
async fn runtime_not_implemented<T>() -> Result<T> {
    std::future::ready(()).await;
    Err(Error::RuntimeNotImplemented)
}
