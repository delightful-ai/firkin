use std::path::{Path, PathBuf};

use firkin_types::Size;

use super::SandboxResources;

/// Capacity limits for the single-node runtime scheduler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SingleNodeSchedulerConfig {
    max_sessions: usize,
    resources: SandboxResources,
}

impl SingleNodeSchedulerConfig {
    /// Construct scheduler capacity limits.
    #[must_use]
    pub const fn new(max_sessions: usize, resources: SandboxResources) -> Self {
        Self {
            max_sessions,
            resources,
        }
    }

    /// Return the maximum concurrently admitted sessions.
    #[must_use]
    pub const fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    /// Return aggregate CPU and memory capacity.
    #[must_use]
    pub const fn resources(&self) -> SandboxResources {
        self.resources
    }
}

impl Default for SingleNodeSchedulerConfig {
    fn default() -> Self {
        Self::new(64, SandboxResources::new(256, Size::gib(768)))
    }
}

/// Runtime configuration for one powerful local Apple/VZ host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SingleNodeConfig {
    root: PathBuf,
    domain: String,
    minimum_free_disk: Size,
    warm_pool_minimum_free_disk: Size,
    scheduler: SingleNodeSchedulerConfig,
}

impl SingleNodeConfig {
    /// Construct configuration rooted at `root` for generated sandbox domains.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, domain: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            domain: domain.into(),
            minimum_free_disk: Size::gib(10),
            warm_pool_minimum_free_disk: Size::gib(20),
            scheduler: SingleNodeSchedulerConfig::default(),
        }
    }

    /// Return the runtime state root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the domain suffix used by local domain proxy routes.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Return the hard free-disk floor for disk-consuming operations.
    #[must_use]
    pub const fn minimum_free_disk(&self) -> Size {
        self.minimum_free_disk
    }

    /// Return the free-disk floor required before warm-pool replenishment.
    #[must_use]
    pub const fn warm_pool_minimum_free_disk(&self) -> Size {
        self.warm_pool_minimum_free_disk
    }

    /// Return scheduler capacity limits.
    #[must_use]
    pub const fn scheduler(&self) -> SingleNodeSchedulerConfig {
        self.scheduler
    }

    /// Override scheduler capacity limits.
    #[must_use]
    pub const fn with_scheduler(mut self, scheduler: SingleNodeSchedulerConfig) -> Self {
        self.scheduler = scheduler;
        self
    }
}
