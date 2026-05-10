//! State records for single-node sessions and snapshots.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{Error, LogEvent, Result, SandboxResources, SnapshotRecord};

/// Persisted active session record for restart reconciliation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ActiveSessionRecord {
    /// Sandbox ID.
    pub sandbox_id: String,
    /// Template ID used to create the session.
    pub template_id: String,
    /// Runtime client ID.
    pub client_id: String,
    /// Optional envd access token.
    pub envd_access_token: Option<String>,
    /// Unix timestamp when the session started.
    pub started_at_unix_seconds: i64,
    /// Unix timestamp when the session should expire.
    pub end_at_unix_seconds: i64,
    /// Resources reserved by the session.
    pub resources: SandboxResources,
    /// Whether a runtime VM/container was attached.
    pub runtime_attached: bool,
}

/// In-memory single-node runtime state used by the hot sandbox lifecycle path.
#[derive(Clone, Debug, Default)]
pub struct StateStore {
    active: Arc<Mutex<Vec<ActiveSessionRecord>>>,
    snapshots: Arc<Mutex<Vec<SnapshotRecord>>>,
    logs: Arc<Mutex<HashMap<String, Vec<LogEvent>>>>,
}

impl StateStore {
    /// Construct an empty in-memory state store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct state from recovered records.
    #[must_use]
    pub fn from_records(
        active: Vec<ActiveSessionRecord>,
        snapshots: Vec<SnapshotRecord>,
        logs: HashMap<String, Vec<LogEvent>>,
    ) -> Self {
        Self {
            active: Arc::new(Mutex::new(active)),
            snapshots: Arc::new(Mutex::new(snapshots)),
            logs: Arc::new(Mutex::new(logs)),
        }
    }

    /// Load active session records from memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when the state lock is poisoned.
    pub fn load_active(&self) -> Result<Vec<ActiveSessionRecord>> {
        Ok(self.lock_active()?.clone())
    }

    /// Replace active session records in memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when the state lock is poisoned.
    pub fn save_active(&self, sessions: Vec<ActiveSessionRecord>) -> Result<()> {
        *self.lock_active()? = sessions;
        Ok(())
    }

    /// Load log events keyed by sandbox ID from memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when the state lock is poisoned.
    pub fn load_logs(&self) -> Result<HashMap<String, Vec<LogEvent>>> {
        Ok(self.lock_logs()?.clone())
    }

    /// Replace log events keyed by sandbox ID in memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when the state lock is poisoned.
    pub fn save_logs(&self, logs: HashMap<String, Vec<LogEvent>>) -> Result<()> {
        *self.lock_logs()? = logs;
        Ok(())
    }

    /// Load snapshot records from memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when the state lock is poisoned.
    pub fn load_snapshots(&self) -> Result<Vec<SnapshotRecord>> {
        Ok(self.lock_snapshots()?.clone())
    }

    /// Replace snapshot records in memory.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when the state lock is poisoned.
    pub fn save_snapshots(&self, snapshots: Vec<SnapshotRecord>) -> Result<()> {
        *self.lock_snapshots()? = snapshots;
        Ok(())
    }

    /// Drop expired active session records using the provided current unix time.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when the state lock is poisoned.
    pub fn reconcile_active_entries(
        &self,
        now_unix_seconds: i64,
    ) -> Result<Vec<ActiveSessionRecord>> {
        let reconciled = self
            .load_active()?
            .into_iter()
            .filter(|session| session.end_at_unix_seconds > now_unix_seconds)
            .collect::<Vec<_>>();
        self.save_active(reconciled.clone())?;
        Ok(reconciled)
    }

    fn lock_active(&self) -> Result<std::sync::MutexGuard<'_, Vec<ActiveSessionRecord>>> {
        self.active.lock().map_err(|_| {
            Error::StatePersistenceFailed("single-node active state lock poisoned".to_owned())
        })
    }

    fn lock_snapshots(&self) -> Result<std::sync::MutexGuard<'_, Vec<SnapshotRecord>>> {
        self.snapshots.lock().map_err(|_| {
            Error::StatePersistenceFailed("single-node snapshot state lock poisoned".to_owned())
        })
    }

    fn lock_logs(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, Vec<LogEvent>>>> {
        self.logs.lock().map_err(|_| {
            Error::StatePersistenceFailed("single-node log state lock poisoned".to_owned())
        })
    }
}

/// Explicit file-backed persistence helper for restart and recovery boundaries.
#[derive(Clone, Debug)]
pub struct FileStateStore {
    dir: Arc<PathBuf>,
}

impl FileStateStore {
    /// Construct file persistence rooted at `dir`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when the directory cannot be created.
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|error| {
            Error::StatePersistenceFailed(format!(
                "create single-node state dir {}: {error}",
                dir.display()
            ))
        })?;
        Ok(Self { dir: Arc::new(dir) })
    }

    /// Return the state root directory.
    #[must_use]
    pub fn root_dir(&self) -> &Path {
        self.dir.as_ref()
    }

    /// Return the managed snapshot artifact directory.
    #[must_use]
    pub fn snapshot_artifacts_dir(&self) -> PathBuf {
        self.dir.join("snapshot-artifacts")
    }

    /// Load all persisted records into an in-memory hot-path state store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be read or decoded.
    pub fn load_state(&self) -> Result<StateStore> {
        Ok(StateStore::from_records(
            self.load_active()?,
            self.load_snapshots()?,
            self.load_logs()?,
        ))
    }

    /// Save all records from an in-memory state store.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be encoded or written.
    pub fn save_state(&self, state: &StateStore) -> Result<()> {
        let active = state.load_active()?;
        let snapshots = state.load_snapshots()?;
        let logs = state.load_logs()?;
        self.save_active(&active)?;
        self.save_snapshots(&snapshots)?;
        self.save_logs(&logs)
    }

    /// Load active session records.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be read or decoded.
    pub fn load_active(&self) -> Result<Vec<ActiveSessionRecord>> {
        Self::read_json(&self.active_path())
    }

    /// Save active session records.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be encoded or written.
    pub fn save_active(&self, sessions: &[ActiveSessionRecord]) -> Result<()> {
        Self::write_json(&self.active_path(), sessions)
    }

    /// Drop expired active session records from persisted state using the provided current unix time.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be read or written.
    pub fn reconcile_active_entries(
        &self,
        now_unix_seconds: i64,
    ) -> Result<Vec<ActiveSessionRecord>> {
        let state = StateStore::from_records(self.load_active()?, Vec::new(), HashMap::new());
        let reconciled = state.reconcile_active_entries(now_unix_seconds)?;
        self.save_active(&reconciled)?;
        Ok(reconciled)
    }

    /// Load log events keyed by sandbox ID.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be read or decoded.
    pub fn load_logs(&self) -> Result<HashMap<String, Vec<LogEvent>>> {
        Self::read_json(&self.logs_path())
    }

    /// Save log events keyed by sandbox ID.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be encoded or written.
    pub fn save_logs(&self, logs: &HashMap<String, Vec<LogEvent>>) -> Result<()> {
        Self::write_json(&self.logs_path(), logs)
    }

    /// Load snapshot records.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be read or decoded.
    pub fn load_snapshots(&self) -> Result<Vec<SnapshotRecord>> {
        Self::read_json(&self.snapshots_path())
    }

    /// Save snapshot records.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be encoded or written.
    pub fn save_snapshots(&self, snapshots: &[SnapshotRecord]) -> Result<()> {
        Self::write_json(&self.snapshots_path(), snapshots)
    }

    /// Reconcile managed snapshot artifact metadata against files on disk.
    ///
    /// Missing managed artifacts are dropped from metadata. Orphan files in the
    /// managed artifact directory are deleted.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state or artifact cleanup fails.
    pub fn reconcile_snapshot_artifacts(
        &self,
        snapshots: Vec<SnapshotRecord>,
    ) -> Result<Vec<SnapshotRecord>> {
        let original_len = snapshots.len();
        let mut referenced_artifacts = HashSet::new();
        let mut reconciled = Vec::with_capacity(snapshots.len());
        for snapshot in snapshots {
            let Some(location) = snapshot.location.as_ref() else {
                reconciled.push(snapshot);
                continue;
            };
            let location = PathBuf::from(location);
            let managed_artifact = self.is_managed_snapshot_artifact(&location);
            if location.exists() {
                if managed_artifact {
                    referenced_artifacts.insert(location);
                }
                reconciled.push(snapshot);
            } else if !managed_artifact {
                reconciled.push(snapshot);
            }
        }

        self.delete_orphan_snapshot_artifacts(&referenced_artifacts)?;
        if reconciled.len() != original_len {
            self.save_snapshots(&reconciled)?;
        }
        Ok(reconciled)
    }

    /// Return the active-session state file path.
    #[must_use]
    pub fn active_path(&self) -> PathBuf {
        self.dir.join("active.json")
    }

    /// Return the log state file path.
    #[must_use]
    pub fn logs_path(&self) -> PathBuf {
        self.dir.join("logs.json")
    }

    /// Return the snapshot state file path.
    #[must_use]
    pub fn snapshots_path(&self) -> PathBuf {
        self.dir.join("snapshots.json")
    }

    fn write_json<T: serde::Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
        let tmp_path = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(value).map_err(|error| {
            Error::StatePersistenceFailed(format!("serialize single-node state: {error}"))
        })?;
        fs::write(&tmp_path, data).map_err(|error| {
            Error::StatePersistenceFailed(format!(
                "write single-node state {}: {error}",
                tmp_path.display()
            ))
        })?;
        fs::rename(&tmp_path, path).map_err(|error| {
            Error::StatePersistenceFailed(format!(
                "replace single-node state {}: {error}",
                path.display()
            ))
        })?;
        Ok(())
    }

    fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
        match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data).map_err(|error| {
                Error::StatePersistenceFailed(format!(
                    "parse single-node state {}: {error}",
                    path.display()
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
            Err(error) => Err(Error::StatePersistenceFailed(format!(
                "read single-node state {}: {error}",
                path.display()
            ))),
        }
    }

    fn is_managed_snapshot_artifact(&self, path: &Path) -> bool {
        path.parent()
            .is_some_and(|parent| parent == self.snapshot_artifacts_dir())
    }

    fn delete_orphan_snapshot_artifacts(&self, referenced: &HashSet<PathBuf>) -> Result<()> {
        let artifact_dir = self.snapshot_artifacts_dir();
        let entries = match fs::read_dir(&artifact_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(Error::StatePersistenceFailed(format!(
                    "read single-node snapshot artifact dir {}: {error}",
                    artifact_dir.display()
                )));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| {
                Error::StatePersistenceFailed(format!(
                    "read single-node snapshot artifact entry {}: {error}",
                    artifact_dir.display()
                ))
            })?;
            let path = entry.path();
            if path.is_file() && !referenced.contains(&path) {
                fs::remove_file(&path).map_err(|error| {
                    Error::StatePersistenceFailed(format!(
                        "delete orphan single-node snapshot artifact {}: {error}",
                        path.display()
                    ))
                })?;
            }
        }
        Ok(())
    }
}

/// Bounded log store over hot-path in-memory state.
#[derive(Clone, Debug)]
pub struct LogStore {
    state: StateStore,
    files: Option<FileStateStore>,
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new(StateStore::new())
    }
}

impl LogStore {
    const MAX_ENTRIES_PER_SANDBOX: usize = 1_000;

    /// Construct a log store over an in-memory state store.
    #[must_use]
    pub const fn new(state: StateStore) -> Self {
        Self { state, files: None }
    }

    /// Construct a log store backed by the same durable file state as sessions.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when persisted state cannot be loaded.
    pub fn with_file_state_store(files: FileStateStore) -> Result<Self> {
        Ok(Self {
            state: files.load_state()?,
            files: Some(files),
        })
    }

    fn persist_logs(&self) -> Result<()> {
        if let Some(files) = &self.files {
            let logs = self.state.load_logs()?;
            files.save_logs(&logs)?;
        }
        Ok(())
    }

    /// Record a log message for a sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when log state cannot be updated.
    pub fn record(&self, sandbox_id: &str, message: String) -> Result<()> {
        let mut logs = self.state.load_logs()?;
        let sandbox_entries = logs.entry(sandbox_id.to_owned()).or_default();
        sandbox_entries.push(LogEvent::new(message));
        let overflow = sandbox_entries
            .len()
            .saturating_sub(Self::MAX_ENTRIES_PER_SANDBOX);
        if overflow > 0 {
            sandbox_entries.drain(0..overflow);
        }
        self.state.save_logs(logs)?;
        self.persist_logs()
    }

    /// Remove all logs for a sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when log state cannot be updated.
    pub fn remove_sandbox(&self, sandbox_id: &str) -> Result<()> {
        let mut logs = self.state.load_logs()?;
        logs.remove(sandbox_id);
        self.state.save_logs(logs)?;
        self.persist_logs()
    }

    /// Return a page of logs for a sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when log state cannot be read.
    pub fn entries(
        &self,
        sandbox_id: &str,
        cursor: Option<i64>,
        limit: i32,
    ) -> Result<Vec<LogEvent>> {
        let logs = self.state.load_logs()?;
        let start = usize::try_from(cursor.unwrap_or(0).max(0)).unwrap_or(usize::MAX);
        let limit = usize::try_from(limit.max(0)).unwrap_or(0);
        let limit = if limit == 0 { usize::MAX } else { limit };
        Ok(logs
            .get(sandbox_id)
            .map(|sandbox_entries| {
                sandbox_entries
                    .iter()
                    .skip(start)
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }
}
