#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! E2B-compatible control-plane wire types.
//!
//! This crate is the typed boundary between the local backend and E2B SDKs.
//! It intentionally models the wire contract; runtime adapters still own
//! execution, persistence, proxying, snapshots, and network-policy enforcement.
pub mod control_plane;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use control_plane::*;
pub mod logs;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use logs::*;
pub mod metrics;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use metrics::*;
pub mod pods;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use pods::*;
pub mod sandbox;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use sandbox::*;
pub mod snapshot;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use snapshot::*;
pub mod template;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use template::*;
pub mod volume;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use volume::*;
