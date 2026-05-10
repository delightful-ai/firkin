#![warn(missing_docs)]
//! VM configuration and Virtualization.framework-backed typestates.
pub(crate) mod disk_image;
#[cfg(target_vendor = "apple")]
pub(crate) mod vz;
pub use disk_image::{
    BlankDiskImage, DiskImageConversion, DiskImageFormat, convert_disk_image,
    create_blank_disk_image,
};
#[allow(unused_imports)]
pub use firkin_types::{VmId, VsockPort};
#[allow(unused_imports)]
pub use firkin_vsock::VsockPeer;
#[cfg(test)]
#[allow(unused_imports)]
use std::marker::PhantomData;
#[cfg(test)]
mod tests {
    use super::*;
    fn running_vm_for_test() -> VirtualMachine<Running> {
        VirtualMachine {
            id: VmId::new(),
            config: VmConfig::default(),
            phase: Some(VmPhase::Running),
            network_interfaces: Vec::new(),
            #[cfg(target_vendor = "apple")]
            runtime: None,
            state: PhantomData,
        }
    }
    #[tokio::test]
    async fn dial_rejects_reserved_ports_before_runtime_backend() {
        let vm = running_vm_for_test();
        assert_eq!(
            vm.dial(VsockPort::new(1024)).await.expect_err("reserved"),
            Error::ReservedPort {
                port: VsockPort::new(1024),
                reason: "1024 is reserved for vminitd gRPC"
            }
        );
        assert_eq!(
            vm.dial(VsockPort::new(0x1000_0001))
                .await
                .expect_err("reserved"),
            Error::ReservedPort {
                port: VsockPort::new(0x1000_0001),
                reason: "0x1000_0000-0x2000_0000 is reserved for library stdio/relay allocation"
            }
        );
    }
    #[test]
    fn listen_rejects_reserved_ports_before_runtime_backend() {
        let vm = running_vm_for_test();
        assert_eq!(
            vm.listen(VsockPort::new(1024)).expect_err("reserved"),
            Error::ReservedPort {
                port: VsockPort::new(1024),
                reason: "1024 is reserved for vminitd gRPC"
            }
        );
    }
}
pub mod config;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use config::*;
pub mod error;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use error::*;
pub mod kernel;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use kernel::*;
pub mod network;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use network::*;
pub mod preflight;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use preflight::*;
pub mod storage;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use storage::*;
pub mod vm;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use vm::*;
