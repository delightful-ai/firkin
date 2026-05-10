#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Typed helpers for vminitd's `SandboxContext` API.
pub use guest_ops::{ApplyOciLayer, FilesystemUsage, FilesystemUsageStats, Fstrim, RemovePath};
/// Generated `SandboxContext` protobuf and tonic client types.
pub mod pb {
    #![allow(missing_docs, clippy::all, clippy::pedantic)]
    tonic::include_proto!("com.apple.containerization.sandbox.v3");
}
pub mod bundle;
pub(crate) mod guest_ops;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use bundle::*;
pub mod connect;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use connect::*;
pub mod copy;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use copy::*;
pub mod error;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use error::*;
pub mod network;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use network::*;
pub mod process;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use process::*;
pub mod proxy;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use proxy::*;
pub mod rosetta;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use rosetta::*;
pub mod stats;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use stats::*;
bitflags::bitflags! {
    #[doc = " Categories of container statistics to request from vminitd."]
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct StatCategory : u32 {
    #[doc = " Process pid statistics."] const PROCESS = 1 << 0; #[doc =
    " Memory usage statistics."] const MEMORY = 1 << 1; #[doc = " CPU usage statistics."]
    const CPU = 1 << 2; #[doc = " Block I/O statistics."] const BLOCK_IO = 1 << 3; #[doc
    = " Network interface statistics."] const NETWORK = 1 << 4; #[doc =
    " Memory event counters."] const MEMORY_EVENTS = 1 << 5; }
}
impl StatCategory {
    pub(crate) fn wants(self, category: Self) -> bool {
        self.is_empty() || self.contains(category)
    }
    pub(crate) fn proto_categories(self) -> Vec<i32> {
        let mut categories = Vec::new();
        if self.contains(Self::PROCESS) {
            categories.push(pb::StatCategory::Process as i32);
        }
        if self.contains(Self::MEMORY) {
            categories.push(pb::StatCategory::Memory as i32);
        }
        if self.contains(Self::CPU) {
            categories.push(pb::StatCategory::Cpu as i32);
        }
        if self.contains(Self::BLOCK_IO) {
            categories.push(pb::StatCategory::BlockIo as i32);
        }
        if self.contains(Self::NETWORK) {
            categories.push(pb::StatCategory::Network as i32);
        }
        if self.contains(Self::MEMORY_EVENTS) {
            categories.push(pb::StatCategory::MemoryEvents as i32);
        }
        categories
    }
}
