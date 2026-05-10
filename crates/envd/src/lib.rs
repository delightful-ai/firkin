#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! envd protocol contracts for Firkin data-plane adapters.
//!
//! This crate owns envd process and filesystem laws. Product surfaces such as
//! E2B may translate these contracts into their own wire protocols, but envd
//! behavior is not owned by those products.

pub mod filesystem;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use filesystem::*;
pub mod process;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use process::*;

/// Default envd service port.
pub const DEFAULT_ENVD_PORT: u16 = 49983;
