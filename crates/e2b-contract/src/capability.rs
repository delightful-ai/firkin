//! capability — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
/// Runtime capability set reported by a local runtime adapter.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct RuntimeCapabilitySet {
    /// Runtime backend name.
    pub backend: String,
    /// Supported capability names.
    pub supported: Vec<String>,
    /// Unsupported capability names and reasons.
    pub unsupported: Vec<(String, String)>,
}
