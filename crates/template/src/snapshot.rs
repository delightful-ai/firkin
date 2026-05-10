//! snapshot — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use std::path::Path;
/// Boxed snapshot sink error.
pub type SnapshotSinkError = Box<dyn std::error::Error + Send + Sync + 'static>;
/// Sink that materializes the durable VM snapshot for a template build.
#[async_trait]
pub trait TemplateSnapshotSink: Send + Sync {
    /// Save a snapshot artifact at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the sink cannot persist the snapshot artifact.
    async fn save_snapshot(&self, path: &Path) -> Result<(), SnapshotSinkError>;
}
