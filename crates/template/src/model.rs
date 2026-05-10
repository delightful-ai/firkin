//! model — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_artifacts::SnapshotArtifactManifest;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
/// Immutable template build job model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateBuildJob {
    #[allow(missing_docs)]
    pub repo: String,
    checkout_ref: String,
    #[allow(missing_docs)]
    pub snapshot_output_path: PathBuf,
    setup_commands: Vec<String>,
    cache_warm_commands: Vec<String>,
}
impl TemplateBuildJob {
    /// Construct a template build job.
    #[must_use]
    pub fn new(
        repo: impl Into<String>,
        checkout_ref: impl Into<String>,
        snapshot_output_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo: repo.into(),
            checkout_ref: checkout_ref.into(),
            snapshot_output_path: snapshot_output_path.into(),
            setup_commands: Vec::new(),
            cache_warm_commands: Vec::new(),
        }
    }
    /// Add a setup command.
    #[must_use]
    pub fn setup_command(mut self, command: impl Into<String>) -> Self {
        self.setup_commands.push(command.into());
        self
    }
    /// Add a cache-warming command.
    #[must_use]
    pub fn cache_warm_command(mut self, command: impl Into<String>) -> Self {
        self.cache_warm_commands.push(command.into());
        self
    }
    /// Return the repository URL or path.
    #[must_use]
    pub fn repo(&self) -> &str {
        &self.repo
    }
    /// Return the checkout ref.
    #[must_use]
    pub fn checkout_ref(&self) -> &str {
        &self.checkout_ref
    }
    /// Return setup commands.
    #[must_use]
    pub fn setup_commands(&self) -> &[String] {
        &self.setup_commands
    }
    /// Return cache-warming commands.
    #[must_use]
    pub fn cache_warm_commands(&self) -> &[String] {
        &self.cache_warm_commands
    }
    /// Return the snapshot output path.
    #[must_use]
    pub fn snapshot_output_path(&self) -> &Path {
        &self.snapshot_output_path
    }
    /// Return the base snapshot manifest this build should produce.
    #[must_use]
    pub fn snapshot_manifest(&self, logical_id: impl Into<String>) -> SnapshotArtifactManifest {
        SnapshotArtifactManifest::base(logical_id, self.snapshot_output_path.clone())
    }
}
