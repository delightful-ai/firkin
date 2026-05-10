//! auth — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
/// Registry authentication configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Auth {
    /// Anonymous registry access.
    #[default]
    Anonymous,
}
