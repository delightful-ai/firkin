use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use firkin_e2b_server::{DomainProxyHttpServer, LocalRuntimeBackend};
use firkin_envd::{
    EnvdProcessEventStream, EnvdProcessInfo, EnvdProcessInput, EnvdProcessOutput,
    EnvdProcessSelector, EnvdProcessSignal, EnvdProcessStartRequest, EnvdPtySize,
};
use firkin_types::Hostname;
use tokio::sync::Mutex;

use super::{
    ActiveSessionRecord, CommandOutput, CommandRequest, DiskPressureGuard, DomainProxyAdapter,
    Error, FileStateStore, PortRegistry, Result, RuntimeCreatedSandbox, RuntimeDriver,
    RuntimeSnapshotRef, SandboxSession, SandboxSessionState, SingleNodeConfig,
    SingleNodeCreateRequest, SingleNodeScheduler, SnapshotRecord, StateStore, TemplateMetadata,
};

/// Product-neutral facade for a local Apple/VZ-backed single-node runtime.
#[derive(Clone)]
pub struct SingleNodeBackend {
    config: SingleNodeConfig,
    scheduler: Arc<SingleNodeScheduler>,
    driver: Option<Arc<dyn RuntimeDriver>>,
    state: StateStore,
    files: Option<FileStateStore>,
    disk_pressure: Option<DiskPressureGuard>,
}

impl SingleNodeBackend {
    /// Construct a backend from explicit single-node runtime configuration.
    #[must_use]
    pub fn from_config(config: SingleNodeConfig) -> Self {
        let scheduler = Arc::new(SingleNodeScheduler::new(config.scheduler()));
        Self {
            config,
            scheduler,
            driver: None,
            state: StateStore::new(),
            files: None,
            disk_pressure: None,
        }
    }

    /// Construct a backend with a concrete runtime driver and file-backed state.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when runtime state cannot be
    /// initialized from the configured root directory.
    pub fn with_driver(config: SingleNodeConfig, driver: Arc<dyn RuntimeDriver>) -> Result<Self> {
        let scheduler = Arc::new(SingleNodeScheduler::new(config.scheduler()));
        Self::with_driver_scheduler(config, driver, scheduler)
    }

    /// Construct a backend with a concrete runtime driver, injected scheduler,
    /// and file-backed state.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when runtime state cannot be
    /// initialized from the configured root directory.
    pub fn with_driver_scheduler(
        config: SingleNodeConfig,
        driver: Arc<dyn RuntimeDriver>,
        scheduler: Arc<SingleNodeScheduler>,
    ) -> Result<Self> {
        let files = FileStateStore::new(config.root())?;
        let state = files.load_state()?;
        let backend = Self::with_driver_scheduler_and_state(config, driver, scheduler, state)
            .with_file_state_store(files)
            .with_disk_pressure_guard_from_config();
        backend.reconcile_active_state()?;
        Ok(backend)
    }

    /// Construct a backend with explicit in-memory state.
    #[must_use]
    pub fn with_driver_and_state(
        config: SingleNodeConfig,
        driver: Arc<dyn RuntimeDriver>,
        state: StateStore,
    ) -> Self {
        let scheduler = Arc::new(SingleNodeScheduler::new(config.scheduler()));
        Self::with_driver_scheduler_and_state(config, driver, scheduler, state)
    }

    /// Construct a backend with explicit scheduler and in-memory state.
    #[must_use]
    pub fn with_driver_scheduler_and_state(
        config: SingleNodeConfig,
        driver: Arc<dyn RuntimeDriver>,
        scheduler: Arc<SingleNodeScheduler>,
        state: StateStore,
    ) -> Self {
        Self {
            config,
            scheduler,
            driver: Some(driver),
            state,
            files: None,
            disk_pressure: None,
        }
    }

    /// Attach an explicit disk pressure guard.
    #[must_use]
    pub fn with_disk_pressure_guard(mut self, guard: DiskPressureGuard) -> Self {
        self.disk_pressure = Some(guard);
        self
    }

    /// Return the in-memory state store.
    #[must_use]
    pub const fn state(&self) -> &StateStore {
        &self.state
    }

    /// Return active session records.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be read.
    pub fn active_records(&self) -> Result<Vec<ActiveSessionRecord>> {
        self.state.load_active()
    }

    /// Return snapshot records.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StatePersistenceFailed`] when state cannot be read.
    pub fn snapshot_records(&self) -> Result<Vec<SnapshotRecord>> {
        self.state.load_snapshots()
    }

    /// Return the backend configuration.
    #[must_use]
    pub const fn config(&self) -> &SingleNodeConfig {
        &self.config
    }

    /// Return the runtime scheduler.
    #[must_use]
    pub fn scheduler(&self) -> &SingleNodeScheduler {
        &self.scheduler
    }

    /// Construct a domain proxy server for active single-node sandbox routes.
    ///
    /// The caller must pass the same [`PortRegistry`] that the runtime driver
    /// uses for sandbox-side port attachments.
    #[must_use]
    pub fn domain_proxy(
        &self,
        ports: PortRegistry,
        domain: Hostname,
    ) -> DomainProxyHttpServer<DomainProxyAdapter> {
        let backend = LocalRuntimeBackend::new(
            DomainProxyAdapter::new(ports),
            now_unix_seconds().to_string(),
        );
        DomainProxyHttpServer::new(Arc::new(Mutex::new(backend)), domain)
    }

    /// Create a new sandbox session.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when capacity, disk, state, or driver creation
    /// fails.
    pub async fn create(&self, request: SingleNodeCreateRequest) -> Result<RuntimeCreatedSandbox> {
        self.check_disk_pressure("sandbox create")?;
        let sandbox_id = request.sandbox_id().to_owned();
        let template_id = request.template_id().to_owned();
        let resources = *request.resources();
        let timeout = request.timeout().unwrap_or(Duration::from_mins(5));
        self.scheduler.admit(&sandbox_id, resources)?;
        let created = self.driver()?.create(request).await.inspect_err(|_| {
            let _ = self.scheduler.release(&sandbox_id);
        })?;
        let now = now_unix_seconds();
        if let Err(error) = self.upsert_active(ActiveSessionRecord {
            sandbox_id: created.sandbox_id.clone(),
            template_id,
            client_id: created.client_id.clone(),
            envd_access_token: created.envd_access_token.clone(),
            started_at_unix_seconds: now,
            end_at_unix_seconds: now + i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX),
            resources,
            runtime_attached: true,
        }) {
            let _ = self.driver()?.delete(&created.sandbox_id).await;
            let _ = self.scheduler.release(&sandbox_id);
            return Err(error);
        }
        Ok(created)
    }

    /// Restore a new sandbox session from a snapshot record.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when capacity, disk, state, or driver restore
    /// fails.
    pub async fn restore(
        &self,
        request: SingleNodeCreateRequest,
        snapshot: SnapshotRecord,
    ) -> Result<RuntimeCreatedSandbox> {
        self.check_disk_pressure("sandbox restore")?;
        let sandbox_id = request.sandbox_id().to_owned();
        let template_id = request.template_id().to_owned();
        let resources = *request.resources();
        let timeout = request.timeout().unwrap_or(Duration::from_mins(5));
        self.scheduler.admit(&sandbox_id, resources)?;
        let created = self
            .driver()?
            .restore(request, snapshot)
            .await
            .inspect_err(|_| {
                let _ = self.scheduler.release(&sandbox_id);
            })?;
        let now = now_unix_seconds();
        if let Err(error) = self.upsert_active(ActiveSessionRecord {
            sandbox_id: created.sandbox_id.clone(),
            template_id,
            client_id: created.client_id.clone(),
            envd_access_token: created.envd_access_token.clone(),
            started_at_unix_seconds: now,
            end_at_unix_seconds: now + i64::try_from(timeout.as_secs()).unwrap_or(i64::MAX),
            resources,
            runtime_attached: true,
        }) {
            let _ = self.driver()?.delete(&created.sandbox_id).await;
            let _ = self.scheduler.release(&sandbox_id);
            return Err(error);
        }
        Ok(created)
    }

    /// Delete an active sandbox session.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox does not exist, driver delete
    /// fails, or state cannot be persisted.
    pub async fn delete(&self, sandbox_id: &str) -> Result<()> {
        self.ensure_active(sandbox_id)?;
        self.driver()?.delete(sandbox_id).await?;
        self.remove_active(sandbox_id)?;
        self.scheduler.release(sandbox_id)?;
        Ok(())
    }

    /// Return a lightweight active-session view.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SandboxNotFound`] when no active session exists.
    pub fn get(&self, sandbox_id: &str) -> Result<SandboxSession> {
        self.ensure_active(sandbox_id)?;
        Ok(SandboxSession::new(
            sandbox_id,
            SandboxSessionState::Running,
        ))
    }

    /// Capture a runtime snapshot and record its metadata.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when disk, driver, or state persistence fails.
    pub async fn snapshot(
        &self,
        sandbox_id: &str,
        name: Option<String>,
    ) -> Result<RuntimeSnapshotRef> {
        self.snapshot_with_metadata(sandbox_id, name, TemplateMetadata::default())
            .await
    }

    /// Capture a runtime snapshot and record template metadata with it.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when disk, driver, or state persistence fails.
    pub async fn snapshot_with_metadata(
        &self,
        sandbox_id: &str,
        name: Option<String>,
        metadata: TemplateMetadata,
    ) -> Result<RuntimeSnapshotRef> {
        self.check_disk_pressure("sandbox snapshot")?;
        self.ensure_active(sandbox_id)?;
        let snapshot = self.driver()?.snapshot(sandbox_id, name.clone()).await?;
        if let Err(error) = self.upsert_snapshot(snapshot_record_from_ref(
            sandbox_id, name, metadata, &snapshot,
        )) {
            let _ = self.driver()?.delete_snapshot(&snapshot.snapshot_id).await;
            return Err(error);
        }
        Ok(snapshot)
    }

    /// Delete a runtime snapshot and its metadata.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the snapshot is not known, driver deletion
    /// fails, or state persistence fails.
    pub async fn delete_snapshot(&self, snapshot_id: &str) -> Result<()> {
        self.ensure_snapshot(snapshot_id)?;
        self.driver()?.delete_snapshot(snapshot_id).await?;
        self.remove_snapshot(snapshot_id)
    }

    /// Execute a foreground command in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox is missing or command execution
    /// fails.
    pub async fn run_command(
        &self,
        sandbox_id: &str,
        request: CommandRequest,
    ) -> Result<CommandOutput> {
        self.ensure_active(sandbox_id)?;
        self.driver()?.run_command(sandbox_id, request).await
    }

    /// Start a retained process stream in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox is missing or process start
    /// fails.
    pub async fn start_process_stream(
        &self,
        sandbox_id: &str,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<firkin_e2b_contract::BackendError>> {
        self.ensure_active(sandbox_id)?;
        self.driver()?
            .start_process_stream(sandbox_id, request)
            .await
    }

    /// List retained process records in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox is missing or process listing
    /// fails.
    pub async fn list_processes(&self, sandbox_id: &str) -> Result<Vec<EnvdProcessInfo>> {
        self.ensure_active(sandbox_id)?;
        self.driver()?.list_processes(sandbox_id).await
    }

    /// Connect to a retained process in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox or process is missing.
    pub async fn connect_process(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
    ) -> Result<EnvdProcessOutput> {
        self.ensure_active(sandbox_id)?;
        self.driver()?.connect_process(sandbox_id, selector).await
    }

    /// Send input to a retained process in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox or process is missing, or input
    /// cannot be delivered.
    pub async fn send_process_input(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
        input: EnvdProcessInput,
    ) -> Result<()> {
        self.ensure_active(sandbox_id)?;
        self.driver()?
            .send_process_input(sandbox_id, selector, input)
            .await
    }

    /// Close stdin for a retained process in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox or process is missing.
    pub async fn close_process_stdin(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
    ) -> Result<()> {
        self.ensure_active(sandbox_id)?;
        self.driver()?
            .close_process_stdin(sandbox_id, selector)
            .await
    }

    /// Signal a retained process in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox or process is missing.
    pub async fn signal_process(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
        signal: EnvdProcessSignal,
    ) -> Result<()> {
        self.ensure_active(sandbox_id)?;
        self.driver()?
            .signal_process(sandbox_id, selector, signal)
            .await
    }

    /// Resize a retained process PTY in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox or process is missing, or when
    /// the process has no PTY.
    pub async fn resize_process_pty(
        &self,
        sandbox_id: &str,
        selector: EnvdProcessSelector,
        pty: Option<EnvdPtySize>,
    ) -> Result<()> {
        self.ensure_active(sandbox_id)?;
        self.driver()?
            .resize_process_pty(sandbox_id, selector, pty)
            .await
    }

    /// Start a detached template initialization command in an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox is missing or command start
    /// fails.
    pub async fn start_template_command(
        &self,
        sandbox_id: &str,
        command: String,
        envs: HashMap<String, String>,
    ) -> Result<()> {
        self.ensure_active(sandbox_id)?;
        self.driver()?
            .start_template_command(sandbox_id, command, envs)
            .await
    }

    /// Apply template start and readiness commands to a restored sandbox.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox is missing, the start command
    /// fails to start, or the readiness command exits unsuccessfully.
    pub async fn run_template_start(
        &self,
        sandbox_id: &str,
        metadata: &TemplateMetadata,
        envs: HashMap<String, String>,
    ) -> Result<()> {
        self.ensure_active(sandbox_id)?;
        if let Some(command) = metadata.start_command() {
            self.driver()?
                .start_template_command(sandbox_id, command.to_owned(), envs.clone())
                .await?;
        }
        if let Some(command) = metadata.ready_command() {
            let mut request = CommandRequest::new(command).with_cwd("/");
            request.envs = envs;
            let output = self.driver()?.run_command(sandbox_id, request).await?;
            if !output.success() {
                return Err(Error::TemplateBuildFailed(format!(
                    "template ready command exited {}: {}",
                    output.exit_code,
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
        }
        Ok(())
    }

    /// Update the active session deadline.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the sandbox is missing or state cannot be
    /// persisted.
    pub fn update_deadline(&self, sandbox_id: &str, timeout_from_now: Duration) -> Result<()> {
        let now = now_unix_seconds();
        let end_at = now + i64::try_from(timeout_from_now.as_secs()).unwrap_or(i64::MAX);
        let original = self.state.load_active()?;
        let mut active = original.clone();
        let Some(session) = active
            .iter_mut()
            .find(|session| session.sandbox_id == sandbox_id)
        else {
            return Err(Error::SandboxNotFound(format!(
                "sandbox {sandbox_id} not found"
            )));
        };
        session.end_at_unix_seconds = end_at;
        self.state.save_active(active)?;
        self.persist_active().inspect_err(|_| {
            let _ = self.state.save_active(original);
        })
    }

    fn with_file_state_store(mut self, files: FileStateStore) -> Self {
        self.files = Some(files);
        self
    }

    fn with_disk_pressure_guard_from_config(mut self) -> Self {
        self.disk_pressure = Some(DiskPressureGuard::from_config(&self.config));
        self
    }

    fn driver(&self) -> Result<&dyn RuntimeDriver> {
        self.driver.as_deref().ok_or_else(|| {
            Error::UnsupportedCapability(
                "single-node backend has no runtime driver configured".to_owned(),
            )
        })
    }

    fn check_disk_pressure(&self, operation: &str) -> Result<()> {
        if let Some(guard) = &self.disk_pressure {
            guard.check(operation)?;
        }
        Ok(())
    }

    fn reconcile_active_state(&self) -> Result<()> {
        let now = now_unix_seconds();
        let active = self.state.reconcile_active_entries(now)?;
        let mut reconciled = Vec::with_capacity(active.len());
        for session in active {
            if !session.runtime_attached
                || self.driver()?.runtime_sandbox_exists(&session.sandbox_id)?
            {
                self.scheduler
                    .admit(&session.sandbox_id, session.resources)?;
                reconciled.push(session);
            }
        }
        self.state.save_active(reconciled)?;
        self.persist_active()
    }

    fn ensure_active(&self, sandbox_id: &str) -> Result<ActiveSessionRecord> {
        self.state
            .load_active()?
            .into_iter()
            .find(|session| session.sandbox_id == sandbox_id)
            .ok_or_else(|| Error::SandboxNotFound(format!("sandbox {sandbox_id} not found")))
    }

    fn upsert_active(&self, session: ActiveSessionRecord) -> Result<()> {
        let original = self.state.load_active()?;
        let mut active = original.clone();
        active.retain(|existing| existing.sandbox_id != session.sandbox_id);
        active.push(session);
        self.state.save_active(active)?;
        self.persist_active().inspect_err(|_| {
            let _ = self.state.save_active(original);
        })
    }

    fn remove_active(&self, sandbox_id: &str) -> Result<()> {
        let original = self.state.load_active()?;
        let mut active = original.clone();
        let original_len = active.len();
        active.retain(|session| session.sandbox_id != sandbox_id);
        if active.len() == original_len {
            return Err(Error::SandboxNotFound(format!(
                "sandbox {sandbox_id} not found"
            )));
        }
        self.state.save_active(active)?;
        self.persist_active().inspect_err(|_| {
            let _ = self.state.save_active(original);
        })
    }

    fn ensure_snapshot(&self, snapshot_id: &str) -> Result<SnapshotRecord> {
        self.state
            .load_snapshots()?
            .into_iter()
            .find(|snapshot| snapshot.snapshot_id == snapshot_id)
            .ok_or_else(|| Error::SnapshotNotFound(format!("snapshot {snapshot_id} not found")))
    }

    fn upsert_snapshot(&self, snapshot: SnapshotRecord) -> Result<()> {
        let original = self.state.load_snapshots()?;
        let mut snapshots = original.clone();
        snapshots.retain(|existing| existing.snapshot_id != snapshot.snapshot_id);
        snapshots.push(snapshot);
        self.state.save_snapshots(snapshots)?;
        self.persist_snapshots().inspect_err(|_| {
            let _ = self.state.save_snapshots(original);
        })
    }

    fn remove_snapshot(&self, snapshot_id: &str) -> Result<()> {
        let original = self.state.load_snapshots()?;
        let mut snapshots = original.clone();
        let original_len = snapshots.len();
        snapshots.retain(|snapshot| snapshot.snapshot_id != snapshot_id);
        if snapshots.len() == original_len {
            return Err(Error::SnapshotNotFound(format!(
                "snapshot {snapshot_id} not found"
            )));
        }
        self.state.save_snapshots(snapshots)?;
        self.persist_snapshots().inspect_err(|_| {
            let _ = self.state.save_snapshots(original);
        })
    }

    fn persist_active(&self) -> Result<()> {
        if let Some(files) = &self.files {
            let active = self.state.load_active()?;
            files.save_active(&active)?;
        }
        Ok(())
    }

    fn persist_snapshots(&self) -> Result<()> {
        if let Some(files) = &self.files {
            let snapshots = self.state.load_snapshots()?;
            files.save_snapshots(&snapshots)?;
        }
        Ok(())
    }
}

fn snapshot_record_from_ref(
    sandbox_id: &str,
    name: Option<String>,
    metadata: TemplateMetadata,
    snapshot: &RuntimeSnapshotRef,
) -> SnapshotRecord {
    let mut record = SnapshotRecord::new(
        snapshot.snapshot_id.clone(),
        snapshot
            .source_sandbox_id
            .clone()
            .unwrap_or_else(|| sandbox_id.to_owned()),
    );
    if let Some(name) = name {
        record.names.push(name);
    }
    record.location.clone_from(&snapshot.location);
    record.staging_dir.clone_from(&snapshot.staging_dir);
    record
        .machine_identifier
        .clone_from(&snapshot.machine_identifier);
    record.network_macs.clone_from(&snapshot.network_macs);
    record.template_metadata = metadata;
    record
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
