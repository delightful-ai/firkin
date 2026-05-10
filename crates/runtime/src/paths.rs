use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Root-level Firkin storage configuration.
///
/// The default layout intentionally avoids `TMPDIR` for runtime artifacts that
/// can grow with snapshot and restore traffic:
///
/// - state: `~/.firkin/state`
/// - cache: `~/.firkin/cache`
///
/// Set `FIRKIN_STATE_DIR` or `FIRKIN_CACHE_DIR` to relocate the two roots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FirkinStorageConfig {
    state_dir: PathBuf,
    cache_dir: PathBuf,
    runtime_continuations: PathBuf,
    runtime_restore_staging: PathBuf,
    single_node_snapshots: PathBuf,
}

impl FirkinStorageConfig {
    /// Build storage config from the current process environment.
    #[must_use]
    pub fn from_env() -> Self {
        let mut config = Self::from_roots(firkin_state_root(), firkin_cache_root());
        if let Some(root) = std::env::var_os("FIRKIN_RUNTIME_CONTINUATION_ROOT") {
            config.runtime_continuations = PathBuf::from(root);
        }
        if let Some(root) = std::env::var_os("FIRKIN_RUNTIME_RESTORE_STAGING_ROOT") {
            config.runtime_restore_staging = PathBuf::from(root);
        }
        if let Some(root) = std::env::var_os("FIRKIN_SINGLE_NODE_SNAPSHOT_ROOT") {
            config.single_node_snapshots = PathBuf::from(root);
        }
        config
    }

    /// Build storage config from explicit state and cache roots.
    #[must_use]
    pub fn from_roots(state_root: impl Into<PathBuf>, cache_root: impl Into<PathBuf>) -> Self {
        let state_root = state_root.into();
        Self {
            runtime_continuations: state_root.join("runtime").join("continuations"),
            runtime_restore_staging: state_root.join("runtime").join("restore-staging"),
            single_node_snapshots: state_root.join("single-node").join("snapshots"),
            state_dir: state_root,
            cache_dir: cache_root.into(),
        }
    }

    /// Override the runtime continuation snapshot directory.
    #[must_use]
    pub fn with_runtime_continuation_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.runtime_continuations = root.into();
        self
    }

    /// Override the runtime restore staging directory.
    #[must_use]
    pub fn with_runtime_restore_staging_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.runtime_restore_staging = root.into();
        self
    }

    /// Override the single-node snapshot directory.
    #[must_use]
    pub fn with_single_node_snapshot_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.single_node_snapshots = root.into();
        self
    }

    /// Durable Firkin state root.
    #[must_use]
    pub fn state_root(&self) -> &Path {
        &self.state_dir
    }

    /// Rebuildable Firkin cache root.
    #[must_use]
    pub fn cache_root(&self) -> &Path {
        &self.cache_dir
    }

    /// E2B/Cube-compatible local control-plane state directory.
    #[must_use]
    pub fn e2b_state_dir(&self) -> PathBuf {
        self.state_dir.join("e2b")
    }

    /// Runtime continuation snapshot directory.
    #[must_use]
    pub fn runtime_continuation_root(&self) -> PathBuf {
        self.runtime_continuations.clone()
    }

    /// Runtime restore staging directory.
    #[must_use]
    pub fn runtime_restore_staging_root(&self) -> PathBuf {
        self.runtime_restore_staging.clone()
    }

    /// Single-node snapshot directory.
    #[must_use]
    pub fn single_node_snapshot_root(&self) -> PathBuf {
        self.single_node_snapshots.clone()
    }
}

impl Default for FirkinStorageConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

/// Default durable Firkin state root.
#[must_use]
pub fn firkin_state_root() -> PathBuf {
    state_root_from_values(
        std::env::var_os("FIRKIN_STATE_DIR"),
        std::env::var_os("HOME"),
    )
}

/// Default rebuildable Firkin cache root.
#[must_use]
pub fn firkin_cache_root() -> PathBuf {
    env_path_or_else("FIRKIN_CACHE_DIR", || {
        firkin_base_from_home(std::env::var_os("HOME")).join("cache")
    })
}

/// Default runtime continuation snapshot directory.
#[must_use]
pub fn default_runtime_continuation_root() -> PathBuf {
    FirkinStorageConfig::from_env().runtime_continuation_root()
}

/// Default runtime restore staging directory.
#[must_use]
pub fn default_runtime_restore_staging_root() -> PathBuf {
    FirkinStorageConfig::from_env().runtime_restore_staging_root()
}

/// Default single-node snapshot directory.
#[must_use]
pub fn default_single_node_snapshot_root() -> PathBuf {
    FirkinStorageConfig::from_env().single_node_snapshot_root()
}

fn env_path_or_else(name: &str, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    std::env::var_os(name).map_or_else(fallback, PathBuf::from)
}

fn state_root_from_values(root: Option<OsString>, home: Option<OsString>) -> PathBuf {
    root.map_or_else(|| firkin_base_from_home(home).join("state"), PathBuf::from)
}

fn firkin_base_from_home(home: Option<OsString>) -> PathBuf {
    home.map_or_else(
        || PathBuf::from(".firkin"),
        |home| PathBuf::from(home).join(".firkin"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_root_uses_configured_path_exactly() {
        assert_eq!(
            state_root_from_values(
                Some(OsString::from("/var/firkin/state")),
                Some(OsString::from("/home/me"))
            ),
            PathBuf::from("/var/firkin/state")
        );
    }

    #[test]
    fn state_root_defaults_to_home_firkin_state() {
        assert_eq!(
            state_root_from_values(None, Some(OsString::from("/home/me"))),
            PathBuf::from("/home/me/.firkin/state")
        );
    }

    #[test]
    fn root_falls_back_to_relative_firkin_when_home_is_absent() {
        assert_eq!(
            state_root_from_values(None, None),
            PathBuf::from(".firkin/state")
        );
    }

    #[test]
    fn explicit_storage_config_derives_runtime_roots() {
        let config = FirkinStorageConfig::from_roots("/var/firkin/state", "/var/firkin/cache");

        assert_eq!(config.state_root(), Path::new("/var/firkin/state"));
        assert_eq!(config.cache_root(), Path::new("/var/firkin/cache"));
        assert_eq!(
            config.e2b_state_dir(),
            PathBuf::from("/var/firkin/state/e2b")
        );
        assert_eq!(
            config.runtime_continuation_root(),
            PathBuf::from("/var/firkin/state/runtime/continuations")
        );
        assert_eq!(
            config.runtime_restore_staging_root(),
            PathBuf::from("/var/firkin/state/runtime/restore-staging")
        );
        assert_eq!(
            config.single_node_snapshot_root(),
            PathBuf::from("/var/firkin/state/single-node/snapshots")
        );
    }

    #[test]
    fn explicit_storage_config_can_override_artifact_roots() {
        let config = FirkinStorageConfig::from_roots("/var/firkin/state", "/var/firkin/cache")
            .with_runtime_continuation_root("/snapshots/continuations")
            .with_runtime_restore_staging_root("/snapshots/restore")
            .with_single_node_snapshot_root("/snapshots/single-node");

        assert_eq!(
            config.runtime_continuation_root(),
            PathBuf::from("/snapshots/continuations")
        );
        assert_eq!(
            config.runtime_restore_staging_root(),
            PathBuf::from("/snapshots/restore")
        );
        assert_eq!(
            config.single_node_snapshot_root(),
            PathBuf::from("/snapshots/single-node")
        );
    }
}
