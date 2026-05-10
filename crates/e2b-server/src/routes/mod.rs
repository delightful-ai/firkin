//! routes — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use std::fmt::Write as _;
pub mod pod;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use pod::*;
pub mod sandbox;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use sandbox::*;
