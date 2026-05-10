//! registry — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
pub mod pod;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use pod::*;
pub mod sandbox;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use sandbox::*;
pub mod template;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use template::*;
pub mod volume;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use volume::*;
