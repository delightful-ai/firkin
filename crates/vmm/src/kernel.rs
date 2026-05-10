//! kernel — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use std::path::PathBuf;
/// Kernel image input for the VM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelImage {
    /// The bundled kernel resolved by the higher-level core crate.
    Bundled,
    /// A caller-provided kernel file.
    File(PathBuf),
}
impl KernelImage {
    /// Use the bundled kernel.
    #[must_use]
    pub const fn bundled() -> Self {
        Self::Bundled
    }
    /// Use a caller-provided kernel path.
    #[must_use]
    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }
}
/// Serial boot log sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BootLog {
    /// Discard serial output.
    None,
    /// Append serial output to a file.
    File(PathBuf),
}
