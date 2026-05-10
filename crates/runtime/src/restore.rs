//! restore — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::DEFAULT_RUNTIME_MINIMUM_FREE_DISK;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::continuation::PersistedContainerSnapshotState;
#[allow(unused_imports)]
use crate::disk::{
    DiskPressureError, DiskPressureProbe, HostDiskPressureProbe, RuntimeDiskPressureGuard,
};
#[allow(unused_imports)]
use crate::template_build::prepared_template_integrity;
#[allow(unused_imports)]
use async_trait::async_trait;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use base64::Engine as _;
#[allow(unused_imports)]
use firkin_admission::CapacityError;
#[allow(unused_imports)]
use firkin_admission::CapacityLedger;
#[allow(unused_imports)]
use firkin_artifacts::SnapshotArtifactIntegrity;
#[allow(unused_imports)]
use firkin_e2b_contract::BackendError;
#[allow(unused_imports)]
use firkin_e2b_contract::RuntimeSandbox;
#[allow(unused_imports)]
use firkin_e2b_contract::SandboxRuntimeConfig;
#[allow(unused_imports)]
use firkin_e2b_contract::{DEFAULT_CODE_INTERPRETER_PORT, DEFAULT_MCP_PORT, StartSandboxRequest};
#[allow(unused_imports)]
use firkin_envd::DEFAULT_ENVD_PORT;
#[allow(unused_imports)]
use firkin_template::SnapshotSinkError;
#[allow(unused_imports)]
use firkin_trace::BenchmarkSample;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use firkin_types::Size;
#[allow(unused_imports)]
use std::fmt::Display;
#[allow(unused_imports)]
use std::path::Path;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use std::path::PathBuf;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use std::sync::Arc;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use std::sync::Mutex as StdMutex;
#[allow(unused_imports)]
use std::time::Duration;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use std::time::{SystemTime, UNIX_EPOCH};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
#[allow(unused_imports)]
use {firkin_admission::ResourceBudget, firkin_artifacts::SnapshotArtifactManifest};
/// Request passed to the runtime snapshot restore launcher.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotRestoreRequest<'a> {
    pub(crate) manifest: &'a SnapshotArtifactManifest,
    pub(crate) budget: ResourceBudget,
}
impl<'a> SnapshotRestoreRequest<'a> {
    /// Construct a snapshot restore request.
    #[must_use]
    pub const fn new(manifest: &'a SnapshotArtifactManifest, budget: ResourceBudget) -> Self {
        Self { manifest, budget }
    }
    /// Return the snapshot manifest used as the restore source.
    #[must_use]
    pub const fn manifest(self) -> &'a SnapshotArtifactManifest {
        self.manifest
    }
    /// Return resources reserved for the restored session.
    #[must_use]
    pub const fn budget(self) -> ResourceBudget {
        self.budget
    }
}
/// Runtime-owned launcher for a session restored from a durable snapshot.
#[async_trait]
pub trait SnapshotSessionLauncher {
    /// Error returned by the launcher implementation.
    type Error;
    /// Restored session handle.
    type Session;
    /// Restore a session from the snapshot described by `request`.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific restore errors.
    async fn restore_from_snapshot(
        &mut self,
        request: &SnapshotRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error>;
}
/// Capacity reservation held by an active restored session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSessionReservation {
    pub(crate) budget: ResourceBudget,
    released: bool,
}
impl ActiveSessionReservation {
    /// Construct a reservation for an already-admitted active session.
    #[must_use]
    pub const fn new(budget: ResourceBudget) -> Self {
        Self {
            budget,
            released: false,
        }
    }
    /// Return reserved resources.
    #[must_use]
    pub const fn budget(&self) -> ResourceBudget {
        self.budget
    }
    /// Release this active reservation into `ledger`.
    ///
    /// Returns the released budget the first time it is called, and `None`
    /// after the reservation has already been released.
    pub fn release_into(&mut self, ledger: &mut CapacityLedger) -> Option<ResourceBudget> {
        if self.released {
            return None;
        }
        self.released = true;
        ledger.release_active(self.budget);
        Some(self.budget)
    }
}
/// Report from restoring a runtime session from a snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotRestoreReport<S> {
    pub(crate) session: S,
    pub(crate) reservation: ActiveSessionReservation,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
impl<S> SnapshotRestoreReport<S> {
    /// Construct a snapshot restore report.
    #[must_use]
    pub fn new(
        session: S,
        reservation: ActiveSessionReservation,
        benchmark_samples: Vec<BenchmarkSample>,
    ) -> Self {
        Self {
            session,
            reservation,
            benchmark_samples,
        }
    }
    /// Return the restored session handle.
    #[must_use]
    pub const fn session(&self) -> &S {
        &self.session
    }
    /// Return the active capacity reservation for the restored session.
    #[must_use]
    pub const fn reservation(&self) -> &ActiveSessionReservation {
        &self.reservation
    }
    /// Return benchmark samples recorded during restore.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
    /// Consume the report and return its session and reservation.
    #[must_use]
    pub fn into_parts(self) -> (S, ActiveSessionReservation) {
        (self.session, self.reservation)
    }
}
/// Snapshot restore orchestration error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum SnapshotRestoreError<E> {
    /// Snapshot artifact failed integrity verification before restore.
    #[error("snapshot restore integrity verification failed: {reason}")]
    Integrity {
        /// Integrity verification failure reason.
        reason: String,
    },
    /// Capacity admission failed before restore started.
    #[error("snapshot restore capacity admission failed: {0}")]
    Capacity(#[from] CapacityError),
    /// Runtime launcher failed after capacity was admitted.
    #[error("snapshot restore launcher failed: {source}")]
    Launch {
        /// Source launcher error.
        source: E,
    },
}
/// SDK-visible runtime config for a Cube/E2B sandbox create result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeCubeSandboxCreateConfig {
    #[allow(missing_docs)]
    pub sandbox_id: String,
    pub(crate) domain: String,
    pub(crate) envd_version: String,
    pub(crate) started_at: String,
    pub(crate) end_at: String,
    pub(crate) cpu_count: u32,
    pub(crate) memory_mb: u32,
}
impl RuntimeCubeSandboxCreateConfig {
    /// Construct a runtime Cube sandbox create config.
    #[must_use]
    pub fn new(
        sandbox_id: impl Into<String>,
        domain: impl Into<String>,
        envd_version: impl Into<String>,
        started_at: impl Into<String>,
        end_at: impl Into<String>,
        cpu_count: u32,
        memory_mb: u32,
    ) -> Self {
        Self {
            sandbox_id: sandbox_id.into(),
            domain: domain.into(),
            envd_version: envd_version.into(),
            started_at: started_at.into(),
            end_at: end_at.into(),
            cpu_count,
            memory_mb,
        }
    }
    pub(crate) fn into_runtime_config(self) -> SandboxRuntimeConfig {
        SandboxRuntimeConfig {
            sandbox_id: self.sandbox_id,
            domain: self.domain,
            envd_version: self.envd_version,
            envd_access_token: None,
            traffic_access_token: None,
            started_at: self.started_at,
            end_at: self.end_at,
            cpu_count: self.cpu_count,
            memory_mb: self.memory_mb,
        }
    }
}
/// Report from creating a Cube/E2B sandbox through Firkin snapshot restore.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeCubeSandboxCreateReport<S> {
    pub(crate) runtime_sandbox: RuntimeSandbox,
    pub(crate) session: S,
    pub(crate) reservation: ActiveSessionReservation,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
impl<S> RuntimeCubeSandboxCreateReport<S> {
    /// Construct a Cube sandbox create report.
    #[must_use]
    pub fn new(
        runtime_sandbox: RuntimeSandbox,
        session: S,
        reservation: ActiveSessionReservation,
        benchmark_samples: Vec<BenchmarkSample>,
    ) -> Self {
        Self {
            runtime_sandbox,
            session,
            reservation,
            benchmark_samples,
        }
    }
    /// Return the SDK-visible runtime sandbox config and exposed ports.
    #[must_use]
    pub const fn runtime_sandbox(&self) -> &RuntimeSandbox {
        &self.runtime_sandbox
    }
    /// Return the restored Firkin session handle.
    #[must_use]
    pub const fn session(&self) -> &S {
        &self.session
    }
    /// Return the active capacity reservation for the restored session.
    #[must_use]
    pub const fn reservation(&self) -> &ActiveSessionReservation {
        &self.reservation
    }
    /// Return benchmark samples recorded during create.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
    /// Consume the report and return its restored session and active reservation.
    #[must_use]
    pub fn into_session_reservation(self) -> (S, ActiveSessionReservation) {
        (self.session, self.reservation)
    }
}
/// Cube/E2B snapshot-backed sandbox create error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum RuntimeCubeSandboxCreateError<E> {
    /// The E2B start request did not include a prepared template snapshot.
    #[error("snapshot-backed Cube sandbox create requires a prepared template")]
    MissingPreparedTemplate,
    /// The prepared template snapshot did not include expected artifact integrity.
    #[error("snapshot-backed Cube sandbox create requires prepared template artifact integrity")]
    MissingPreparedTemplateIntegrity,
    /// Snapshot restore failed.
    #[error(transparent)]
    Restore(#[from] SnapshotRestoreError<E>),
}
pub(crate) fn runtime_create_error_to_backend<E>(
    error: RuntimeCubeSandboxCreateError<E>,
) -> BackendError
where
    E: Display,
{
    match error {
        RuntimeCubeSandboxCreateError::MissingPreparedTemplate => BackendError::Runtime(
            "Firkin RuntimeAdapter start requires a prepared template snapshot".to_owned(),
        ),
        RuntimeCubeSandboxCreateError::MissingPreparedTemplateIntegrity => BackendError::Runtime(
            "Firkin RuntimeAdapter start requires prepared template artifact integrity".to_owned(),
        ),
        RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::Capacity(error)) => {
            BackendError::Runtime(error.to_string())
        }
        RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::Integrity { reason }) => {
            BackendError::Runtime(format!("Firkin snapshot integrity check failed: {reason}"))
        }
        RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::Launch { source }) => {
            BackendError::Runtime(format!("Firkin snapshot restore failed: {source}"))
        }
    }
}
#[cfg(feature = "snapshot")]
pub(crate) fn snapshot_restore_error_to_backend<E>(error: SnapshotRestoreError<E>) -> BackendError
where
    E: Display,
{
    match error {
        SnapshotRestoreError::Integrity { reason } => {
            BackendError::Runtime(format!("Firkin snapshot integrity check failed: {reason}"))
        }
        SnapshotRestoreError::Capacity(error) => BackendError::Runtime(error.to_string()),
        SnapshotRestoreError::Launch { source } => {
            BackendError::Runtime(format!("Firkin snapshot restore failed: {source}"))
        }
    }
}
/// Runtime-level Cube/E2B sandbox create operation backed by snapshot restore.
#[derive(Debug)]
pub struct RuntimeCubeSandboxCreate<'a> {
    pub(crate) ledger: &'a mut CapacityLedger,
    pub(crate) request: &'a StartSandboxRequest,
    pub(crate) budget: ResourceBudget,
    #[allow(missing_docs)]
    pub config: RuntimeCubeSandboxCreateConfig,
}
impl<'a> RuntimeCubeSandboxCreate<'a> {
    /// Construct a snapshot-backed Cube/E2B sandbox create operation.
    #[must_use]
    pub const fn new(
        ledger: &'a mut CapacityLedger,
        request: &'a StartSandboxRequest,
        budget: ResourceBudget,
        config: RuntimeCubeSandboxCreateConfig,
    ) -> Self {
        Self {
            ledger,
            request,
            budget,
            config,
        }
    }
    /// Restore the prepared template snapshot and return the SDK-visible runtime sandbox shape.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when capacity admission or launch fails.
    pub async fn execute_with_elapsed<L>(
        self,
        launcher: &mut L,
        elapsed: Duration,
    ) -> Result<RuntimeCubeSandboxCreateReport<L::Session>, RuntimeCubeSandboxCreateError<L::Error>>
    where
        L: SnapshotSessionLauncher,
    {
        let mut probe = HostDiskPressureProbe::new();
        self.execute_with_disk_probe_elapsed(launcher, elapsed, &mut probe)
            .await
    }
    /// Restore the prepared template snapshot with a caller-provided disk probe.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when capacity admission or launch fails.
    pub async fn execute_with_disk_probe_elapsed<L, P>(
        self,
        launcher: &mut L,
        elapsed: Duration,
        disk_probe: &mut P,
    ) -> Result<RuntimeCubeSandboxCreateReport<L::Session>, RuntimeCubeSandboxCreateError<L::Error>>
    where
        L: SnapshotSessionLauncher,
        P: DiskPressureProbe,
    {
        let template = self
            .request
            .prepared_template
            .as_ref()
            .ok_or(RuntimeCubeSandboxCreateError::MissingPreparedTemplate)?;
        let manifest =
            SnapshotArtifactManifest::base(template.template_id.clone(), template.artifact.clone());
        let integrity = prepared_template_integrity(template, &manifest)?;
        let report = RuntimeSnapshotRestore::new(self.ledger, &manifest, self.budget)
            .execute_with_integrity_disk_probe_elapsed(launcher, elapsed, &integrity, disk_probe)
            .await?;
        let benchmark_samples = report.benchmark_samples().to_vec();
        let (session, reservation) = report.into_parts();
        let runtime_sandbox = RuntimeSandbox {
            config: self.config.into_runtime_config(),
            exposed_ports: vec![
                DEFAULT_ENVD_PORT,
                DEFAULT_CODE_INTERPRETER_PORT,
                DEFAULT_MCP_PORT,
            ],
        };
        Ok(RuntimeCubeSandboxCreateReport::new(
            runtime_sandbox,
            session,
            reservation,
            benchmark_samples,
        ))
    }
}
/// Runtime-level snapshot restore orchestrator.
#[derive(Debug)]
pub struct RuntimeSnapshotRestore<'a> {
    pub(crate) ledger: &'a mut CapacityLedger,
    pub(crate) manifest: &'a SnapshotArtifactManifest,
    pub(crate) budget: ResourceBudget,
}
impl<'a> RuntimeSnapshotRestore<'a> {
    /// Construct a runtime-level snapshot restore operation.
    pub fn new(
        ledger: &'a mut CapacityLedger,
        manifest: &'a SnapshotArtifactManifest,
        budget: ResourceBudget,
    ) -> Self {
        Self {
            ledger,
            manifest,
            budget,
        }
    }
    /// Execute restore with a caller-provided elapsed time sample.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when capacity admission or launch fails.
    pub async fn execute_with_elapsed<L>(
        self,
        launcher: &mut L,
        elapsed: Duration,
    ) -> Result<SnapshotRestoreReport<L::Session>, SnapshotRestoreError<L::Error>>
    where
        L: SnapshotSessionLauncher,
    {
        let mut probe = HostDiskPressureProbe::new();
        self.execute_with_disk_probe_elapsed(launcher, elapsed, &mut probe)
            .await
    }
    /// Execute restore after verifying snapshot artifact integrity.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when integrity verification, disk
    /// pressure, capacity admission, or launch fails.
    pub async fn execute_with_integrity_elapsed<L>(
        self,
        launcher: &mut L,
        elapsed: Duration,
        integrity: &SnapshotArtifactIntegrity,
    ) -> Result<SnapshotRestoreReport<L::Session>, SnapshotRestoreError<L::Error>>
    where
        L: SnapshotSessionLauncher,
    {
        let mut probe = HostDiskPressureProbe::new();
        self.execute_with_integrity_disk_probe_elapsed(launcher, elapsed, integrity, &mut probe)
            .await
    }
    /// Execute restore after reading and verifying snapshot integrity sidecar.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when sidecar reading, integrity
    /// verification, disk pressure, capacity admission, or launch fails.
    pub async fn execute_with_integrity_sidecar_elapsed<L>(
        self,
        launcher: &mut L,
        elapsed: Duration,
        integrity_path: impl AsRef<Path>,
    ) -> Result<SnapshotRestoreReport<L::Session>, SnapshotRestoreError<L::Error>>
    where
        L: SnapshotSessionLauncher,
    {
        let mut probe = HostDiskPressureProbe::new();
        self.execute_with_integrity_sidecar_disk_probe_elapsed(
            launcher,
            elapsed,
            integrity_path,
            &mut probe,
        )
        .await
    }
    /// Execute restore with a caller-provided elapsed time sample and disk probe.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when disk pressure, capacity admission,
    /// or launch fails.
    pub async fn execute_with_disk_probe_elapsed<L, P>(
        self,
        launcher: &mut L,
        elapsed: Duration,
        disk_probe: &mut P,
    ) -> Result<SnapshotRestoreReport<L::Session>, SnapshotRestoreError<L::Error>>
    where
        L: SnapshotSessionLauncher,
        P: DiskPressureProbe,
    {
        let disk_root = self.manifest.path().parent().unwrap_or(Path::new("/"));
        RuntimeDiskPressureGuard::new(disk_root, DEFAULT_RUNTIME_MINIMUM_FREE_DISK)
            .check(disk_probe)
            .map_err(|error| disk_pressure_to_capacity_error(&error))?;
        self.ledger.reserve_active(self.budget)?;
        let request = SnapshotRestoreRequest::new(self.manifest, self.budget);
        let session = match launcher.restore_from_snapshot(&request).await {
            Ok(session) => session,
            Err(source) => {
                self.ledger.release_active(self.budget);
                return Err(SnapshotRestoreError::Launch { source });
            }
        };
        let sample = BenchmarkSample::new(
            "warm_snapshot_restore",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            elapsed.as_secs_f64() * 1000.0,
        );
        Ok(SnapshotRestoreReport::new(
            session,
            ActiveSessionReservation::new(self.budget),
            vec![sample],
        ))
    }
    /// Execute restore after verifying snapshot artifact integrity.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when integrity verification, disk
    /// pressure, capacity admission, or launch fails.
    pub async fn execute_with_integrity_disk_probe_elapsed<L, P>(
        self,
        launcher: &mut L,
        elapsed: Duration,
        integrity: &SnapshotArtifactIntegrity,
        disk_probe: &mut P,
    ) -> Result<SnapshotRestoreReport<L::Session>, SnapshotRestoreError<L::Error>>
    where
        L: SnapshotSessionLauncher,
        P: DiskPressureProbe,
    {
        integrity
            .verify(self.manifest)
            .map_err(|error| SnapshotRestoreError::Integrity {
                reason: error.to_string(),
            })?;
        self.execute_with_disk_probe_elapsed(launcher, elapsed, disk_probe)
            .await
    }
    /// Execute restore after reading a snapshot integrity sidecar.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when sidecar reading, integrity
    /// verification, disk pressure, capacity admission, or launch fails.
    pub async fn execute_with_integrity_sidecar_disk_probe_elapsed<L, P>(
        self,
        launcher: &mut L,
        elapsed: Duration,
        integrity_path: impl AsRef<Path>,
        disk_probe: &mut P,
    ) -> Result<SnapshotRestoreReport<L::Session>, SnapshotRestoreError<L::Error>>
    where
        L: SnapshotSessionLauncher,
        P: DiskPressureProbe,
    {
        let integrity = SnapshotArtifactIntegrity::read_json(integrity_path).map_err(|error| {
            SnapshotRestoreError::Integrity {
                reason: error.to_string(),
            }
        })?;
        self.execute_with_integrity_disk_probe_elapsed(launcher, elapsed, &integrity, disk_probe)
            .await
    }
}
pub(crate) fn disk_pressure_to_capacity_error<E>(error: &DiskPressureError<E>) -> CapacityError {
    match error {
        DiskPressureError::BelowMinimum { minimum, available } => CapacityError::Disk {
            requested: *minimum,
            available: *available,
        },
        DiskPressureError::Probe { .. } => CapacityError::Disk {
            requested: DEFAULT_RUNTIME_MINIMUM_FREE_DISK,
            available: Size::bytes(0),
        },
    }
}
pub(crate) fn snapshot_output_disk_root(snapshot_path: &Path) -> &Path {
    snapshot_path.parent().unwrap_or(Path::new("/"))
}
pub(crate) fn write_snapshot_artifact_sidecars(
    manifest: &SnapshotArtifactManifest,
) -> Result<(), SnapshotSinkError> {
    manifest
        .write_json(SnapshotArtifactManifest::sidecar_path_for_artifact(
            manifest.path(),
        ))
        .map_err(|source| Box::new(source) as SnapshotSinkError)?;
    let integrity = SnapshotArtifactIntegrity::from_file(manifest)
        .map_err(|source| Box::new(source) as SnapshotSinkError)?;
    integrity
        .write_json(SnapshotArtifactIntegrity::sidecar_path_for_artifact(
            manifest.path(),
        ))
        .map_err(|source| Box::new(source) as SnapshotSinkError)
}
/// Error returned by the live core snapshot restore launcher.
#[cfg(feature = "snapshot")]
#[derive(Debug, ThisError)]
pub enum CoreSnapshotSessionLaunchError {
    /// Failed to read persisted snapshot restore state.
    #[error("failed to read persisted snapshot state at {path}: {source}")]
    ReadState {
        /// State sidecar path.
        path: PathBuf,
        /// Source I/O error.
        source: std::io::Error,
    },
    /// Failed to decode persisted snapshot restore state.
    #[error("failed to decode persisted snapshot state at {path}: {source}")]
    DecodeState {
        /// State sidecar path.
        path: PathBuf,
        /// Source JSON error.
        source: serde_json::Error,
    },
    /// Core restore failed.
    #[error("core snapshot restore failed: {source}")]
    Core {
        /// Source core runtime error.
        source: firkin_core::Error,
    },
}
/// Live VZ-backed launcher for restoring an implicit-VM container snapshot.
#[cfg(feature = "snapshot")]
#[derive(Clone, Debug)]
pub struct CoreSnapshotSessionLauncher {
    builder: firkin_core::ContainerBuilder<firkin_core::ImplicitVm, firkin_core::Ready>,
    pub(crate) state_path: Option<PathBuf>,
    restore_staging_root: PathBuf,
    timing_samples: Option<Arc<StdMutex<Vec<firkin_core::ContainerRestoreTimings>>>>,
}
#[cfg(feature = "snapshot")]
impl CoreSnapshotSessionLauncher {
    /// Construct a one-shot launcher from a ready implicit-VM container builder.
    #[must_use]
    pub fn new(
        builder: firkin_core::ContainerBuilder<firkin_core::ImplicitVm, firkin_core::Ready>,
    ) -> Self {
        Self {
            builder,
            state_path: None,
            restore_staging_root: crate::default_runtime_restore_staging_root(),
            timing_samples: None,
        }
    }
    /// Read persisted restore state from `state_path` instead of deriving it
    /// from the snapshot path.
    #[must_use]
    pub fn with_state_path(mut self, state_path: impl Into<PathBuf>) -> Self {
        self.state_path = Some(state_path.into());
        self
    }
    /// Place restored sessions under a persistent staging root.
    ///
    /// Restored sessions must use persistent staging when they may later become
    /// continuation snapshot sources.
    #[must_use]
    pub fn with_restore_staging_root(mut self, restore_staging_root: impl Into<PathBuf>) -> Self {
        self.restore_staging_root = restore_staging_root.into();
        self
    }
    /// Record restore phase timings into `timing_samples`.
    #[must_use]
    pub fn with_timing_samples(
        mut self,
        timing_samples: Arc<StdMutex<Vec<firkin_core::ContainerRestoreTimings>>>,
    ) -> Self {
        self.timing_samples = Some(timing_samples);
        self
    }
    /// Return the state path that will be read for `snapshot_path`.
    #[must_use]
    pub fn state_path_for_snapshot(&self, snapshot_path: &Path) -> PathBuf {
        self.state_path
            .clone()
            .unwrap_or_else(|| snapshot_path.with_extension("state.json"))
    }
    /// Return the persistent staging directory used for one restore attempt.
    #[must_use]
    pub fn restore_staging_path_for_snapshot(&self, snapshot_path: &Path) -> PathBuf {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(snapshot_path.as_os_str().as_encoded_bytes());
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        self.restore_staging_root.join(format!(
            "restore-{encoded}-{}-{timestamp}",
            std::process::id()
        ))
    }
    fn read_state(
        &self,
        snapshot_path: &Path,
    ) -> Result<PersistedContainerSnapshotState, CoreSnapshotSessionLaunchError> {
        let state_path = self.state_path_for_snapshot(snapshot_path);
        let bytes = std::fs::read(&state_path).map_err(|source| {
            CoreSnapshotSessionLaunchError::ReadState {
                path: state_path.clone(),
                source,
            }
        })?;
        serde_json::from_slice(&bytes).map_err(|source| {
            CoreSnapshotSessionLaunchError::DecodeState {
                path: state_path,
                source,
            }
        })
    }
}
#[cfg(feature = "snapshot")]
#[async_trait]
impl SnapshotSessionLauncher for CoreSnapshotSessionLauncher {
    type Error = CoreSnapshotSessionLaunchError;
    type Session = firkin_core::Container<firkin_core::Streams>;
    async fn restore_from_snapshot(
        &mut self,
        request: &SnapshotRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        let snapshot_path = request.manifest().path();
        let persisted = self.read_state(snapshot_path)?;
        let state = persisted.to_snapshot_state();
        let restore_staging_path = self.restore_staging_path_for_snapshot(snapshot_path);
        if let Some(timing_samples) = self.timing_samples.as_ref() {
            let restore = self
                .builder
                .clone()
                .restore_from_snapshot_state_with_staging_dir_and_timings(
                    snapshot_path,
                    restore_staging_path,
                    &state,
                )
                .await
                .map_err(|source| CoreSnapshotSessionLaunchError::Core { source })?;
            let (container, timings) = restore.into_parts();
            timing_samples
                .lock()
                .expect("lock restore timing samples")
                .push(timings);
            Ok(container)
        } else {
            self.builder
                .clone()
                .restore_from_snapshot_state_with_staging_dir(
                    snapshot_path,
                    restore_staging_path,
                    &state,
                )
                .await
                .map_err(|source| CoreSnapshotSessionLaunchError::Core { source })
        }
    }
}
