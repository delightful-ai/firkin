#![forbid(unsafe_code)]
#![allow(missing_docs)]
#![allow(
    ambiguous_glob_reexports,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use
)]

//! Public sandbox orchestration laws for Firkin.
//!
//! This crate owns neutral Rust types, capability traits, and contract tests.
//! Concrete Apple/VZ, E2B, HTTP, OCI, VM, envd implementation, benchmark, and
//! evidence behavior stays in adapter crates.

pub mod backend;
pub mod capability;
pub mod contract;
pub mod data_plane;
pub mod error;
pub mod event;
pub mod filesystem;
pub mod ids;
pub mod logs;
pub mod metrics;
pub mod ports;
pub mod prelude;
pub mod process;
pub mod runtime;
pub mod sandbox;
pub mod snapshot;
pub mod template;
pub mod warm_pool;

pub use backend::*;
pub use capability::*;
pub use contract::*;
pub use data_plane::*;
pub use error::*;
pub use event::*;
pub use filesystem::*;
pub use ids::*;
pub use logs::*;
pub use metrics::*;
pub use ports::*;
pub use process::*;
pub use runtime::*;
pub use sandbox::*;
pub use snapshot::*;
pub use template::*;
pub use warm_pool::*;
