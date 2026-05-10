//! Single-node Apple/VZ runtime composition.
//!
//! This module is the product-neutral boundary for local runtime orchestration.
//! Product APIs such as Cube map their request and response DTOs to this surface.
mod apple_vz;
mod backend;
mod config;
mod disk;
mod driver;
mod error;
mod model;
mod proxy;
mod sandbox_api;
mod scheduler;
mod state;
pub use apple_vz::AppleVzLocalRuntimeDriver;
pub use backend::SingleNodeBackend;
pub use config::{SingleNodeConfig, SingleNodeSchedulerConfig};
pub use disk::{CachedDiskProbe, DfDiskProbe, DiskPressureGuard, DiskProbe};
pub use driver::RuntimeDriver;
pub use error::{Error, Result};
pub use model::{
    CommandOutput, CommandRequest, LogEvent, PortRoute, RuntimeCreatedSandbox, RuntimeIdentity,
    RuntimeSnapshotRef, SandboxResources, SandboxSession, SandboxSessionState,
    SingleNodeCreateRequest, SingleNodeRuntimeMode, SnapshotKind, SnapshotRecord, TemplateMetadata,
};
pub use proxy::{DomainProxyAdapter, PortRegistry};
pub use sandbox_api::AppleVzBackend;
pub use scheduler::SingleNodeScheduler;
pub use state::{ActiveSessionRecord, FileStateStore, LogStore, StateStore};
