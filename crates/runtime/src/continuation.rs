//! continuation — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::DEFAULT_RUNTIME_MINIMUM_FREE_DISK;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::disk::{DiskPressureProbe, HostDiskPressureProbe, RuntimeDiskPressureGuard};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::restore::{
    ActiveSessionReservation, RuntimeCubeSandboxCreateConfig, RuntimeSnapshotRestore,
    SnapshotRestoreError, SnapshotSessionLauncher, disk_pressure_to_capacity_error,
    snapshot_output_disk_root, write_snapshot_artifact_sidecars,
};
#[allow(unused_imports)]
use async_trait::async_trait;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use base64::Engine as _;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_admission::CapacityError;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_admission::{CapacityLedger, ResourceBudget};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_artifacts::ContinuationSnapshotPlan;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_artifacts::ContinuationSnapshotReason;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_artifacts::SnapshotArtifactManifest;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_e2b_contract::RuntimeSandbox;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_e2b_contract::StartSandboxRequest;
#[allow(unused_imports)]
use firkin_e2b_contract::{DEFAULT_CODE_INTERPRETER_PORT, DEFAULT_MCP_PORT};
#[allow(unused_imports)]
use firkin_envd::DEFAULT_ENVD_PORT;
#[allow(unused_imports)]
use firkin_template::SnapshotSinkError;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_template::TemplateSnapshotSink;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_trace::BenchmarkSample;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use std::time::Duration;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Persisted restore state that must live beside a saved VM snapshot.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PersistedContainerSnapshotState {
    staging_dir: PathBuf,
    machine_identifier: Vec<u8>,
    network_macs: Vec<String>,
}
impl PersistedContainerSnapshotState {
    /// Construct persisted restore state from a live core snapshot state value.
    #[cfg(feature = "snapshot")]
    #[must_use]
    pub fn from_snapshot_state(state: &firkin_core::ContainerSnapshotState) -> Self {
        Self {
            staging_dir: state.staging_dir().to_path_buf(),
            machine_identifier: state.machine_identifier().to_vec(),
            network_macs: state.network_macs().to_vec(),
        }
    }
    /// Reconstruct the core restore state.
    #[cfg(feature = "snapshot")]
    #[must_use]
    pub fn to_snapshot_state(&self) -> firkin_core::ContainerSnapshotState {
        firkin_core::ContainerSnapshotState::new(
            self.staging_dir.clone(),
            self.machine_identifier.clone(),
            self.network_macs.clone(),
        )
    }
    /// Return the persistent staging directory that backs the VM block devices.
    #[must_use]
    pub fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }
    /// Return the opaque Virtualization.framework machine identifier.
    #[must_use]
    pub fn machine_identifier(&self) -> &[u8] {
        &self.machine_identifier
    }
    /// Return guest network MAC addresses in network declaration order.
    #[must_use]
    pub fn network_macs(&self) -> &[String] {
        &self.network_macs
    }
}
/// Runtime session that can persist a durable continuation snapshot.
#[async_trait]
pub trait RuntimeContinuationSnapshotSource: Send + Sync {
    /// Save continuation snapshot bytes and any sidecar state required for restore.
    ///
    /// # Errors
    ///
    /// Returns snapshot sink errors from the runtime backend.
    async fn save_continuation_snapshot(&self, path: &Path) -> Result<(), SnapshotSinkError>;
    /// Remove restored staging that is not retained by a continuation snapshot.
    ///
    /// # Errors
    ///
    /// Returns cleanup errors from the runtime backend.
    async fn cleanup_unsnapshotted_staging(&self) -> Result<(), SnapshotSinkError> {
        Ok(())
    }
}
#[cfg(feature = "snapshot")]
#[async_trait]
impl<S> RuntimeContinuationSnapshotSource for firkin_core::Container<S>
where
    S: firkin_core::ContainerStdio + Send + Sync,
{
    async fn save_continuation_snapshot(&self, path: &Path) -> Result<(), SnapshotSinkError> {
        CoreContainerSnapshotSink::new(self)
            .save_snapshot(path)
            .await
    }
    async fn cleanup_unsnapshotted_staging(&self) -> Result<(), SnapshotSinkError> {
        let state = self
            .snapshot_state()
            .await
            .map_err(|source| Box::new(source) as SnapshotSinkError)?;
        match std::fs::remove_dir_all(state.staging_dir()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Box::new(source) as SnapshotSinkError),
        }
    }
}
/// SDK-visible runtime config for a Cube/E2B follow-up sandbox create result.
#[cfg(feature = "snapshot")]
pub type RuntimeCubeSandboxFollowupConfig = RuntimeCubeSandboxCreateConfig;
#[cfg(feature = "snapshot")]
pub(crate) fn runtime_continuation_snapshot_path(snapshot_id: &str) -> PathBuf {
    let root = crate::default_runtime_continuation_root();
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(snapshot_id.as_bytes());
    root.join(format!("{encoded}.vz"))
}
/// Report from capturing a runtime continuation snapshot.
#[cfg(feature = "snapshot")]
#[derive(Clone, Debug, PartialEq)]
pub struct ContinuationSnapshotReport {
    pub(crate) manifest: SnapshotArtifactManifest,
    pub(crate) reason: ContinuationSnapshotReason,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
#[cfg(feature = "snapshot")]
impl ContinuationSnapshotReport {
    /// Construct a continuation snapshot report.
    #[must_use]
    pub fn new(
        manifest: SnapshotArtifactManifest,
        reason: ContinuationSnapshotReason,
        benchmark_samples: Vec<BenchmarkSample>,
    ) -> Self {
        Self {
            manifest,
            reason,
            benchmark_samples,
        }
    }
    /// Return the captured continuation snapshot manifest.
    #[must_use]
    pub const fn manifest(&self) -> &SnapshotArtifactManifest {
        &self.manifest
    }
    /// Return why the continuation snapshot was captured.
    #[must_use]
    pub const fn reason(&self) -> ContinuationSnapshotReason {
        self.reason
    }
    /// Return benchmark samples recorded during snapshot capture.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
}
/// Runtime continuation snapshot capture error.
#[cfg(feature = "snapshot")]
#[derive(Debug, ThisError)]
pub enum ContinuationSnapshotError {
    /// Disk/capacity admission failed before snapshot capture started.
    #[error("continuation snapshot capacity admission failed: {0}")]
    Capacity(#[from] CapacityError),
    /// Snapshot sink failed.
    #[error("continuation snapshot sink failed: {source}")]
    Snapshot {
        /// Source sink error.
        #[source]
        source: SnapshotSinkError,
    },
}
/// Report from restoring a runtime session from a continuation snapshot.
#[cfg(feature = "snapshot")]
#[derive(Clone, Debug, PartialEq)]
pub struct ContinuationSnapshotRestoreReport<S> {
    pub(crate) session: S,
    pub(crate) reason: ContinuationSnapshotReason,
    pub(crate) reservation: ActiveSessionReservation,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
#[cfg(feature = "snapshot")]
impl<S> ContinuationSnapshotRestoreReport<S> {
    /// Construct a continuation snapshot restore report.
    #[must_use]
    pub fn new(
        session: S,
        reason: ContinuationSnapshotReason,
        reservation: ActiveSessionReservation,
        benchmark_samples: Vec<BenchmarkSample>,
    ) -> Self {
        Self {
            session,
            reason,
            reservation,
            benchmark_samples,
        }
    }
    /// Return the restored session handle.
    #[must_use]
    pub const fn session(&self) -> &S {
        &self.session
    }
    /// Return why the continuation snapshot was captured.
    #[must_use]
    pub const fn reason(&self) -> ContinuationSnapshotReason {
        self.reason
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
    /// Consume the report and return its session, reason, reservation, and samples.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        S,
        ContinuationSnapshotReason,
        ActiveSessionReservation,
        Vec<BenchmarkSample>,
    ) {
        (
            self.session,
            self.reason,
            self.reservation,
            self.benchmark_samples,
        )
    }
}
/// Report from creating a Cube/E2B follow-up sandbox through continuation restore.
#[cfg(feature = "snapshot")]
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeCubeSandboxFollowupReport<S> {
    pub(crate) runtime_sandbox: RuntimeSandbox,
    pub(crate) session: S,
    pub(crate) reason: ContinuationSnapshotReason,
    pub(crate) reservation: ActiveSessionReservation,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
#[cfg(feature = "snapshot")]
impl<S> RuntimeCubeSandboxFollowupReport<S> {
    /// Construct a Cube follow-up create report.
    #[must_use]
    pub fn new(
        runtime_sandbox: RuntimeSandbox,
        session: S,
        reason: ContinuationSnapshotReason,
        reservation: ActiveSessionReservation,
        benchmark_samples: Vec<BenchmarkSample>,
    ) -> Self {
        Self {
            runtime_sandbox,
            session,
            reason,
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
    /// Return why the continuation snapshot was captured.
    #[must_use]
    pub const fn reason(&self) -> ContinuationSnapshotReason {
        self.reason
    }
    /// Return the active capacity reservation for the restored session.
    #[must_use]
    pub const fn reservation(&self) -> &ActiveSessionReservation {
        &self.reservation
    }
    /// Return benchmark samples recorded during follow-up create.
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
/// Runtime-level Cube/E2B follow-up create backed by continuation snapshot restore.
#[cfg(feature = "snapshot")]
#[derive(Debug)]
pub struct RuntimeCubeSandboxFollowup<'a> {
    pub(crate) ledger: &'a mut CapacityLedger,
    pub(crate) request: &'a StartSandboxRequest,
    pub(crate) plan: &'a ContinuationSnapshotPlan,
    pub(crate) budget: ResourceBudget,
    #[allow(missing_docs)]
    pub config: RuntimeCubeSandboxFollowupConfig,
}
#[cfg(feature = "snapshot")]
impl<'a> RuntimeCubeSandboxFollowup<'a> {
    /// Construct a continuation-backed Cube/E2B follow-up create operation.
    #[must_use]
    pub const fn new(
        ledger: &'a mut CapacityLedger,
        request: &'a StartSandboxRequest,
        plan: &'a ContinuationSnapshotPlan,
        budget: ResourceBudget,
        config: RuntimeCubeSandboxFollowupConfig,
    ) -> Self {
        Self {
            ledger,
            request,
            plan,
            budget,
            config,
        }
    }
    /// Restore the continuation snapshot and return the SDK-visible runtime sandbox shape.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when capacity admission or launch fails.
    pub async fn execute_with_elapsed<L>(
        self,
        launcher: &mut L,
        elapsed: Duration,
    ) -> Result<RuntimeCubeSandboxFollowupReport<L::Session>, SnapshotRestoreError<L::Error>>
    where
        L: SnapshotSessionLauncher,
    {
        let _ = self.request;
        let report = RuntimeContinuationSnapshotRestore::new(self.ledger, self.plan, self.budget)
            .execute_with_elapsed(launcher, elapsed)
            .await?;
        let runtime_sandbox = RuntimeSandbox {
            config: self.config.into_runtime_config(),
            exposed_ports: vec![
                DEFAULT_ENVD_PORT,
                DEFAULT_CODE_INTERPRETER_PORT,
                DEFAULT_MCP_PORT,
            ],
        };
        let (session, reason, reservation, benchmark_samples) = report.into_parts();
        Ok(RuntimeCubeSandboxFollowupReport::new(
            runtime_sandbox,
            session,
            reason,
            reservation,
            benchmark_samples,
        ))
    }
}
/// Runtime-level continuation snapshot capture orchestrator.
#[cfg(feature = "snapshot")]
#[derive(Debug)]
pub struct RuntimeContinuationSnapshotCapture<'a> {
    pub(crate) plan: &'a ContinuationSnapshotPlan,
}
#[cfg(feature = "snapshot")]
impl<'a> RuntimeContinuationSnapshotCapture<'a> {
    /// Construct a runtime-level continuation snapshot capture operation.
    #[must_use]
    pub const fn new(plan: &'a ContinuationSnapshotPlan) -> Self {
        Self { plan }
    }
    /// Save a continuation snapshot and record snapshot-save latency.
    ///
    /// # Errors
    ///
    /// Returns [`ContinuationSnapshotError`] when the snapshot sink fails.
    pub async fn execute_with_elapsed<S>(
        self,
        snapshot_sink: &S,
        elapsed: Duration,
    ) -> Result<ContinuationSnapshotReport, ContinuationSnapshotError>
    where
        S: TemplateSnapshotSink,
    {
        let mut probe = HostDiskPressureProbe::new();
        self.execute_with_disk_probe_elapsed(snapshot_sink, elapsed, &mut probe)
            .await
    }
    /// Save a continuation snapshot after checking host disk pressure.
    ///
    /// # Errors
    ///
    /// Returns [`ContinuationSnapshotError`] when disk admission or snapshot
    /// capture fails.
    pub async fn execute_with_disk_probe_elapsed<S, P>(
        self,
        snapshot_sink: &S,
        elapsed: Duration,
        disk_probe: &mut P,
    ) -> Result<ContinuationSnapshotReport, ContinuationSnapshotError>
    where
        S: TemplateSnapshotSink,
        P: DiskPressureProbe,
    {
        let disk_root = snapshot_output_disk_root(self.plan.snapshot_output_path());
        RuntimeDiskPressureGuard::new(disk_root, DEFAULT_RUNTIME_MINIMUM_FREE_DISK)
            .check(disk_probe)
            .map_err(|error| {
                ContinuationSnapshotError::Capacity(disk_pressure_to_capacity_error(&error))
            })?;
        snapshot_sink
            .save_snapshot(self.plan.snapshot_output_path())
            .await
            .map_err(|source| ContinuationSnapshotError::Snapshot { source })?;
        let manifest = self.plan.snapshot_manifest();
        write_snapshot_artifact_sidecars(&manifest)
            .map_err(|source| ContinuationSnapshotError::Snapshot { source })?;
        let sample = BenchmarkSample::new(
            "snapshot_save",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            elapsed.as_secs_f64() * 1000.0,
        );
        Ok(ContinuationSnapshotReport::new(
            manifest,
            self.plan.reason(),
            vec![sample],
        ))
    }
}
/// Runtime-level continuation snapshot restore orchestrator.
#[cfg(feature = "snapshot")]
#[derive(Debug)]
pub struct RuntimeContinuationSnapshotRestore<'a> {
    pub(crate) ledger: &'a mut CapacityLedger,
    pub(crate) plan: &'a ContinuationSnapshotPlan,
    pub(crate) budget: ResourceBudget,
}
#[cfg(feature = "snapshot")]
impl<'a> RuntimeContinuationSnapshotRestore<'a> {
    /// Construct a runtime-level continuation snapshot restore operation.
    pub fn new(
        ledger: &'a mut CapacityLedger,
        plan: &'a ContinuationSnapshotPlan,
        budget: ResourceBudget,
    ) -> Self {
        Self {
            ledger,
            plan,
            budget,
        }
    }
    /// Restore a continuation snapshot with a caller-provided elapsed sample.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotRestoreError`] when capacity admission or launch fails.
    pub async fn execute_with_elapsed<L>(
        self,
        launcher: &mut L,
        elapsed: Duration,
    ) -> Result<ContinuationSnapshotRestoreReport<L::Session>, SnapshotRestoreError<L::Error>>
    where
        L: SnapshotSessionLauncher,
    {
        let manifest = self.plan.snapshot_manifest();
        let report = RuntimeSnapshotRestore::new(self.ledger, &manifest, self.budget)
            .execute_with_elapsed(launcher, elapsed)
            .await?;
        let benchmark_samples = report.benchmark_samples().to_vec();
        let (session, reservation) = report.into_parts();
        Ok(ContinuationSnapshotRestoreReport::new(
            session,
            self.plan.reason(),
            reservation,
            benchmark_samples,
        ))
    }
}
/// Template snapshot sink backed by a live `firkin-core` container.
#[cfg(feature = "snapshot")]
#[derive(Debug)]
pub struct CoreContainerSnapshotSink<'a, S = firkin_core::Streams> {
    pub(crate) container: &'a firkin_core::Container<S>,
    pub(crate) state_path: Option<PathBuf>,
}
#[cfg(feature = "snapshot")]
impl<'a, S> CoreContainerSnapshotSink<'a, S> {
    /// Construct a sink that saves snapshots from `container`.
    #[must_use]
    pub const fn new(container: &'a firkin_core::Container<S>) -> Self {
        Self {
            container,
            state_path: None,
        }
    }
    /// Persist restore state at `state_path` instead of deriving it from the
    /// snapshot path.
    #[must_use]
    pub fn with_state_path(mut self, state_path: impl Into<PathBuf>) -> Self {
        self.state_path = Some(state_path.into());
        self
    }
    /// Return the container used as the snapshot source.
    #[must_use]
    pub const fn container(&self) -> &'a firkin_core::Container<S> {
        self.container
    }
    /// Return the state path that would be written for `snapshot_path`.
    #[must_use]
    pub fn state_path_for_snapshot(&self, snapshot_path: &Path) -> PathBuf {
        self.state_path
            .clone()
            .unwrap_or_else(|| snapshot_path.with_extension("state.json"))
    }
}
#[cfg(feature = "snapshot")]
#[async_trait]
impl<S> TemplateSnapshotSink for CoreContainerSnapshotSink<'_, S>
where
    S: firkin_core::ContainerStdio + Send + Sync,
{
    async fn save_snapshot(&self, path: &std::path::Path) -> Result<(), SnapshotSinkError> {
        self.container
            .save_snapshot(path)
            .await
            .map_err(|source| Box::new(source) as SnapshotSinkError)?;
        let state = self
            .container
            .snapshot_state()
            .await
            .map_err(|source| Box::new(source) as SnapshotSinkError)?;
        let persisted = PersistedContainerSnapshotState::from_snapshot_state(&state);
        let state_path = self.state_path_for_snapshot(path);
        let bytes = serde_json::to_vec_pretty(&persisted)
            .map_err(|source| Box::new(source) as SnapshotSinkError)?;
        std::fs::write(state_path, bytes).map_err(|source| Box::new(source) as SnapshotSinkError)
    }
}
