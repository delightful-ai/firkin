#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! E2B-compatible control-plane wire types.
//!
//! This crate is the typed boundary between the local backend and E2B SDKs.
//! It intentionally models the wire contract; runtime adapters still own
//! execution, persistence, proxying, snapshots, and network-policy enforcement.
pub mod capability;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use capability::*;
pub mod port;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use port::*;
pub mod runtime;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use runtime::*;
pub mod template;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use template::*;
