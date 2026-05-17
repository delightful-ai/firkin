//! adapter — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::DEFAULT_RUNTIME_MINIMUM_FREE_DISK;
#[allow(unused_imports)]
use crate::DEFAULT_RUNTIME_WARM_POOL_MINIMUM_FREE_DISK;
#[allow(unused_imports)]
use crate::FRESHNESS_SYNC_CHECKOUT_METADATA;
#[allow(unused_imports)]
use crate::continuation::RuntimeContinuationSnapshotSource;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::continuation::{RuntimeCubeSandboxFollowupConfig, runtime_continuation_snapshot_path};
#[allow(unused_imports)]
use crate::disk::{DiskPressureProbe, HostDiskPressureProbe, RuntimeDiskPressureGuard};
#[allow(unused_imports)]
use crate::hygiene::{
    ambiguous_process_error, filesystem_exit_error, finite_process_error, parse_filesystem_entries,
    parse_single_filesystem_entry, runtime_timestamps, unique_process_match, validate_marker_id,
    write_active_vm_heartbeat,
};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::hygiene::{delete_firkin_snapshot_file, snapshot_artifact_integrity};
#[allow(unused_imports)]
use crate::interactive::{RuntimeInteractiveProcess, RuntimeInteractiveProcessRunner};
#[allow(unused_imports)]
use crate::preflight::RuntimePreflight;
#[allow(unused_imports)]
use crate::restore::{
    ActiveSessionReservation, RuntimeCubeSandboxCreateConfig, RuntimeCubeSandboxCreateError,
    SnapshotRestoreError, SnapshotRestoreRequest, SnapshotSessionLauncher,
    disk_pressure_to_capacity_error, runtime_create_error_to_backend,
};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use crate::restore::{snapshot_restore_error_to_backend, write_snapshot_artifact_sidecars};
#[allow(unused_imports)]
use crate::session::{
    RuntimeCommandRunner, RuntimeCommandStartReport, RuntimeCommandStreamCompletion,
    RuntimeCommandStreamRunner, RuntimePortRouter, RuntimeReadinessProbe, RuntimeSessionStop,
};
#[allow(unused_imports)]
use crate::template_build::{
    FirkinWarmTemplateMaintainReport, freshness_sync_checkout_for_request,
    freshness_sync_gate_for_request, verify_prepared_template_integrity,
};
#[allow(unused_imports)]
use crate::warm_pool::WarmPoolMaintenanceReport;
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use base64::Engine as _;
#[allow(unused_imports)]
use firkin_admission::ActiveQueuePolicy;
#[allow(unused_imports)]
use firkin_admission::{CapacityLedger, WarmPoolKey, WarmPoolLedger};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_artifacts::SnapshotArtifactIntegrity;
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_artifacts::{ContinuationSnapshotPlan, ContinuationSnapshotReason};
#[cfg(feature = "snapshot")]
#[allow(unused_imports)]
use firkin_e2b_contract::PreparedTemplateArtifactIntegrity;
#[allow(unused_imports)]
use firkin_envd::{
    EnvdProcessEventStream, EnvdProcessInfo, EnvdProcessOutput, EnvdProcessStreamEvent,
};
#[allow(unused_imports)]
use firkin_template::FreshnessSyncGate;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use firkin_trace::{
    BenchmarkSample, EventTraceRecorder, LifecycleClass, RuntimeProfile, SandboxEventName,
    SandboxEventTrace, WorkloadClass,
};
#[allow(unused_imports)]
use firkin_types::{SandboxNetworkPolicy, Size};
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::collections::VecDeque;
#[allow(unused_imports)]
use std::fmt::Display;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
#[allow(unused_imports)]
use tokio::net::{TcpListener, TcpStream, UnixStream};
#[allow(unused_imports)]
use tokio::sync::Mutex;
#[allow(unused_imports)]
use tokio::sync::Notify;
use tokio::sync::mpsc;
#[allow(unused_imports)]
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use {
    firkin_admission::{
        ActiveBackpressureDecision, ActiveBackpressurePlan, BackpressureRejection, ResourceBudget,
        WarmPoolEntry,
    },
    firkin_artifacts::SnapshotArtifactManifest,
    firkin_hygiene::RestartResourceKind,
};
#[allow(unused_imports)]
use {
    firkin_e2b_contract::{
        BackendError, DEFAULT_CODE_INTERPRETER_PORT, DEFAULT_MCP_PORT, FollowupSnapshot,
        PausedSandbox, PortProxyStream, PortTarget, PreparedTemplate, RuntimeAdapter,
        RuntimeCapabilitySet, RuntimeSandbox, RuntimeTemplateBuild, SandboxRuntimeConfig,
        SnapshotRef, StartSandboxRequest,
    },
    firkin_e2b_server::EnvdProcessHttpServer,
    firkin_e2b_wire::{SandboxLogs, SandboxMetric, TemplateBuildRequest},
    firkin_envd::{
        DEFAULT_ENVD_PORT, EnvdFilesystemAdapter, EnvdFilesystemEntry, EnvdFilesystemEvent,
        EnvdFilesystemEventType, EnvdFilesystemWriteInfo, EnvdProcessAdapter, EnvdProcessInput,
        EnvdProcessSelector, EnvdProcessSignal, EnvdProcessStartRequest, EnvdPtySize,
    },
};
const FIRKIN_ADAPTER_VSOCK_CID: u32 = 0;
const DEFAULT_ACTIVE_VM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_ACTIVE_QUEUE_MAX_PENDING: usize = 64;
const CODE_INTERPRETER_CONTEXT_CODE_ENV: &str = "FIRKIN_CODE_INTERPRETER_CODE_B64";
const CODE_INTERPRETER_CONTEXT_PATH_ENV: &str = "FIRKIN_CODE_INTERPRETER_CONTEXT_PATH";
const CODE_INTERPRETER_CONTEXT_RUNNER: &str = r#"
import base64
import builtins
import os
import pathlib
import pickle

code = base64.b64decode(os.environ["FIRKIN_CODE_INTERPRETER_CODE_B64"]).decode()
path = pathlib.Path(os.environ["FIRKIN_CODE_INTERPRETER_CONTEXT_PATH"])
namespace = {"__builtins__": builtins}
if path.exists():
    loaded = pickle.loads(path.read_bytes())
    if isinstance(loaded, dict):
        namespace.update(loaded)

try:
    exec(compile(code, "<firkin-code-interpreter>", "exec"), namespace, namespace)
finally:
    path.parent.mkdir(parents=True, exist_ok=True)
    state = {}
    for key, value in namespace.items():
        if key.startswith("__"):
            continue
        try:
            pickle.dumps({key: value})
        except Exception:
            continue
        state[key] = value
    path.write_bytes(pickle.dumps(state))
"#;
#[derive(Debug)]
struct FirkinRuntimeSession<S> {
    pub(crate) session: Arc<Mutex<S>>,
    pub(crate) reservation: ActiveSessionReservation,
    resume_started: Instant,
    resume_snapshot_to_first_stdout_recorded: bool,
    startup_trace: Option<EventTraceRecorder>,
    envd_port: u16,
    envd_task: JoinHandle<()>,
    code_interpreter_port: u16,
    code_interpreter_task: JoinHandle<()>,
    active_vm_heartbeat_task: Option<JoinHandle<()>>,
    freshness_sync: Option<FreshnessSyncGate>,
    freshness_sync_checkout: Option<String>,
    continuation_snapshot_saved: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FirkinRuntimeProcessRecord {
    info: EnvdProcessInfo,
    pub(crate) output: EnvdProcessOutput,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FirkinRuntimeProcessKey {
    #[allow(missing_docs)]
    pub sandbox_id: String,
    pub(crate) pid: u32,
}
pub(crate) struct FirkinRuntimeAdapterState<L>
where
    L: SnapshotSessionLauncher,
{
    pub(crate) ledger: CapacityLedger,
    pub(crate) launcher: L,
    next_sandbox: u64,
    active: BTreeMap<String, FirkinRuntimeSession<L::Session>>,
    warm_pool: WarmPoolLedger,
    warm_sessions: BTreeMap<WarmPoolKey, VecDeque<WarmPoolMaintenanceReport<L::Session>>>,
    pub(crate) processes: BTreeMap<FirkinRuntimeProcessKey, FirkinRuntimeProcessRecord>,
    interactive_processes: BTreeMap<FirkinRuntimeProcessKey, Box<dyn RuntimeInteractiveProcess>>,
    restored_paths: Vec<PathBuf>,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
    pub(crate) benchmark_event_traces: Vec<SandboxEventTrace>,
}
struct ActiveRestoreAdmission<L>
where
    L: SnapshotSessionLauncher,
{
    pub(crate) launcher: L,
    evicted_warm_sessions: Vec<L::Session>,
}
struct ActiveQueueWaiter {
    pending: Arc<AtomicUsize>,
}
impl ActiveQueueWaiter {
    fn try_acquire(pending: Arc<AtomicUsize>, policy: ActiveQueuePolicy) -> Option<Self> {
        let max_pending = policy.max_pending();
        let mut current = pending.load(Ordering::SeqCst);
        loop {
            if current >= max_pending {
                return None;
            }
            match pending.compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Some(Self { pending }),
                Err(actual) => current = actual,
            }
        }
    }
}
impl Drop for ActiveQueueWaiter {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn forward_interactive_process_event(
    sender: &mpsc::Sender<Result<EnvdProcessStreamEvent, BackendError>>,
    event: Result<EnvdProcessStreamEvent, BackendError>,
) {
    match sender.try_send(event) {
        Err(mpsc::error::TrySendError::Full(event)) => {
            let _ = sender.send(event).await;
        }
        Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

/// E2B `RuntimeAdapter` backed by Firkin snapshot restore.
///
/// The adapter is the CubeAPI-facing bridge: E2B/Cube keeps product semantics
/// and registry state, while Firkin owns capacity admission and VM snapshot
/// restore. The default create path requires a prepared template artifact.
pub struct FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher,
{
    pub(crate) state: Arc<Mutex<FirkinRuntimeAdapterState<L>>>,
    pub(crate) budget: ResourceBudget,
    pub(crate) domain: String,
    pub(crate) envd_version: String,
    pub(crate) cpu_count: u32,
    pub(crate) memory_mb: u32,
    runtime_preflight: Option<RuntimePreflight>,
    active_vm_marker_root: Option<PathBuf>,
    active_vm_heartbeat_interval: Duration,
    active_queue_policy: ActiveQueuePolicy,
    active_queue_pending: Arc<AtomicUsize>,
    active_capacity_notify: Arc<Notify>,
    restore_minimum_free_disk: Size,
}
impl<L> Clone for FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher,
{
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            budget: self.budget,
            domain: self.domain.clone(),
            envd_version: self.envd_version.clone(),
            cpu_count: self.cpu_count,
            memory_mb: self.memory_mb,
            runtime_preflight: self.runtime_preflight.clone(),
            active_vm_marker_root: self.active_vm_marker_root.clone(),
            active_vm_heartbeat_interval: self.active_vm_heartbeat_interval,
            active_queue_policy: self.active_queue_policy,
            active_queue_pending: Arc::clone(&self.active_queue_pending),
            active_capacity_notify: Arc::clone(&self.active_capacity_notify),
            restore_minimum_free_disk: self.restore_minimum_free_disk,
        }
    }
}
impl<L> FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher,
{
    /// Construct a snapshot-backed Firkin `RuntimeAdapter`.
    #[must_use]
    pub fn new(
        ledger: CapacityLedger,
        launcher: L,
        budget: ResourceBudget,
        domain: impl Into<String>,
        envd_version: impl Into<String>,
        cpu_count: u32,
        memory_mb: u32,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(FirkinRuntimeAdapterState {
                ledger,
                launcher,
                next_sandbox: 0,
                active: BTreeMap::new(),
                warm_pool: WarmPoolLedger::default(),
                warm_sessions: BTreeMap::new(),
                processes: BTreeMap::new(),
                interactive_processes: BTreeMap::new(),
                restored_paths: Vec::new(),
                benchmark_samples: Vec::new(),
                benchmark_event_traces: Vec::new(),
            })),
            budget,
            domain: domain.into(),
            envd_version: envd_version.into(),
            cpu_count,
            memory_mb,
            runtime_preflight: None,
            active_vm_marker_root: None,
            active_vm_heartbeat_interval: DEFAULT_ACTIVE_VM_HEARTBEAT_INTERVAL,
            active_queue_policy: ActiveQueuePolicy::new(DEFAULT_ACTIVE_QUEUE_MAX_PENDING),
            active_queue_pending: Arc::new(AtomicUsize::new(0)),
            active_capacity_notify: Arc::new(Notify::new()),
            restore_minimum_free_disk: DEFAULT_RUNTIME_MINIMUM_FREE_DISK,
        }
    }
    /// Configure the active VM marker root consumed by runtime reconciliation.
    #[must_use]
    pub fn with_active_vm_marker_root(mut self, root: impl AsRef<Path>) -> Self {
        self.active_vm_marker_root = Some(root.as_ref().to_path_buf());
        self
    }
    /// Configure how often active VM marker heartbeats are refreshed.
    #[must_use]
    pub fn with_active_vm_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.active_vm_heartbeat_interval = interval.max(Duration::from_millis(1));
        self
    }
    /// Configure the bounded queue for active restore work waiting on capacity.
    #[must_use]
    pub fn with_active_queue_policy(mut self, policy: ActiveQueuePolicy) -> Self {
        self.active_queue_policy = policy;
        self
    }
    /// Configure the free-disk floor checked before active snapshot restores.
    #[must_use]
    pub fn with_restore_minimum_free_disk(mut self, minimum_free_disk: Size) -> Self {
        self.restore_minimum_free_disk = minimum_free_disk;
        self
    }
    /// Configure a runtime preflight gate checked before restore work.
    #[must_use]
    pub fn with_runtime_preflight(
        mut self,
        snapshot_root: impl Into<PathBuf>,
        log_root: impl Into<PathBuf>,
        minimum_free_disk: Size,
    ) -> Self {
        self.runtime_preflight = Some(RuntimePreflight::new(
            snapshot_root,
            log_root,
            minimum_free_disk,
        ));
        self
    }
    /// Configure the production runtime roots used for disk preflight and
    /// restart reconciliation.
    ///
    /// This is the one-path helper for production composition: active Firkin
    /// sessions publish heartbeat/PID/executable markers under
    /// `active_vm_marker_root`, and restore work checks the same snapshot/log
    /// roots against `minimum_free_disk` before launching.
    #[must_use]
    pub fn with_managed_runtime_roots(
        mut self,
        snapshot_root: impl Into<PathBuf>,
        log_root: impl Into<PathBuf>,
        active_vm_marker_root: impl Into<PathBuf>,
        minimum_free_disk: Size,
    ) -> Self {
        self.runtime_preflight = Some(RuntimePreflight::new(
            snapshot_root,
            log_root,
            minimum_free_disk,
        ));
        self.active_vm_marker_root = Some(active_vm_marker_root.into());
        self
    }
    fn check_runtime_preflight(&self) -> Result<(), BackendError> {
        if let Some(preflight) = &self.runtime_preflight {
            preflight
                .check()
                .map_err(|error| BackendError::Runtime(error.to_string()))?;
        }
        Ok(())
    }
    fn active_backpressure_error(rejection: Option<BackpressureRejection>) -> BackendError
    where
        L::Error: Display,
    {
        match rejection {
            Some(BackpressureRejection::Oversized(error)) => {
                runtime_create_error_to_backend(RuntimeCubeSandboxCreateError::<L::Error>::Restore(
                    SnapshotRestoreError::Capacity(error),
                ))
            }
            Some(BackpressureRejection::QueueFull {
                max_pending,
                pending,
            }) => BackendError::Runtime(format!(
                "Firkin active restore queue is full: pending {pending}, max {max_pending}"
            )),
            None => BackendError::Runtime(
                "Firkin active restore queue rejected without a reason".to_owned(),
            ),
        }
    }
    async fn reserve_active_restore_capacity(
        &self,
    ) -> Result<ActiveRestoreAdmission<L>, BackendError>
    where
        L: Clone,
        L::Error: Display,
    {
        loop {
            let notified = self.active_capacity_notify.notified();
            let mut state = self.state.lock().await;
            let pending = self.active_queue_pending.load(Ordering::SeqCst);
            let plan = ActiveBackpressurePlan::from_warm_pool(
                self.budget,
                &state.warm_pool,
                state.ledger,
                pending,
                self.active_queue_policy,
            );
            match plan.decision() {
                ActiveBackpressureDecision::AdmitNow => {
                    let eviction_keys = plan
                        .evictions()
                        .iter()
                        .map(|entry| entry.key().clone())
                        .collect::<Vec<_>>();
                    let mut evicted_warm_sessions = Vec::new();
                    for key in eviction_keys {
                        let state_ref = &mut *state;
                        let Some(_entry) = state_ref.warm_pool.expire(&key, &mut state_ref.ledger)
                        else {
                            continue;
                        };
                        let remove_reports =
                            if let Some(reports) = state_ref.warm_sessions.get_mut(&key) {
                                if let Some(report) = reports.pop_front() {
                                    let (session, _entry) = report.into_parts();
                                    evicted_warm_sessions.push(session);
                                }
                                reports.is_empty()
                            } else {
                                false
                            };
                        if remove_reports {
                            state_ref.warm_sessions.remove(&key);
                        }
                    }
                    state.ledger.reserve_active(self.budget).map_err(|error| {
                        runtime_create_error_to_backend(
                            RuntimeCubeSandboxCreateError::<L::Error>::Restore(
                                SnapshotRestoreError::Capacity(error),
                            ),
                        )
                    })?;
                    return Ok(ActiveRestoreAdmission {
                        launcher: state.launcher.clone(),
                        evicted_warm_sessions,
                    });
                }
                ActiveBackpressureDecision::Queue => {
                    let Some(waiter) = ActiveQueueWaiter::try_acquire(
                        Arc::clone(&self.active_queue_pending),
                        self.active_queue_policy,
                    ) else {
                        return Err(Self::active_backpressure_error(Some(
                            BackpressureRejection::QueueFull {
                                max_pending: self.active_queue_policy.max_pending(),
                                pending,
                            },
                        )));
                    };
                    drop(state);
                    notified.await;
                    drop(waiter);
                }
                ActiveBackpressureDecision::Reject => {
                    return Err(Self::active_backpressure_error(plan.rejection()));
                }
            }
        }
    }
    async fn release_active_budget(&self, budget: ResourceBudget) {
        self.state.lock().await.ledger.release_active(budget);
        self.notify_active_capacity_released();
    }
    async fn release_active_reservation(&self, reservation: &mut ActiveSessionReservation) {
        let released = {
            let mut state = self.state.lock().await;
            reservation.release_into(&mut state.ledger).is_some()
        };
        if released {
            self.notify_active_capacity_released();
        }
    }
    fn notify_active_capacity_released(&self) {
        self.active_capacity_notify.notify_one();
    }
    fn active_vm_marker_path(&self, sandbox_id: &str) -> Result<Option<PathBuf>, BackendError> {
        let Some(root) = &self.active_vm_marker_root else {
            return Ok(None);
        };
        validate_marker_id(RestartResourceKind::ActiveVm, sandbox_id)
            .map_err(|error| BackendError::Runtime(error.to_string()))?;
        Ok(Some(root.join(sandbox_id)))
    }
    fn start_active_vm_heartbeat(
        &self,
        sandbox_id: &str,
    ) -> Result<Option<JoinHandle<()>>, BackendError> {
        let Some(path) = self.active_vm_marker_path(sandbox_id)? else {
            return Ok(None);
        };
        write_active_vm_heartbeat(&path)?;
        let interval = self.active_vm_heartbeat_interval;
        Ok(Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if write_active_vm_heartbeat(&path).is_err() {
                    break;
                }
            }
        })))
    }
    fn remove_active_vm_marker(&self, sandbox_id: &str) -> Result<(), BackendError> {
        let Some(path) = self.active_vm_marker_path(sandbox_id)? else {
            return Ok(());
        };
        let remove_result = match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => Err(error),
        };
        match remove_result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(BackendError::Runtime(format!(
                "Firkin active VM marker remove failed at `{}`: {source}",
                path.display()
            ))),
        }
    }
    /// Return the currently active capacity budget.
    pub async fn active_budget(&self) -> ResourceBudget {
        self.state.lock().await.ledger.active()
    }
    /// Return snapshot paths restored through this adapter.
    pub async fn restored_paths(&self) -> Vec<PathBuf> {
        self.state.lock().await.restored_paths.clone()
    }
    /// Return lifecycle benchmark samples recorded through this adapter.
    pub async fn benchmark_samples(&self) -> Vec<BenchmarkSample> {
        self.state.lock().await.benchmark_samples.clone()
    }
    /// Return raw benchmark event traces recorded through this adapter.
    pub async fn benchmark_event_traces(&self) -> Vec<SandboxEventTrace> {
        self.state.lock().await.benchmark_event_traces.clone()
    }
    /// Mark a sandbox freshness sync as complete, unlocking writes.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not active.
    pub async fn complete_freshness_sync(
        &self,
        sandbox_id: &str,
        commit: impl Into<String>,
    ) -> Result<(), BackendError> {
        let mut state = self.state.lock().await;
        let active = state
            .active
            .get_mut(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        if let Some(gate) = active.freshness_sync.take() {
            active.freshness_sync = Some(gate.complete_sync(commit));
        }
        Ok(())
    }
    /// Mark a sandbox freshness sync as failed, keeping writes blocked.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::NotFound`] when the sandbox is not active.
    pub async fn fail_freshness_sync(
        &self,
        sandbox_id: &str,
        reason: impl Into<String>,
    ) -> Result<(), BackendError> {
        let mut state = self.state.lock().await;
        let active = state
            .active
            .get_mut(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        if let Some(gate) = active.freshness_sync.take() {
            active.freshness_sync = Some(gate.fail_sync(reason));
        }
        Ok(())
    }
    /// Restore and retain a prepared template session for the next matching create.
    ///
    /// Returns `Ok(false)` when the matching warm entry already exists.
    ///
    /// # Errors
    ///
    /// Returns runtime restore or capacity errors when the template cannot be
    /// retained in the adapter warm pool.
    pub async fn prewarm_template(&self, template: PreparedTemplate) -> Result<bool, BackendError>
    where
        L: Clone + Send,
        L::Session: RuntimeReadinessProbe,
        L::Error: Display + Send,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
    {
        let mut disk_probe = HostDiskPressureProbe::new();
        self.prewarm_template_with_disk_probe(template, &mut disk_probe)
            .await
    }
    /// Restore and retain one prepared template using an injected disk probe.
    ///
    /// This is primarily useful for operator tests and custom hosts that want
    /// to prove or substitute the warm-pool admission source. The normal path
    /// uses [`HostDiskPressureProbe`].
    ///
    /// # Errors
    ///
    /// Returns runtime restore or capacity errors when the template cannot be
    /// retained in the adapter warm pool.
    pub async fn prewarm_template_with_disk_probe<P>(
        &self,
        template: PreparedTemplate,
        disk_probe: &mut P,
    ) -> Result<bool, BackendError>
    where
        L: Clone + Send,
        L::Session: RuntimeReadinessProbe,
        L::Error: Display + Send,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
        P: DiskPressureProbe,
    {
        let key = Self::warm_pool_key(&template.template_id);
        if self.state.lock().await.warm_pool.contains(&key) {
            return Ok(false);
        }
        self.prewarm_template_entry_with_disk_probe(template, false, disk_probe)
            .await
    }
    async fn prewarm_template_entry_with_disk_probe<P>(
        &self,
        template: PreparedTemplate,
        allow_existing: bool,
        disk_probe: &mut P,
    ) -> Result<bool, BackendError>
    where
        L: Clone + Send,
        L::Session: RuntimeReadinessProbe,
        L::Error: Display + Send,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
        P: DiskPressureProbe,
    {
        let key = Self::warm_pool_key(&template.template_id);
        self.check_runtime_preflight()?;
        let manifest =
            SnapshotArtifactManifest::base(template.template_id.clone(), template.artifact.clone());
        verify_prepared_template_integrity::<L::Error>(&template, &manifest)
            .map_err(runtime_create_error_to_backend)?;
        let disk_root = manifest.path().parent().unwrap_or(Path::new("/"));
        RuntimeDiskPressureGuard::new(disk_root, DEFAULT_RUNTIME_WARM_POOL_MINIMUM_FREE_DISK)
            .check(disk_probe)
            .map_err(|error| disk_pressure_to_capacity_error(&error))
            .map_err(|error| {
                runtime_create_error_to_backend(RuntimeCubeSandboxCreateError::Restore(
                    SnapshotRestoreError::<L::Error>::Capacity(error),
                ))
            })?;
        let mut launcher = {
            let state = self.state.lock().await;
            if !allow_existing && state.warm_pool.contains(&key) {
                return Ok(false);
            }
            state.launcher.clone()
        };
        let restore_request = SnapshotRestoreRequest::new(&manifest, self.budget);
        let mut session = launcher
            .restore_from_snapshot(&restore_request)
            .await
            .map_err(|source| {
                runtime_create_error_to_backend(RuntimeCubeSandboxCreateError::Restore(
                    SnapshotRestoreError::Launch { source },
                ))
            })?;
        let mut readiness_trace = EventTraceRecorder::new(
            LifecycleClass::Warm,
            WorkloadClass::ReadinessProbe,
            RuntimeProfile::FastAgent,
        );
        readiness_trace.record(SandboxEventName::SnapshotRestoreDone);
        let readiness_report = match session.probe_ready(readiness_trace).await {
            Ok(report) => report,
            Err(error) => {
                let _ = session.stop_session().await;
                return Err(BackendError::Runtime(format!(
                    "Firkin warm-pool readiness probe failed: {error}"
                )));
            }
        };
        let entry = WarmPoolEntry::new(key.clone(), manifest.logical_id().to_owned(), self.budget);
        let mut state = self.state.lock().await;
        if !allow_existing && state.warm_pool.contains(&key) {
            let _ = session.stop_session().await;
            return Ok(false);
        }
        let state_ref = &mut *state;
        if let Err(error) = state_ref
            .warm_pool
            .maintain(entry.clone(), &mut state_ref.ledger)
        {
            let _ = session.stop_session().await;
            return Err(runtime_create_error_to_backend(
                RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::<L::Error>::Capacity(
                    error,
                )),
            ));
        }
        state_ref
            .warm_sessions
            .entry(key)
            .or_default()
            .push_back(WarmPoolMaintenanceReport::new(session, entry));
        state_ref
            .benchmark_event_traces
            .extend_from_slice(readiness_report.benchmark_event_traces());
        state_ref
            .restored_paths
            .push(PathBuf::from(&template.artifact));
        Ok(true)
    }
    /// Restore and retain every missing prepared template target.
    ///
    /// This is the scheduler-facing form of [`Self::prewarm_template`]: it is
    /// idempotent for already-warm templates and returns which targets were
    /// restored during this pass.
    ///
    /// # Errors
    ///
    /// Returns runtime restore or capacity errors when any missing target cannot
    /// be retained in the adapter warm pool.
    pub async fn maintain_warm_templates(
        &self,
        templates: impl IntoIterator<Item = PreparedTemplate>,
    ) -> Result<FirkinWarmTemplateMaintainReport, BackendError>
    where
        L: Clone + Send,
        L::Session: RuntimeReadinessProbe,
        L::Error: Display + Send,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
    {
        let mut disk_probe = HostDiskPressureProbe::new();
        self.maintain_warm_templates_with_disk_probe(templates, &mut disk_probe)
            .await
    }
    /// Restore and retain missing prepared template targets using an injected
    /// disk-pressure probe.
    ///
    /// This mirrors [`Self::maintain_warm_templates`] for deterministic tests
    /// and managed hosts that already own disk admission telemetry. Production
    /// callers should use [`Self::maintain_warm_templates`] so the host disk
    /// floor remains enforced by the real probe.
    ///
    /// # Errors
    ///
    /// Returns runtime restore or capacity errors when any missing target cannot
    /// be retained in the adapter warm pool.
    pub async fn maintain_warm_templates_with_disk_probe<P>(
        &self,
        templates: impl IntoIterator<Item = PreparedTemplate>,
        disk_probe: &mut P,
    ) -> Result<FirkinWarmTemplateMaintainReport, BackendError>
    where
        L: Clone + Send,
        L::Session: RuntimeReadinessProbe,
        L::Error: Display + Send,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        L::Session: RuntimeSessionStop + Send + 'static,
        <L::Session as RuntimeSessionStop>::Error: Display,
        P: DiskPressureProbe,
    {
        let mut report = FirkinWarmTemplateMaintainReport::default();
        let mut desired_by_key = BTreeMap::<WarmPoolKey, usize>::new();
        for template in templates {
            let template_id = template.template_id.clone();
            let key = Self::warm_pool_key(&template.template_id);
            let desired_depth = {
                let desired = desired_by_key.entry(key.clone()).or_insert(0);
                *desired = desired.saturating_add(1);
                *desired
            };
            let current_depth = self.state.lock().await.warm_pool.count(&key);
            if current_depth < desired_depth
                && self
                    .prewarm_template_entry_with_disk_probe(template, true, disk_probe)
                    .await?
            {
                report.maintained.push(template_id);
            } else {
                report.skipped_already_warm.push(template_id);
            }
        }
        Ok(report)
    }
    /// Restore a continuation snapshot for a Cube/E2B follow-up prompt.
    ///
    /// This is intentionally not wired to E2B `resume`: follow-up prompts
    /// create a new runtime sandbox from a durable continuation artifact.
    ///
    /// # Errors
    ///
    /// Returns runtime restore errors when capacity admission or snapshot launch
    /// fails.
    #[cfg(feature = "snapshot")]
    #[allow(clippy::too_many_lines)]
    pub async fn start_followup(
        &self,
        request: StartSandboxRequest,
        plan: &ContinuationSnapshotPlan,
    ) -> Result<RuntimeSandbox, BackendError>
    where
        L: Clone + Send + 'static,
        L::Error: Display + Send,
        L::Session: RuntimeCommandRunner
            + RuntimeCommandStreamRunner
            + RuntimeInteractiveProcessRunner
            + RuntimeReadinessProbe
            + RuntimeSessionStop
            + Send
            + Sync
            + 'static,
        <L::Session as RuntimeCommandRunner>::Error: Display + Send,
        <L::Session as RuntimeCommandStreamRunner>::Error: Display + Send,
        <L::Session as RuntimeInteractiveProcessRunner>::Error: Display + Send,
        <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
        <L::Session as RuntimeSessionStop>::Error: Display,
    {
        let freshness_sync = freshness_sync_gate_for_request(&request);
        let freshness_sync_checkout = freshness_sync_checkout_for_request(&request);
        let start_freshness_sync = freshness_sync.is_some() && freshness_sync_checkout.is_some();
        let disk_root = plan
            .snapshot_output_path()
            .parent()
            .unwrap_or(Path::new("/"));
        let mut disk_probe = HostDiskPressureProbe::new();
        RuntimeDiskPressureGuard::new(disk_root, self.restore_minimum_free_disk)
            .check(&mut disk_probe)
            .map_err(|error| disk_pressure_to_capacity_error(&error))
            .map_err(|error| {
                snapshot_restore_error_to_backend(SnapshotRestoreError::<L::Error>::Capacity(error))
            })?;
        let admission = self.reserve_active_restore_capacity().await?;
        let (sandbox_id, runtime_config) = {
            let mut state = self.state.lock().await;
            state.next_sandbox = state.next_sandbox.saturating_add(1).max(1);
            let sandbox_id = format!("sbx_firkin_{}", state.next_sandbox);
            let runtime_config =
                self.next_config(sandbox_id.clone(), request.create_request.timeout);
            (sandbox_id, runtime_config)
        };
        let restore_plan = ContinuationSnapshotPlan::new(
            sandbox_id.clone(),
            plan.reason(),
            plan.snapshot_output_path().to_path_buf(),
        );
        let create_config = RuntimeCubeSandboxFollowupConfig::new(
            runtime_config.sandbox_id.clone(),
            runtime_config.domain.clone(),
            runtime_config.envd_version.clone(),
            runtime_config.started_at.clone(),
            runtime_config.end_at.clone(),
            runtime_config.cpu_count,
            runtime_config.memory_mb,
        );
        for mut session in admission.evicted_warm_sessions {
            let _ = session.stop_session().await;
        }
        let started = Instant::now();
        let mut startup_trace = EventTraceRecorder::new(
            LifecycleClass::Resumed,
            WorkloadClass::TinyExec,
            RuntimeProfile::FastAgent,
        );
        startup_trace.record(SandboxEventName::SnapshotRestoreStart);
        let manifest = restore_plan.snapshot_manifest();
        let restore_request = SnapshotRestoreRequest::new(&manifest, self.budget);
        let mut launcher = admission.launcher;
        let mut session = match launcher.restore_from_snapshot(&restore_request).await {
            Ok(session) => session,
            Err(source) => {
                self.release_active_budget(self.budget).await;
                return Err(snapshot_restore_error_to_backend(
                    SnapshotRestoreError::Launch { source },
                ));
            }
        };
        startup_trace.record(SandboxEventName::SnapshotRestoreDone);
        let runtime_sandbox = RuntimeSandbox {
            config: create_config.into_runtime_config(),
            exposed_ports: vec![
                DEFAULT_ENVD_PORT,
                DEFAULT_CODE_INTERPRETER_PORT,
                DEFAULT_MCP_PORT,
            ],
        };
        let mut benchmark_samples = vec![BenchmarkSample::new(
            "warm_snapshot_restore",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            started.elapsed().as_secs_f64() * 1000.0,
        )];
        let mut reservation = ActiveSessionReservation::new(self.budget);
        let readiness_trace = EventTraceRecorder::new(
            LifecycleClass::Resumed,
            WorkloadClass::ReadinessProbe,
            RuntimeProfile::FastAgent,
        );
        let ready_started = Instant::now();
        let readiness_report = match session.probe_ready(readiness_trace).await {
            Ok(report) => report,
            Err(error) => {
                let _ = session.stop_session().await;
                self.release_active_reservation(&mut reservation).await;
                return Err(BackendError::Runtime(format!(
                    "Firkin readiness probe failed: {error}"
                )));
            }
        };
        startup_trace.record(SandboxEventName::ReadyProbePassed);
        benchmark_samples.push(BenchmarkSample::new(
            "ready_probe",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            ready_started.elapsed().as_secs_f64() * 1000.0,
        ));
        let (envd_port, envd_task) = match self.spawn_envd_server(sandbox_id.clone()).await {
            Ok(started) => started,
            Err(error) => {
                let _ = session.stop_session().await;
                self.release_active_reservation(&mut reservation).await;
                return Err(error);
            }
        };
        let (code_interpreter_port, code_interpreter_task) =
            match self.spawn_code_interpreter_server(sandbox_id.clone()).await {
                Ok(started) => started,
                Err(error) => {
                    envd_task.abort();
                    let _ = session.stop_session().await;
                    self.release_active_reservation(&mut reservation).await;
                    return Err(error);
                }
            };
        let active_vm_heartbeat_task = match self.start_active_vm_heartbeat(&sandbox_id) {
            Ok(task) => task,
            Err(error) => {
                envd_task.abort();
                code_interpreter_task.abort();
                let _ = session.stop_session().await;
                self.release_active_reservation(&mut reservation).await;
                return Err(error);
            }
        };
        let mut state = self.state.lock().await;
        state.benchmark_samples.extend(benchmark_samples);
        state
            .benchmark_event_traces
            .extend_from_slice(readiness_report.benchmark_event_traces());
        state.active.insert(
            sandbox_id.clone(),
            FirkinRuntimeSession {
                session: Arc::new(Mutex::new(session)),
                reservation,
                resume_started: started,
                resume_snapshot_to_first_stdout_recorded: false,
                startup_trace: Some(startup_trace),
                envd_port,
                envd_task,
                code_interpreter_port,
                code_interpreter_task,
                active_vm_heartbeat_task,
                freshness_sync,
                freshness_sync_checkout,
                continuation_snapshot_saved: false,
            },
        );
        state
            .restored_paths
            .push(restore_plan.snapshot_output_path().to_path_buf());
        if start_freshness_sync {
            self.spawn_freshness_sync_task(sandbox_id);
        }
        Ok(runtime_sandbox)
    }
    fn next_config(
        &self,
        sandbox_id: String,
        timeout_seconds: Option<u64>,
    ) -> SandboxRuntimeConfig {
        let (started_at, end_at) = runtime_timestamps(timeout_seconds.unwrap_or(300));
        RuntimeCubeSandboxCreateConfig::new(
            sandbox_id,
            self.domain.clone(),
            self.envd_version.clone(),
            started_at,
            end_at,
            self.cpu_count,
            self.memory_mb,
        )
        .into_runtime_config()
    }
    fn warm_pool_key(template_id: &str) -> WarmPoolKey {
        WarmPoolKey::new(template_id, "base-template", "apple-vz-arm64")
    }
    async fn spawn_envd_server(
        &self,
        sandbox_id: String,
    ) -> Result<(u16, JoinHandle<()>), BackendError>
    where
        FirkinRuntimeSandboxEnvdAdapter<L>: EnvdProcessAdapter<Error = BackendError>
            + EnvdFilesystemAdapter<Error = BackendError>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|error| {
            BackendError::Runtime(format!("bind Firkin envd listener: {error}"))
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| BackendError::Runtime(format!("read Firkin envd listener: {error}")))?
            .port();
        let server = EnvdProcessHttpServer::new(FirkinRuntimeSandboxEnvdAdapter {
            adapter: self.clone(),
            sandbox_id,
        });
        let task = tokio::spawn(async move {
            let _ = server.serve(listener).await;
        });
        Ok((port, task))
    }
    async fn spawn_code_interpreter_server(
        &self,
        sandbox_id: String,
    ) -> Result<(u16, JoinHandle<()>), BackendError>
    where
        FirkinRuntimeSandboxEnvdAdapter<L>:
            EnvdProcessAdapter<Error = BackendError> + Clone + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|error| {
            BackendError::Runtime(format!("bind Firkin code-interpreter listener: {error}"))
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| {
                BackendError::Runtime(format!("read Firkin code-interpreter listener: {error}"))
            })?
            .port();
        let adapter = FirkinRuntimeSandboxEnvdAdapter {
            adapter: self.clone(),
            sandbox_id: sandbox_id.clone(),
        };
        let task = tokio::spawn(async move {
            serve_firkin_code_interpreter(listener, sandbox_id, adapter).await;
        });
        Ok((port, task))
    }
}
#[allow(clippy::too_many_lines)]
#[async_trait]
impl<L> RuntimeAdapter for FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher + Clone + Send + 'static,
    L::Error: Display + Send,
    L::Session: RuntimeCommandRunner
        + RuntimeCommandStreamRunner
        + RuntimeInteractiveProcessRunner
        + RuntimePortRouter
        + RuntimeReadinessProbe
        + RuntimeSessionStop
        + RuntimeContinuationSnapshotSource
        + Send
        + Sync
        + 'static,
    <L::Session as RuntimeCommandRunner>::Error: Display + Send,
    <L::Session as RuntimeCommandStreamRunner>::Error: Display + Send,
    <L::Session as RuntimeInteractiveProcessRunner>::Error: Display + Send,
    <L::Session as RuntimePortRouter>::Error: Display,
    <L::Session as RuntimeReadinessProbe>::Error: Display + Send,
    <L::Session as RuntimeSessionStop>::Error: Display,
{
    async fn preflight(&self) -> Result<RuntimeCapabilitySet, BackendError> {
        Ok(RuntimeCapabilitySet {
            backend: "firkin-vz".to_owned(),
            supported: vec![
                "apple-vz-snapshot-restore".to_owned(),
                "e2b-runtime-adapter".to_owned(),
                "single-node-capacity-admission".to_owned(),
            ],
            unsupported: vec![
                (
                    "restrictive-network-policy".to_owned(),
                    "Firkin does not yet enforce guest firewall or host PF policy".to_owned(),
                ),
                (
                    "adapter-level-template-build".to_owned(),
                    "template builds are orchestrated through firkin-runtime template build jobs"
                        .to_owned(),
                ),
            ],
        })
    }
    async fn prepare_template(
        &self,
        request: TemplateBuildRequest,
    ) -> Result<firkin_e2b_contract::PreparedTemplate, BackendError> {
        Err(BackendError::Runtime(format!(
            "Firkin RuntimeAdapter cannot prepare template `{}` directly; run the runtime template build pipeline and pass its prepared snapshot artifact",
            request.name.unwrap_or_else(|| "unnamed".to_owned())
        )))
    }
    async fn build_template(
        &self,
        request: RuntimeTemplateBuild,
    ) -> Result<PreparedTemplate, BackendError> {
        Err(BackendError::Runtime(format!(
            "Firkin RuntimeAdapter cannot build template `{}/{}` directly; run the runtime template build pipeline and register its prepared snapshot artifact",
            request.template_id, request.build_id
        )))
    }
    async fn start(&self, request: StartSandboxRequest) -> Result<RuntimeSandbox, BackendError> {
        self.check_runtime_preflight()?;
        let template = request
            .prepared_template
            .as_ref()
            .ok_or(RuntimeCubeSandboxCreateError::<L::Error>::MissingPreparedTemplate)
            .map_err(runtime_create_error_to_backend)?;
        let manifest =
            SnapshotArtifactManifest::base(template.template_id.clone(), template.artifact.clone());
        verify_prepared_template_integrity::<L::Error>(template, &manifest)
            .map_err(runtime_create_error_to_backend)?;
        let freshness_sync = freshness_sync_gate_for_request(&request);
        let freshness_sync_checkout = freshness_sync_checkout_for_request(&request);
        let start_freshness_sync = freshness_sync.is_some() && freshness_sync_checkout.is_some();
        let restored_path = PathBuf::from(&template.artifact);
        let started = Instant::now();
        let warm_key = Self::warm_pool_key(&template.template_id);
        let mut warm_pool_trace = EventTraceRecorder::new(
            LifecycleClass::Hot,
            WorkloadClass::TinyExec,
            RuntimeProfile::FastAgent,
        );
        warm_pool_trace.record(SandboxEventName::RequestStart);
        let warm_checkout = {
            let mut state = self.state.lock().await;
            if state.warm_pool.contains(&warm_key) {
                warm_pool_trace.record(SandboxEventName::PoolLeaseRequested);
                state.next_sandbox = state.next_sandbox.saturating_add(1).max(1);
                let sandbox_id = format!("sbx_firkin_{}", state.next_sandbox);
                let runtime_config =
                    self.next_config(sandbox_id.clone(), request.create_request.timeout);
                let state_ref = &mut *state;
                let entry = state_ref
                    .warm_pool
                    .checkout(&warm_key, &mut state_ref.ledger)
                    .map_err(|error| {
                        runtime_create_error_to_backend(RuntimeCubeSandboxCreateError::Restore(
                            SnapshotRestoreError::<L::Error>::Capacity(error),
                        ))
                    })?
                    .ok_or_else(|| {
                        BackendError::Runtime(format!(
                            "Firkin warm-pool entry `{}` disappeared during checkout",
                            template.template_id
                        ))
                    })?;
                let Some(reports) = state_ref.warm_sessions.get_mut(&warm_key) else {
                    state_ref.ledger.release_active(entry.budget());
                    return Err(BackendError::Runtime(format!(
                        "Firkin warm-pool entry `{}` was missing its retained session",
                        template.template_id
                    )));
                };
                let Some(report) = reports.pop_front() else {
                    state_ref.warm_sessions.remove(&warm_key);
                    state_ref.ledger.release_active(entry.budget());
                    return Err(BackendError::Runtime(format!(
                        "Firkin warm-pool entry `{}` had no retained session",
                        template.template_id
                    )));
                };
                if reports.is_empty() {
                    state_ref.warm_sessions.remove(&warm_key);
                }
                let (session, _retained_entry) = report.into_parts();
                warm_pool_trace.record(SandboxEventName::PoolLeaseAcquired);
                let mut readiness_trace = EventTraceRecorder::new(
                    LifecycleClass::Hot,
                    WorkloadClass::ReadinessProbe,
                    RuntimeProfile::FastAgent,
                );
                readiness_trace.record(SandboxEventName::PoolLeaseAcquired);
                let runtime_sandbox = RuntimeSandbox {
                    config: runtime_config,
                    exposed_ports: vec![
                        DEFAULT_ENVD_PORT,
                        DEFAULT_CODE_INTERPRETER_PORT,
                        DEFAULT_MCP_PORT,
                    ],
                };
                Some((
                    sandbox_id,
                    runtime_sandbox,
                    session,
                    ActiveSessionReservation::new(entry.budget()),
                    vec![BenchmarkSample::new(
                        "warm_pool_checkout",
                        BenchmarkMetricKind::LifecycleLatency,
                        BenchmarkUnit::Milliseconds,
                        started.elapsed().as_secs_f64() * 1000.0,
                    )],
                    vec![warm_pool_trace.clone().finish()],
                    warm_pool_trace,
                    readiness_trace,
                ))
            } else {
                None
            }
        };
        if let Some((
            sandbox_id,
            runtime_sandbox,
            mut session,
            mut reservation,
            mut benchmark_samples,
            benchmark_event_traces,
            mut startup_trace,
            mut readiness_trace,
        )) = warm_checkout
        {
            let ready_started = Instant::now();
            readiness_trace.record(SandboxEventName::GuestAgentPingPassed);
            readiness_trace.record(SandboxEventName::WorkspaceReady);
            readiness_trace.record(SandboxEventName::ReadyProbePassed);
            startup_trace.record(SandboxEventName::ReadyProbePassed);
            benchmark_samples.push(BenchmarkSample::new(
                "ready_certificate_check",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                ready_started.elapsed().as_secs_f64() * 1000.0,
            ));
            let envd_started = Instant::now();
            let (envd_port, envd_task) = match self.spawn_envd_server(sandbox_id.clone()).await {
                Ok(started) => started,
                Err(error) => {
                    let _ = session.stop_session().await;
                    self.release_active_reservation(&mut reservation).await;
                    return Err(error);
                }
            };
            benchmark_samples.push(BenchmarkSample::new(
                "start.hot_envd_server_start_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                envd_started.elapsed().as_secs_f64() * 1000.0,
            ));
            let code_interpreter_started = Instant::now();
            let (code_interpreter_port, code_interpreter_task) =
                match self.spawn_code_interpreter_server(sandbox_id.clone()).await {
                    Ok(started) => started,
                    Err(error) => {
                        envd_task.abort();
                        let _ = session.stop_session().await;
                        self.release_active_reservation(&mut reservation).await;
                        return Err(error);
                    }
                };
            benchmark_samples.push(BenchmarkSample::new(
                "start.hot_code_interpreter_server_start_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                code_interpreter_started.elapsed().as_secs_f64() * 1000.0,
            ));
            let heartbeat_started = Instant::now();
            let active_vm_heartbeat_task = match self.start_active_vm_heartbeat(&sandbox_id) {
                Ok(task) => task,
                Err(error) => {
                    envd_task.abort();
                    code_interpreter_task.abort();
                    let _ = session.stop_session().await;
                    self.release_active_reservation(&mut reservation).await;
                    return Err(error);
                }
            };
            benchmark_samples.push(BenchmarkSample::new(
                "start.hot_heartbeat_start_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                heartbeat_started.elapsed().as_secs_f64() * 1000.0,
            ));
            let mut state = self.state.lock().await;
            state.benchmark_samples.extend(benchmark_samples);
            state
                .benchmark_event_traces
                .extend_from_slice(&benchmark_event_traces);
            state.benchmark_event_traces.push(readiness_trace.finish());
            state.active.insert(
                sandbox_id.clone(),
                FirkinRuntimeSession {
                    session: Arc::new(Mutex::new(session)),
                    reservation,
                    resume_started: started,
                    resume_snapshot_to_first_stdout_recorded: false,
                    startup_trace: Some(startup_trace),
                    envd_port,
                    envd_task,
                    code_interpreter_port,
                    code_interpreter_task,
                    active_vm_heartbeat_task,
                    freshness_sync: freshness_sync.clone(),
                    freshness_sync_checkout: freshness_sync_checkout.clone(),
                    continuation_snapshot_saved: false,
                },
            );
            if start_freshness_sync {
                self.spawn_freshness_sync_task(sandbox_id);
            }
            return Ok(runtime_sandbox);
        }
        let (sandbox_id, runtime_config, mut launcher, evicted_warm_sessions) = {
            let disk_root = manifest.path().parent().unwrap_or(Path::new("/"));
            let mut disk_probe = HostDiskPressureProbe::new();
            RuntimeDiskPressureGuard::new(disk_root, self.restore_minimum_free_disk)
                .check(&mut disk_probe)
                .map_err(|error| disk_pressure_to_capacity_error(&error))
                .map_err(|error| {
                    runtime_create_error_to_backend(RuntimeCubeSandboxCreateError::Restore(
                        SnapshotRestoreError::<L::Error>::Capacity(error),
                    ))
                })?;
            let admission = self.reserve_active_restore_capacity().await?;
            let mut state = self.state.lock().await;
            state.next_sandbox = state.next_sandbox.saturating_add(1).max(1);
            let sandbox_id = format!("sbx_firkin_{}", state.next_sandbox);
            let runtime_config =
                self.next_config(sandbox_id.clone(), request.create_request.timeout);
            (
                sandbox_id,
                runtime_config,
                admission.launcher,
                admission.evicted_warm_sessions,
            )
        };
        for mut session in evicted_warm_sessions {
            let _ = session.stop_session().await;
        }
        let create_config = RuntimeCubeSandboxCreateConfig::new(
            runtime_config.sandbox_id.clone(),
            runtime_config.domain.clone(),
            runtime_config.envd_version.clone(),
            runtime_config.started_at.clone(),
            runtime_config.end_at.clone(),
            runtime_config.cpu_count,
            runtime_config.memory_mb,
        );
        let mut startup_trace = EventTraceRecorder::new(
            LifecycleClass::Warm,
            WorkloadClass::TinyExec,
            RuntimeProfile::FastAgent,
        );
        startup_trace.record(SandboxEventName::RequestStart);
        startup_trace.record(SandboxEventName::SnapshotRestoreStart);
        let restore_request = SnapshotRestoreRequest::new(&manifest, self.budget);
        let mut session = match launcher.restore_from_snapshot(&restore_request).await {
            Ok(session) => session,
            Err(source) => {
                self.release_active_budget(self.budget).await;
                return Err(runtime_create_error_to_backend(
                    RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::Launch { source }),
                ));
            }
        };
        startup_trace.record(SandboxEventName::SnapshotRestoreDone);
        let runtime_sandbox = RuntimeSandbox {
            config: create_config.into_runtime_config(),
            exposed_ports: vec![
                DEFAULT_ENVD_PORT,
                DEFAULT_CODE_INTERPRETER_PORT,
                DEFAULT_MCP_PORT,
            ],
        };
        let mut benchmark_samples = vec![BenchmarkSample::new(
            "warm_snapshot_restore",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            started.elapsed().as_secs_f64() * 1000.0,
        )];
        let mut reservation = ActiveSessionReservation::new(self.budget);
        let readiness_trace = EventTraceRecorder::new(
            LifecycleClass::Warm,
            WorkloadClass::ReadinessProbe,
            RuntimeProfile::FastAgent,
        );
        let ready_started = Instant::now();
        let readiness_report = match session.probe_ready(readiness_trace).await {
            Ok(report) => report,
            Err(error) => {
                let _ = session.stop_session().await;
                self.release_active_reservation(&mut reservation).await;
                return Err(BackendError::Runtime(format!(
                    "Firkin readiness probe failed: {error}"
                )));
            }
        };
        startup_trace.record(SandboxEventName::ReadyProbePassed);
        benchmark_samples.push(BenchmarkSample::new(
            "ready_probe",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            ready_started.elapsed().as_secs_f64() * 1000.0,
        ));
        let (envd_port, envd_task) = match self.spawn_envd_server(sandbox_id.clone()).await {
            Ok(started) => started,
            Err(error) => {
                let _ = session.stop_session().await;
                self.release_active_reservation(&mut reservation).await;
                return Err(error);
            }
        };
        let (code_interpreter_port, code_interpreter_task) =
            match self.spawn_code_interpreter_server(sandbox_id.clone()).await {
                Ok(started) => started,
                Err(error) => {
                    envd_task.abort();
                    let _ = session.stop_session().await;
                    self.release_active_reservation(&mut reservation).await;
                    return Err(error);
                }
            };
        let active_vm_heartbeat_task = match self.start_active_vm_heartbeat(&sandbox_id) {
            Ok(task) => task,
            Err(error) => {
                envd_task.abort();
                code_interpreter_task.abort();
                let _ = session.stop_session().await;
                self.release_active_reservation(&mut reservation).await;
                return Err(error);
            }
        };
        let mut state = self.state.lock().await;
        state.benchmark_samples.extend(benchmark_samples);
        state
            .benchmark_event_traces
            .extend_from_slice(readiness_report.benchmark_event_traces());
        state.active.insert(
            sandbox_id.clone(),
            FirkinRuntimeSession {
                session: Arc::new(Mutex::new(session)),
                reservation,
                resume_started: started,
                resume_snapshot_to_first_stdout_recorded: false,
                startup_trace: Some(startup_trace),
                envd_port,
                envd_task,
                code_interpreter_port,
                code_interpreter_task,
                active_vm_heartbeat_task,
                freshness_sync,
                freshness_sync_checkout,
                continuation_snapshot_saved: false,
            },
        );
        state.restored_paths.push(restored_path);
        if start_freshness_sync {
            self.spawn_freshness_sync_task(sandbox_id);
        }
        Ok(runtime_sandbox)
    }
    async fn start_followup(
        &self,
        request: StartSandboxRequest,
        snapshot: FollowupSnapshot,
    ) -> Result<RuntimeSandbox, BackendError> {
        #[cfg(feature = "snapshot")]
        {
            let manifest = SnapshotArtifactManifest::continuation(
                snapshot.snapshot_id.clone(),
                snapshot.location.clone(),
            );
            snapshot_artifact_integrity::<L::Error>(
                snapshot.artifact_integrity.as_ref(),
                &manifest,
            )
            .and_then(|integrity| {
                integrity.verify(&manifest).map_err(|error| {
                    RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::Integrity {
                        reason: error.to_string(),
                    })
                })
            })
            .map_err(runtime_create_error_to_backend)?;
            let plan = ContinuationSnapshotPlan::new(
                snapshot.snapshot_id,
                ContinuationSnapshotReason::Idle,
                snapshot.location,
            );
            return FirkinRuntimeAdapter::start_followup(self, request, &plan).await;
        }
        #[cfg(not(feature = "snapshot"))]
        {
            let _ = request;
            let _ = snapshot;
            Err(BackendError::Runtime(
                "Firkin follow-up sandbox create requires the snapshot feature".to_owned(),
            ))
        }
    }
    async fn stop(&self, sandbox_id: &str) -> Result<(), BackendError> {
        let mut state = self.state.lock().await;
        let mut active = state
            .active
            .remove(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        let started = Instant::now();
        {
            let stop_result = {
                let mut session = active.session.lock().await;
                session.stop_session().await
            };
            if let Err(error) = stop_result {
                state.active.insert(sandbox_id.to_owned(), active);
                return Err(BackendError::Runtime(format!(
                    "Firkin session stop failed: {error}"
                )));
            }
        }
        let cleanup_result = if active.continuation_snapshot_saved {
            Ok(())
        } else {
            let session = active.session.lock().await;
            session.cleanup_unsnapshotted_staging().await
        };
        active.envd_task.abort();
        active.code_interpreter_task.abort();
        if let Some(task) = active.active_vm_heartbeat_task.take() {
            task.abort();
        }
        let released_capacity = active.reservation.release_into(&mut state.ledger).is_some();
        state
            .processes
            .retain(|key, _| key.sandbox_id != sandbox_id);
        state
            .interactive_processes
            .retain(|key, _| key.sandbox_id != sandbox_id);
        let marker_result = self.remove_active_vm_marker(sandbox_id);
        state.benchmark_samples.push(BenchmarkSample::new(
            "kill_delete",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            started.elapsed().as_secs_f64() * 1000.0,
        ));
        if released_capacity {
            self.notify_active_capacity_released();
        }
        if let Err(error) = cleanup_result {
            return Err(BackendError::Runtime(format!(
                "Firkin restored staging cleanup failed: {error}"
            )));
        }
        marker_result?;
        Ok(())
    }
    async fn pause(&self, sandbox_id: &str) -> Result<PausedSandbox, BackendError> {
        Err(BackendError::Runtime(format!(
            "Firkin RuntimeAdapter does not pause `{sandbox_id}`; use continuation snapshot capture"
        )))
    }
    async fn resume(&self, paused: PausedSandbox) -> Result<RuntimeSandbox, BackendError> {
        Err(BackendError::Runtime(format!(
            "Firkin RuntimeAdapter does not resume `{}` from E2B pause; restore a continuation snapshot as a new create",
            paused.sandbox_id
        )))
    }
    async fn snapshot(
        &self,
        sandbox_id: &str,
        name: Option<String>,
    ) -> Result<SnapshotRef, BackendError> {
        #[cfg(feature = "snapshot")]
        {
            let snapshot_id = name.unwrap_or_else(|| format!("{sandbox_id}-continuation"));
            let snapshot_path = runtime_continuation_snapshot_path(&snapshot_id);
            if snapshot_path.exists() {
                return Err(BackendError::AlreadyExists(snapshot_id));
            }
            if let Some(parent) = snapshot_path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|error| {
                    BackendError::Runtime(format!(
                        "create Firkin continuation snapshot dir: {error}"
                    ))
                })?;
            }
            let session = {
                let mut state = self.state.lock().await;
                state
                    .active
                    .get_mut(sandbox_id)
                    .map(|active| {
                        active.continuation_snapshot_saved = true;
                        Arc::clone(&active.session)
                    })
                    .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?
            };
            let started = Instant::now();
            {
                let session = session.lock().await;
                if let Err(error) = session.save_continuation_snapshot(&snapshot_path).await {
                    let mut state = self.state.lock().await;
                    if let Some(active) = state.active.get_mut(sandbox_id) {
                        active.continuation_snapshot_saved = false;
                    }
                    return Err(BackendError::Runtime(format!(
                        "Firkin continuation snapshot capture failed: {error}"
                    )));
                }
            }
            let manifest = SnapshotArtifactManifest::continuation(&snapshot_id, &snapshot_path);
            let integrity = SnapshotArtifactIntegrity::from_file(&manifest).map_err(|error| {
                BackendError::Runtime(format!(
                    "Firkin continuation snapshot integrity failed: {error}"
                ))
            })?;
            if let Err(error) = write_snapshot_artifact_sidecars(&manifest) {
                let mut state = self.state.lock().await;
                if let Some(active) = state.active.get_mut(sandbox_id) {
                    active.continuation_snapshot_saved = false;
                }
                return Err(BackendError::Runtime(format!(
                    "Firkin continuation snapshot sidecar write failed: {error}"
                )));
            }
            let mut state = self.state.lock().await;
            state.benchmark_samples.push(BenchmarkSample::new(
                "snapshot_save",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                started.elapsed().as_secs_f64() * 1000.0,
            ));
            return Ok(SnapshotRef {
                snapshot_id,
                location: Some(snapshot_path.display().to_string()),
                artifact_integrity: Some(PreparedTemplateArtifactIntegrity {
                    size_bytes: integrity.size_bytes(),
                    sha256_hex: integrity.sha256_hex().to_owned(),
                }),
            });
        }
        #[cfg(not(feature = "snapshot"))]
        {
            let _ = sandbox_id;
            let _ = name;
            Err(BackendError::Runtime(
                "Firkin continuation snapshot capture requires the snapshot feature".to_owned(),
            ))
        }
    }
    async fn delete_snapshot(&self, snapshot_id: &str) -> Result<(), BackendError> {
        #[cfg(feature = "snapshot")]
        {
            let snapshot_path = runtime_continuation_snapshot_path(snapshot_id);
            delete_firkin_snapshot_file(&snapshot_path).await?;
            delete_firkin_snapshot_file(&SnapshotArtifactManifest::sidecar_path_for_artifact(
                &snapshot_path,
            ))
            .await?;
            delete_firkin_snapshot_file(&SnapshotArtifactIntegrity::sidecar_path_for_artifact(
                &snapshot_path,
            ))
            .await?;
            Ok(())
        }
        #[cfg(not(feature = "snapshot"))]
        {
            let _ = snapshot_id;
            Err(BackendError::Runtime(
                "Firkin continuation snapshot deletion requires the snapshot feature".to_owned(),
            ))
        }
    }
    async fn metrics(&self, sandbox_id: &str) -> Result<Vec<SandboxMetric>, BackendError> {
        if self.state.lock().await.active.contains_key(sandbox_id) {
            Ok(Vec::new())
        } else {
            Err(BackendError::NotFound(sandbox_id.to_owned()))
        }
    }
    async fn logs(&self, sandbox_id: &str) -> Result<SandboxLogs, BackendError> {
        if self.state.lock().await.active.contains_key(sandbox_id) {
            Ok(SandboxLogs { logs: Vec::new() })
        } else {
            Err(BackendError::NotFound(sandbox_id.to_owned()))
        }
    }
    async fn apply_network(
        &self,
        _sandbox_id: &str,
        _policy: SandboxNetworkPolicy,
    ) -> Result<(), BackendError> {
        Err(BackendError::Runtime(
            "Firkin RuntimeAdapter does not enforce E2B network policy".to_owned(),
        ))
    }
    async fn port_target(&self, sandbox_id: &str, port: u16) -> Result<PortTarget, BackendError> {
        let state = self.state.lock().await;
        let session = state
            .active
            .get(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        if port == DEFAULT_ENVD_PORT {
            return Ok(PortTarget::Tcp {
                host: "127.0.0.1".to_owned(),
                port: session.envd_port,
            });
        }
        if port == DEFAULT_CODE_INTERPRETER_PORT {
            return Ok(PortTarget::Tcp {
                host: "127.0.0.1".to_owned(),
                port: session.code_interpreter_port,
            });
        }
        Ok(PortTarget::Vsock {
            cid: FIRKIN_ADAPTER_VSOCK_CID,
            port: u32::from(port),
        })
    }
    async fn connect_port_target(
        &self,
        sandbox_id: &str,
        target: PortTarget,
    ) -> Result<PortProxyStream, BackendError> {
        let PortTarget::Vsock { cid, port } = target else {
            return match target {
                PortTarget::Tcp { host, port } => {
                    let stream =
                        TcpStream::connect((host.as_str(), port))
                            .await
                            .map_err(|error| {
                                BackendError::Runtime(format!(
                                    "failed to connect Firkin proxy target {host}:{port}: {error}"
                                ))
                            })?;
                    stream.set_nodelay(true).map_err(|error| {
                        BackendError::Runtime(format!(
                            "failed to configure Firkin proxy target {host}:{port} TCP_NODELAY: {error}"
                        ))
                    })?;
                    Ok(Box::new(stream) as PortProxyStream)
                }
                PortTarget::UnixSocket { path } => UnixStream::connect(&path)
                    .await
                    .map(|stream| Box::new(stream) as PortProxyStream)
                    .map_err(|error| {
                        BackendError::Runtime(format!(
                            "failed to connect Firkin proxy target {path}: {error}"
                        ))
                    }),
                PortTarget::Vsock { .. } => unreachable!("vsock target handled above"),
            };
        };
        if cid != FIRKIN_ADAPTER_VSOCK_CID {
            return Err(BackendError::Runtime(format!(
                "Firkin RuntimeAdapter cannot route non-Firkin vsock cid {cid}"
            )));
        }
        let port = u16::try_from(port).map_err(|_| {
            BackendError::Runtime(format!("Firkin RuntimeAdapter port {port} is out of range"))
        })?;
        let state = self.state.lock().await;
        let session = state
            .active
            .get(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        session
            .session
            .lock()
            .await
            .connect_port(port)
            .await
            .map_err(|error| BackendError::Runtime(format!("Firkin port routing failed: {error}")))
    }
}
#[async_trait]
impl<L> EnvdProcessAdapter for FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher + Send + 'static,
    L::Error: Display + Send,
    L::Session: RuntimeCommandRunner
        + RuntimeCommandStreamRunner
        + RuntimeInteractiveProcessRunner
        + Send
        + Sync
        + 'static,
    <L::Session as RuntimeCommandRunner>::Error: Display + Send,
    <L::Session as RuntimeCommandStreamRunner>::Error: Display + Send,
    <L::Session as RuntimeInteractiveProcessRunner>::Error: Display + Send,
{
    type Error = BackendError;

    async fn list_processes(&self) -> Result<Vec<EnvdProcessInfo>, BackendError> {
        self.list_processes_for(None).await
    }
    async fn send_process_input(
        &self,
        selector: EnvdProcessSelector,
        input: EnvdProcessInput,
    ) -> Result<(), BackendError> {
        self.send_process_input_for(None, selector, input).await
    }
    async fn close_process_stdin(&self, selector: EnvdProcessSelector) -> Result<(), BackendError> {
        self.close_process_stdin_for(None, selector).await
    }
    async fn signal_process(
        &self,
        selector: EnvdProcessSelector,
        signal: EnvdProcessSignal,
    ) -> Result<(), BackendError> {
        self.signal_process_for(None, selector, signal).await
    }
    async fn connect_process(
        &self,
        selector: EnvdProcessSelector,
    ) -> Result<EnvdProcessOutput, BackendError> {
        self.connect_process_for(None, selector).await
    }
    async fn update_process_pty(
        &self,
        selector: EnvdProcessSelector,
        pty: Option<EnvdPtySize>,
    ) -> Result<(), BackendError> {
        self.update_process_pty_for(None, selector, pty).await
    }
    async fn start_process(
        &self,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessOutput, BackendError> {
        self.start_process_for(None, request).await
    }
    async fn start_process_stream(
        &self,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<BackendError>, BackendError> {
        self.start_process_stream_for(None, request).await
    }
}
impl<L> FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher + Send + 'static,
    L::Error: Display + Send,
    L::Session: RuntimeCommandRunner
        + RuntimeCommandStreamRunner
        + RuntimeInteractiveProcessRunner
        + Send
        + Sync
        + 'static,
    <L::Session as RuntimeCommandRunner>::Error: Display + Send,
    <L::Session as RuntimeCommandStreamRunner>::Error: Display + Send,
    <L::Session as RuntimeInteractiveProcessRunner>::Error: Display + Send,
{
    async fn process_session_for(
        &self,
        sandbox_id: Option<&str>,
        operation: &'static str,
    ) -> Result<(String, Arc<Mutex<L::Session>>), BackendError> {
        let session = {
            let state = self.state.lock().await;
            if let Some(sandbox_id) = sandbox_id {
                return state
                    .active
                    .get(sandbox_id)
                    .map(|active| (sandbox_id.to_owned(), Arc::clone(&active.session)))
                    .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()));
            }
            if state.active.len() != 1 {
                return Err(BackendError::Runtime(format!(
                    "Firkin RuntimeAdapter envd process {operation} requires exactly one active sandbox, found {}",
                    state.active.len()
                )));
            }
            let (sandbox_id, active) = state.active.iter().next().expect("active length checked");
            Ok((sandbox_id.clone(), Arc::clone(&active.session)))
        }?;
        Ok(session)
    }
    async fn start_process_for(
        &self,
        sandbox_id: Option<&str>,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessOutput, BackendError> {
        let (sandbox_id, session) = self.process_session_for(sandbox_id, "start").await?;
        if request.stdin == Some(true) || request.pty.is_some() {
            let report = session
                .lock()
                .await
                .start_interactive_process(&request)
                .await
                .map_err(|error| {
                    BackendError::Runtime(format!(
                        "Firkin interactive process start failed: {error}"
                    ))
                })?;
            let mut state = self.state.lock().await;
            state
                .benchmark_samples
                .extend_from_slice(report.benchmark_samples());
            let (output, _, process) = report.into_parts();
            let key = FirkinRuntimeProcessKey {
                sandbox_id,
                pid: output.pid,
            };
            state.processes.insert(
                key.clone(),
                FirkinRuntimeProcessRecord {
                    info: EnvdProcessInfo {
                        pid: output.pid,
                        tag: request.tag,
                        cmd: request.cmd,
                        args: request.args,
                        envs: request.envs,
                        cwd: request.cwd,
                    },
                    output: output.clone(),
                },
            );
            state.interactive_processes.insert(key, process);
            return Ok(output);
        }
        let event_trace = self.take_startup_event_trace(&sandbox_id).await?;
        let command_started = Instant::now();
        let report = session
            .lock()
            .await
            .run_command(&request, event_trace)
            .await
            .map_err(|error| {
                BackendError::Runtime(format!("Firkin command start failed: {error}"))
            })?;
        self.persist_command_start_report(sandbox_id, request, command_started, report)
            .await
    }

    async fn start_process_stream_for(
        &self,
        sandbox_id: Option<&str>,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<BackendError>, BackendError> {
        if request.stdin == Some(true) || request.pty.is_some() {
            return self
                .start_interactive_process_stream_for(sandbox_id, request)
                .await;
        }
        let (sandbox_id, session) = self.process_session_for(sandbox_id, "start stream").await?;
        let event_trace = self.take_startup_event_trace(&sandbox_id).await?;
        let command_started = Instant::now();
        let report = session
            .lock()
            .await
            .run_command_stream(&request, event_trace)
            .await
            .map_err(|error| {
                BackendError::Runtime(format!("Firkin command stream start failed: {error}"))
            })?;
        let (pid, stream, completion) = report.into_parts();
        {
            let mut state = self.state.lock().await;
            state.processes.insert(
                FirkinRuntimeProcessKey {
                    sandbox_id: sandbox_id.clone(),
                    pid,
                },
                FirkinRuntimeProcessRecord {
                    info: EnvdProcessInfo {
                        pid,
                        tag: request.tag.clone(),
                        cmd: request.cmd.clone(),
                        args: request.args.clone(),
                        envs: request.envs.clone(),
                        cwd: request.cwd.clone(),
                    },
                    output: EnvdProcessOutput {
                        pid,
                        status: "running".to_owned(),
                        ..EnvdProcessOutput::default()
                    },
                },
            );
        }
        let adapter = self.clone();
        tokio::spawn(async move {
            if let Ok(completion) = completion.await {
                let _ = adapter
                    .persist_command_stream_completion(
                        sandbox_id,
                        request,
                        command_started,
                        completion,
                    )
                    .await;
            }
        });
        Ok(stream)
    }

    async fn start_interactive_process_stream_for(
        &self,
        sandbox_id: Option<&str>,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<BackendError>, BackendError> {
        let (sandbox_id, session) = self.process_session_for(sandbox_id, "start stream").await?;
        let report = session
            .lock()
            .await
            .start_interactive_process(&request)
            .await
            .map_err(|error| {
                BackendError::Runtime(format!(
                    "Firkin interactive process stream start failed: {error}"
                ))
            })?;
        let mut state = self.state.lock().await;
        state
            .benchmark_samples
            .extend_from_slice(report.benchmark_samples());
        let (output, _, process, live_stream) = report.into_parts_with_stream();
        let pid = output.pid;
        let key = FirkinRuntimeProcessKey { sandbox_id, pid };
        state.processes.insert(
            key.clone(),
            FirkinRuntimeProcessRecord {
                info: EnvdProcessInfo {
                    pid,
                    tag: request.tag,
                    cmd: request.cmd,
                    args: request.args,
                    envs: request.envs,
                    cwd: request.cwd,
                },
                output: output.clone(),
            },
        );
        state.interactive_processes.insert(key, process);
        drop(state);

        let Some(mut live_stream) = live_stream else {
            return Ok(EnvdProcessEventStream::from_output(&output));
        };
        let (sender, receiver) = mpsc::channel(32);
        sender
            .try_send(Ok(EnvdProcessStreamEvent::Start { pid }))
            .expect("fresh process event stream channel has capacity");
        tokio::spawn(async move {
            while let Some(event) = live_stream.recv().await {
                forward_interactive_process_event(&sender, event).await;
            }
        });
        Ok(EnvdProcessEventStream::from_receiver(receiver))
    }

    async fn take_startup_event_trace(
        &self,
        sandbox_id: &str,
    ) -> Result<EventTraceRecorder, BackendError> {
        let mut state = self.state.lock().await;
        let active = state
            .active
            .get_mut(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        Ok(active.startup_trace.take().unwrap_or_else(|| {
            EventTraceRecorder::new(
                LifecycleClass::Hot,
                WorkloadClass::TinyExec,
                RuntimeProfile::FastAgent,
            )
        }))
    }

    async fn persist_command_start_report(
        &self,
        sandbox_id: String,
        request: EnvdProcessStartRequest,
        command_started: Instant,
        report: RuntimeCommandStartReport,
    ) -> Result<EnvdProcessOutput, BackendError> {
        let (output, benchmark_samples, event_traces) = report.into_parts();
        let mut state = self.state.lock().await;
        let mut first_command_samples = Vec::new();
        let resume_to_first_stdout = state.active.get_mut(&sandbox_id).and_then(|active| {
            if active.resume_snapshot_to_first_stdout_recorded {
                return None;
            }
            let first_stdout_ms = benchmark_samples
                .iter()
                .find(|sample| sample.metric() == "first_stdout_byte")
                .map(BenchmarkSample::value)?;
            active.resume_snapshot_to_first_stdout_recorded = true;
            first_command_samples
                .extend(sandbox_first_command_samples(&request, &benchmark_samples));
            Some(BenchmarkSample::new(
                "start.resume_to_first_stdout_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                command_started
                    .duration_since(active.resume_started)
                    .as_secs_f64()
                    * 1000.0
                    + first_stdout_ms,
            ))
        });
        state
            .benchmark_samples
            .extend_from_slice(&benchmark_samples);
        state
            .benchmark_event_traces
            .extend_from_slice(&event_traces);
        if let Some(sample) = resume_to_first_stdout {
            state.benchmark_samples.push(sample);
        }
        state.benchmark_samples.extend(first_command_samples);
        state.processes.insert(
            FirkinRuntimeProcessKey {
                sandbox_id,
                pid: output.pid,
            },
            FirkinRuntimeProcessRecord {
                info: EnvdProcessInfo {
                    pid: output.pid,
                    tag: request.tag,
                    cmd: request.cmd,
                    args: request.args,
                    envs: request.envs,
                    cwd: request.cwd,
                },
                output: output.clone(),
            },
        );
        Ok(output)
    }

    async fn persist_command_stream_completion(
        &self,
        sandbox_id: String,
        request: EnvdProcessStartRequest,
        command_started: Instant,
        completion: RuntimeCommandStreamCompletion,
    ) -> Result<(), BackendError> {
        let (output, benchmark_samples, event_traces) = completion.into_parts();
        let report = RuntimeCommandStartReport::new(output, benchmark_samples, event_traces);
        self.persist_command_start_report(sandbox_id, request, command_started, report)
            .await
            .map(drop)
    }
}

fn sandbox_first_command_samples(
    request: &EnvdProcessStartRequest,
    command_samples: &[BenchmarkSample],
) -> Vec<BenchmarkSample> {
    let Some(command_start_ms) = command_samples
        .iter()
        .find(|sample| sample.metric() == "command_start")
        .map(BenchmarkSample::value)
    else {
        return Vec::new();
    };
    let Some(first_stdout_ms) = command_samples
        .iter()
        .find(|sample| sample.metric() == "first_stdout_byte")
        .map(BenchmarkSample::value)
    else {
        return Vec::new();
    };
    let command_args = request.args.join("|||");
    vec![
        BenchmarkSample::new(
            "debug.exec.sandbox_first_command_start_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            command_start_ms,
        )
        .with_dynamic_tag("cmd", request.cmd.clone())
        .with_dynamic_tag("args", command_args.clone()),
        BenchmarkSample::new(
            "debug.exec.sandbox_first_stdout_byte_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            first_stdout_ms,
        )
        .with_dynamic_tag("cmd", request.cmd.clone())
        .with_dynamic_tag("args", command_args),
    ]
}

impl<L> FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher,
{
    async fn list_processes_for(
        &self,
        sandbox_id: Option<&str>,
    ) -> Result<Vec<EnvdProcessInfo>, BackendError> {
        let state = self.state.lock().await;
        Ok(state
            .processes
            .iter()
            .filter(|(key, _)| sandbox_id.is_none_or(|sandbox_id| key.sandbox_id == sandbox_id))
            .map(|(_, record)| record.info.clone())
            .collect())
    }
    async fn send_process_input_for(
        &self,
        sandbox_id: Option<&str>,
        selector: EnvdProcessSelector,
        input: EnvdProcessInput,
    ) -> Result<(), BackendError> {
        match self
            .take_interactive_process(sandbox_id, selector.clone())
            .await
        {
            Ok((key, mut process)) => {
                let started = Instant::now();
                let result = process.send_input(input).await;
                let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
                let mut state = self.state.lock().await;
                state.interactive_processes.insert(key, process);
                if result.is_ok() {
                    state.benchmark_samples.push(
                        BenchmarkSample::new(
                            "sandbox.exec.stdin_write_latency_ms",
                            BenchmarkMetricKind::LifecycleLatency,
                            BenchmarkUnit::Milliseconds,
                            latency_ms,
                        )
                        .with_static_tag("measurement_boundary", "runtime_process_write_flush")
                        .with_static_tag("excludes_client_http", "true")
                        .with_static_tag("excludes_envd_rpc_decode", "true"),
                    );
                }
                result
            }
            Err(BackendError::Runtime(_)) => {
                let process = self.require_recorded_process(sandbox_id, selector).await?;
                Err(finite_process_error(
                    process.info.pid,
                    "does not keep interactive stdin",
                ))
            }
            Err(error) => Err(error),
        }
    }
    async fn close_process_stdin_for(
        &self,
        sandbox_id: Option<&str>,
        selector: EnvdProcessSelector,
    ) -> Result<(), BackendError> {
        match self
            .take_interactive_process(sandbox_id, selector.clone())
            .await
        {
            Ok((key, mut process)) => {
                let result = process.close_stdin().await;
                self.state
                    .lock()
                    .await
                    .interactive_processes
                    .insert(key, process);
                result
            }
            Err(BackendError::Runtime(_)) => {
                let process = self.require_recorded_process(sandbox_id, selector).await?;
                Err(finite_process_error(
                    process.info.pid,
                    "does not keep interactive stdin",
                ))
            }
            Err(error) => Err(error),
        }
    }
    async fn signal_process_for(
        &self,
        sandbox_id: Option<&str>,
        selector: EnvdProcessSelector,
        signal: EnvdProcessSignal,
    ) -> Result<(), BackendError> {
        match self
            .take_interactive_process(sandbox_id, selector.clone())
            .await
        {
            Ok((key, mut process)) => {
                let result = process.signal(signal).await;
                self.state
                    .lock()
                    .await
                    .interactive_processes
                    .insert(key, process);
                result
            }
            Err(BackendError::Runtime(_)) => {
                let process = self.require_recorded_process(sandbox_id, selector).await?;
                Err(finite_process_error(
                    process.info.pid,
                    "has already exited and cannot be signaled",
                ))
            }
            Err(error) => Err(error),
        }
    }
    async fn connect_process_for(
        &self,
        sandbox_id: Option<&str>,
        selector: EnvdProcessSelector,
    ) -> Result<EnvdProcessOutput, BackendError> {
        match self
            .take_interactive_process(sandbox_id, selector.clone())
            .await
        {
            Ok((key, mut process)) => {
                let result = process.connect().await;
                self.state
                    .lock()
                    .await
                    .interactive_processes
                    .insert(key, process);
                result
            }
            Err(BackendError::Runtime(_)) => self
                .require_recorded_process(sandbox_id, selector)
                .await
                .map(|record| record.output),
            Err(error) => Err(error),
        }
    }
    async fn update_process_pty_for(
        &self,
        sandbox_id: Option<&str>,
        selector: EnvdProcessSelector,
        pty: Option<EnvdPtySize>,
    ) -> Result<(), BackendError> {
        match self
            .take_interactive_process(sandbox_id, selector.clone())
            .await
        {
            Ok((key, mut process)) => {
                let result = process.update_pty(pty).await;
                self.state
                    .lock()
                    .await
                    .interactive_processes
                    .insert(key, process);
                result
            }
            Err(BackendError::Runtime(_)) => {
                let process = self.require_recorded_process(sandbox_id, selector).await?;
                Err(finite_process_error(
                    process.info.pid,
                    "does not keep an interactive PTY",
                ))
            }
            Err(error) => Err(error),
        }
    }
    async fn require_recorded_process(
        &self,
        sandbox_id: Option<&str>,
        selector: EnvdProcessSelector,
    ) -> Result<FirkinRuntimeProcessRecord, BackendError> {
        let state = self.state.lock().await;
        let record = match selector {
            EnvdProcessSelector::Pid(pid) => unique_process_match(
                state
                    .processes
                    .iter()
                    .filter(|(key, _)| {
                        key.pid == pid
                            && sandbox_id.is_none_or(|sandbox_id| key.sandbox_id == sandbox_id)
                    })
                    .map(|(_, record)| record),
                || BackendError::NotFound(pid.to_string()),
                || ambiguous_process_error("pid", &pid.to_string()),
            )?,
            EnvdProcessSelector::Tag(tag) => unique_process_match(
                state
                    .processes
                    .iter()
                    .filter(|(key, _)| {
                        sandbox_id.is_none_or(|sandbox_id| key.sandbox_id == sandbox_id)
                    })
                    .map(|(_, record)| record)
                    .filter(|record| record.info.tag.as_deref() == Some(tag.as_str())),
                || BackendError::NotFound(tag.clone()),
                || ambiguous_process_error("tag", &tag),
            )?,
        };
        Ok(record.clone())
    }
    async fn take_interactive_process(
        &self,
        sandbox_id: Option<&str>,
        selector: EnvdProcessSelector,
    ) -> Result<(FirkinRuntimeProcessKey, Box<dyn RuntimeInteractiveProcess>), BackendError> {
        let mut state = self.state.lock().await;
        let key = match selector {
            EnvdProcessSelector::Pid(pid) => unique_process_match(
                state.processes.keys().filter(|key| {
                    key.pid == pid
                        && sandbox_id.is_none_or(|sandbox_id| key.sandbox_id == sandbox_id)
                }),
                || BackendError::NotFound(pid.to_string()),
                || ambiguous_process_error("pid", &pid.to_string()),
            )?
            .clone(),
            EnvdProcessSelector::Tag(tag) => unique_process_match(
                state
                    .processes
                    .iter()
                    .filter(|(key, _)| {
                        sandbox_id.is_none_or(|sandbox_id| key.sandbox_id == sandbox_id)
                    })
                    .filter(|(_, record)| record.info.tag.as_deref() == Some(tag.as_str()))
                    .map(|(key, _)| key),
                || BackendError::NotFound(tag.clone()),
                || ambiguous_process_error("tag", &tag),
            )?
            .clone(),
        };
        state
            .interactive_processes
            .remove(&key)
            .map(|process| (key.clone(), process))
            .ok_or_else(|| {
                finite_process_error(
                    key.pid,
                    "does not keep interactive stdin, signal, or PTY state",
                )
            })
    }
}
impl<L> FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher + Send + 'static,
    L::Session: RuntimeCommandRunner + Send + Sync + 'static,
    <L::Session as RuntimeCommandRunner>::Error: Display + Send,
{
    fn spawn_freshness_sync_task(&self, sandbox_id: String) {
        let adapter = self.clone();
        tokio::spawn(async move {
            if let Err(error) = adapter.run_freshness_sync(&sandbox_id).await {
                let _ = adapter
                    .fail_freshness_sync(&sandbox_id, error.to_string())
                    .await;
            }
        });
    }
    /// Run the pending freshness sync inside an active restored runtime session.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError`] when the sandbox is missing, the sync metadata
    /// is incomplete, or any runtime git command fails.
    #[allow(clippy::too_many_lines)]
    pub async fn run_freshness_sync(&self, sandbox_id: &str) -> Result<(), BackendError> {
        let (session, branch, target, checkout_dir) = {
            let state = self.state.lock().await;
            let active = state
                .active
                .get(sandbox_id)
                .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
            let gate = active.freshness_sync.as_ref().ok_or_else(|| {
                BackendError::Runtime(format!(
                    "Firkin sandbox `{sandbox_id}` does not have pending freshness sync"
                ))
            })?;
            if gate.writes_allowed() {
                return Ok(());
            }
            let target = gate.sync_target().ok_or_else(|| {
                BackendError::Runtime(format!(
                    "Firkin sandbox `{sandbox_id}` freshness sync is missing a target"
                ))
            })?;
            let checkout_dir = active
                .freshness_sync_checkout
                .clone()
                .ok_or_else(|| {
                    BackendError::Runtime(
                        format!(
                            "Firkin sandbox `{sandbox_id}` freshness sync is missing `{FRESHNESS_SYNC_CHECKOUT_METADATA}` metadata"
                        ),
                    )
                })?;
            (
                Arc::clone(&active.session),
                gate.branch().to_owned(),
                target.to_owned(),
                checkout_dir,
            )
        };
        let branch_name = branch.strip_prefix("refs/heads/").unwrap_or(&branch);
        let started = Instant::now();
        let commands = [
            EnvdProcessStartRequest {
                cmd: "git".to_owned(),
                args: vec![
                    "fetch".to_owned(),
                    "--quiet".to_owned(),
                    "origin".to_owned(),
                    branch_name.to_owned(),
                ],
                cwd: Some(checkout_dir.clone()),
                ..EnvdProcessStartRequest::default()
            },
            EnvdProcessStartRequest {
                cmd: "git".to_owned(),
                args: vec![
                    "checkout".to_owned(),
                    "--quiet".to_owned(),
                    branch_name.to_owned(),
                ],
                cwd: Some(checkout_dir.clone()),
                ..EnvdProcessStartRequest::default()
            },
            EnvdProcessStartRequest {
                cmd: "git".to_owned(),
                args: vec![
                    "reset".to_owned(),
                    "--hard".to_owned(),
                    "--quiet".to_owned(),
                    target.clone(),
                ],
                cwd: Some(checkout_dir),
                ..EnvdProcessStartRequest::default()
            },
        ];
        for command in commands {
            let report = session
                .lock()
                .await
                .run_command(
                    &command,
                    EventTraceRecorder::new(
                        LifecycleClass::Hot,
                        WorkloadClass::TinyExec,
                        RuntimeProfile::FastAgent,
                    ),
                )
                .await
                .map_err(|error| {
                    BackendError::Runtime(format!("Firkin runtime freshness sync failed: {error}"))
                })?;
            let (output, samples, event_traces) = report.into_parts();
            {
                let mut state = self.state.lock().await;
                state.benchmark_samples.extend_from_slice(&samples);
                state
                    .benchmark_event_traces
                    .extend_from_slice(&event_traces);
            }
            if output.exit_code != 0 {
                let reason = output
                    .error
                    .unwrap_or_else(|| format!("process exited {}", output.exit_code));
                self.fail_freshness_sync(sandbox_id, reason.clone()).await?;
                return Err(BackendError::Runtime(format!(
                    "Firkin runtime freshness sync command `{}` failed: {reason}",
                    command.cmd
                )));
            }
        }
        let mut state = self.state.lock().await;
        let active = state
            .active
            .get_mut(sandbox_id)
            .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?;
        if let Some(gate) = active.freshness_sync.take() {
            active.freshness_sync = Some(gate.complete_sync(target));
        }
        state.benchmark_samples.push(BenchmarkSample::new(
            "freshness_sync",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            started.elapsed().as_secs_f64() * 1000.0,
        ));
        Ok(())
    }
    async fn run_filesystem_command_for(
        &self,
        sandbox_id: Option<&str>,
        operation: &'static str,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessOutput, BackendError> {
        let session = {
            let state = self.state.lock().await;
            if let Some(sandbox_id) = sandbox_id {
                state
                    .active
                    .get(sandbox_id)
                    .map(|active| Arc::clone(&active.session))
                    .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?
            } else if state.active.len() != 1 {
                return Err(BackendError::Runtime(format!(
                    "Firkin RuntimeAdapter envd filesystem {operation} requires exactly one active sandbox, found {}",
                    state.active.len()
                )));
            } else {
                Arc::clone(
                    &state
                        .active
                        .values()
                        .next()
                        .expect("active length checked")
                        .session,
                )
            }
        };
        let report = session
            .lock()
            .await
            .run_command(
                &request,
                EventTraceRecorder::new(
                    LifecycleClass::Hot,
                    WorkloadClass::TinyExec,
                    RuntimeProfile::FastAgent,
                ),
            )
            .await
            .map_err(|error| {
                BackendError::Runtime(format!("Firkin filesystem {operation} failed: {error}"))
            })?;
        let (output, samples, event_traces) = report.into_parts();
        {
            let mut state = self.state.lock().await;
            state.benchmark_samples.extend_from_slice(&samples);
            state
                .benchmark_event_traces
                .extend_from_slice(&event_traces);
        }
        Ok(output)
    }
    async fn ensure_filesystem_write_allowed(
        &self,
        sandbox_id: Option<&str>,
        operation: &'static str,
    ) -> Result<(), BackendError> {
        let state = self.state.lock().await;
        let (sandbox_id, active) = if let Some(sandbox_id) = sandbox_id {
            (
                sandbox_id.to_owned(),
                state
                    .active
                    .get(sandbox_id)
                    .ok_or_else(|| BackendError::NotFound(sandbox_id.to_owned()))?,
            )
        } else if state.active.len() != 1 {
            return Err(BackendError::Runtime(format!(
                "Firkin RuntimeAdapter envd filesystem {operation} requires exactly one active sandbox, found {}",
                state.active.len()
            )));
        } else {
            state
                .active
                .iter()
                .next()
                .map(|(sandbox_id, active)| (sandbox_id.clone(), active))
                .expect("active length checked")
        };
        if let Some(gate) = &active.freshness_sync
            && !gate.writes_allowed()
        {
            return Err(BackendError::Runtime(format!(
                "Firkin freshness sync for sandbox `{sandbox_id}` blocks write operation `{operation}` on branch `{}` while phase is {:?}",
                gate.branch(),
                gate.phase()
            )));
        }
        Ok(())
    }
}
#[async_trait]
impl<L> EnvdFilesystemAdapter for FirkinRuntimeAdapter<L>
where
    L: SnapshotSessionLauncher + Send + 'static,
    L::Session: RuntimeCommandRunner + Send + Sync + 'static,
    <L::Session as RuntimeCommandRunner>::Error: Display + Send,
{
    type Error = BackendError;

    async fn read_file(&self, path: String) -> Result<Vec<u8>, BackendError> {
        let output = self
            .run_filesystem_command_for(
                None,
                "read",
                EnvdProcessStartRequest {
                    cmd: "/bin/cat".to_owned(),
                    args: vec![path.clone()],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(BackendError::Runtime(format!(
                "Firkin filesystem read `{path}` failed: {}",
                output
                    .error
                    .unwrap_or_else(|| format!("process exited {}", output.exit_code))
            )));
        }
        Ok(output.stdout)
    }
    async fn write_file(
        &self,
        path: String,
        data: Vec<u8>,
    ) -> Result<EnvdFilesystemWriteInfo, BackendError> {
        self.ensure_filesystem_write_allowed(None, "write").await?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        let output = self
            .run_filesystem_command_for(
                None,
                "write",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "mkdir -p -- \"$(dirname -- \"$2\")\" && printf '%s' \"$1\" | base64 -d > \"$2\""
                        .to_owned(), "firkin-write-file".to_owned(), encoded, path
                        .clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(BackendError::Runtime(format!(
                "Firkin filesystem write `{path}` failed: {}",
                output
                    .error
                    .unwrap_or_else(|| format!("process exited {}", output.exit_code))
            )));
        }
        Ok(EnvdFilesystemWriteInfo {
            name: path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(&path)
                .to_owned(),
            file_type: "file".to_owned(),
            path,
        })
    }
    async fn list_dir(
        &self,
        path: String,
        depth: u32,
    ) -> Result<Vec<EnvdFilesystemEntry>, BackendError> {
        let depth = depth.to_string();
        let output = self
            .run_filesystem_command_for(
                None,
                "list",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "if [ \"$2\" = 0 ]; then find \"$1\" -mindepth 1 -exec stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- {} \\;; else find \"$1\" -mindepth 1 -maxdepth \"$2\" -exec stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- {} \\;; fi"
                        .to_owned(), "firkin-list-dir".to_owned(), path.clone(), depth,
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("list", &path, output));
        }
        parse_filesystem_entries(&output.stdout)
    }
    async fn make_dir(&self, path: String) -> Result<EnvdFilesystemEntry, BackendError> {
        self.ensure_filesystem_write_allowed(None, "mkdir").await?;
        let output = self
            .run_filesystem_command_for(
                None,
                "mkdir",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "mkdir -p -- \"$1\" && stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- \"$1\""
                            .to_owned(),
                        "firkin-make-dir".to_owned(),
                        path.clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("mkdir", &path, output));
        }
        parse_single_filesystem_entry(&output.stdout)
    }
    async fn move_entry(
        &self,
        source: String,
        destination: String,
    ) -> Result<EnvdFilesystemEntry, BackendError> {
        self.ensure_filesystem_write_allowed(None, "move").await?;
        let output = self
            .run_filesystem_command_for(
                None,
                "move",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "mkdir -p -- \"$(dirname -- \"$2\")\" && mv -- \"$1\" \"$2\" && stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- \"$2\""
                        .to_owned(), "firkin-move-entry".to_owned(), source.clone(),
                        destination.clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("move", &source, output));
        }
        parse_single_filesystem_entry(&output.stdout)
    }
    async fn remove_entry(&self, path: String) -> Result<(), BackendError> {
        self.ensure_filesystem_write_allowed(None, "remove").await?;
        let output = self
            .run_filesystem_command_for(
                None,
                "remove",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "rm -rf -- \"$1\"".to_owned(),
                        "firkin-remove-entry".to_owned(),
                        path.clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("remove", &path, output));
        }
        Ok(())
    }
    async fn stat_entry(&self, path: String) -> Result<EnvdFilesystemEntry, BackendError> {
        let output = self
            .run_filesystem_command_for(
                None,
                "stat",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- \"$1\"".to_owned(),
                        "firkin-stat-entry".to_owned(),
                        path.clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(BackendError::NotFound(path));
        }
        parse_single_filesystem_entry(&output.stdout)
    }
    async fn watch_dir(
        &self,
        path: String,
        recursive: bool,
    ) -> Result<Vec<EnvdFilesystemEvent>, BackendError> {
        let recursive = recursive.to_string();
        let output = self
            .run_filesystem_command_for(
                None,
                "watch",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "test -e \"$1\"".to_owned(),
                        "firkin-watch-dir".to_owned(),
                        path.clone(),
                        recursive,
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("watch", &path, output));
        }
        Ok(vec![EnvdFilesystemEvent {
            name: path,
            event_type: EnvdFilesystemEventType::Write,
        }])
    }
}
struct FirkinRuntimeSandboxEnvdAdapter<L>
where
    L: SnapshotSessionLauncher,
{
    pub(crate) adapter: FirkinRuntimeAdapter<L>,
    #[allow(missing_docs)]
    pub sandbox_id: String,
}
impl<L> Clone for FirkinRuntimeSandboxEnvdAdapter<L>
where
    L: SnapshotSessionLauncher,
{
    fn clone(&self) -> Self {
        Self {
            adapter: self.adapter.clone(),
            sandbox_id: self.sandbox_id.clone(),
        }
    }
}
#[async_trait]
impl<L> EnvdProcessAdapter for FirkinRuntimeSandboxEnvdAdapter<L>
where
    L: SnapshotSessionLauncher + Send + 'static,
    L::Error: Display + Send,
    L::Session: RuntimeCommandRunner
        + RuntimeCommandStreamRunner
        + RuntimeInteractiveProcessRunner
        + Send
        + Sync
        + 'static,
    <L::Session as RuntimeCommandRunner>::Error: Display + Send,
    <L::Session as RuntimeCommandStreamRunner>::Error: Display + Send,
    <L::Session as RuntimeInteractiveProcessRunner>::Error: Display + Send,
{
    type Error = BackendError;

    async fn list_processes(&self) -> Result<Vec<EnvdProcessInfo>, BackendError> {
        self.adapter
            .list_processes_for(Some(&self.sandbox_id))
            .await
    }
    async fn send_process_input(
        &self,
        selector: EnvdProcessSelector,
        input: EnvdProcessInput,
    ) -> Result<(), BackendError> {
        self.adapter
            .send_process_input_for(Some(&self.sandbox_id), selector, input)
            .await
    }
    async fn close_process_stdin(&self, selector: EnvdProcessSelector) -> Result<(), BackendError> {
        self.adapter
            .close_process_stdin_for(Some(&self.sandbox_id), selector)
            .await
    }
    async fn signal_process(
        &self,
        selector: EnvdProcessSelector,
        signal: EnvdProcessSignal,
    ) -> Result<(), BackendError> {
        self.adapter
            .signal_process_for(Some(&self.sandbox_id), selector, signal)
            .await
    }
    async fn connect_process(
        &self,
        selector: EnvdProcessSelector,
    ) -> Result<EnvdProcessOutput, BackendError> {
        self.adapter
            .connect_process_for(Some(&self.sandbox_id), selector)
            .await
    }
    async fn update_process_pty(
        &self,
        selector: EnvdProcessSelector,
        pty: Option<EnvdPtySize>,
    ) -> Result<(), BackendError> {
        self.adapter
            .update_process_pty_for(Some(&self.sandbox_id), selector, pty)
            .await
    }
    async fn start_process(
        &self,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessOutput, BackendError> {
        self.adapter
            .start_process_for(Some(&self.sandbox_id), request)
            .await
    }
    async fn start_process_stream(
        &self,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<BackendError>, BackendError> {
        self.adapter
            .start_process_stream_for(Some(&self.sandbox_id), request)
            .await
    }
}
#[async_trait]
impl<L> EnvdFilesystemAdapter for FirkinRuntimeSandboxEnvdAdapter<L>
where
    L: SnapshotSessionLauncher + Send + 'static,
    L::Session: RuntimeCommandRunner + Send + Sync + 'static,
    <L::Session as RuntimeCommandRunner>::Error: Display + Send,
{
    type Error = BackendError;

    async fn read_file(&self, path: String) -> Result<Vec<u8>, BackendError> {
        let output = self
            .adapter
            .run_filesystem_command_for(
                Some(&self.sandbox_id),
                "read",
                EnvdProcessStartRequest {
                    cmd: "/bin/cat".to_owned(),
                    args: vec![path.clone()],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(BackendError::Runtime(format!(
                "Firkin filesystem read `{path}` failed: {}",
                output
                    .error
                    .unwrap_or_else(|| format!("process exited {}", output.exit_code))
            )));
        }
        Ok(output.stdout)
    }
    async fn write_file(
        &self,
        path: String,
        data: Vec<u8>,
    ) -> Result<EnvdFilesystemWriteInfo, BackendError> {
        self.adapter
            .ensure_filesystem_write_allowed(Some(&self.sandbox_id), "write")
            .await?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        let output = self
            .adapter
            .run_filesystem_command_for(
                Some(&self.sandbox_id),
                "write",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "mkdir -p -- \"$(dirname -- \"$2\")\" && printf '%s' \"$1\" | base64 -d > \"$2\""
                        .to_owned(), "firkin-write-file".to_owned(), encoded, path
                        .clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(BackendError::Runtime(format!(
                "Firkin filesystem write `{path}` failed: {}",
                output
                    .error
                    .unwrap_or_else(|| format!("process exited {}", output.exit_code))
            )));
        }
        Ok(EnvdFilesystemWriteInfo {
            name: path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(&path)
                .to_owned(),
            file_type: "file".to_owned(),
            path,
        })
    }
    async fn list_dir(
        &self,
        path: String,
        depth: u32,
    ) -> Result<Vec<EnvdFilesystemEntry>, BackendError> {
        let depth = depth.to_string();
        let output = self
            .adapter
            .run_filesystem_command_for(
                Some(&self.sandbox_id),
                "list",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "if [ \"$2\" = 0 ]; then find \"$1\" -mindepth 1 -exec stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- {} \\;; else find \"$1\" -mindepth 1 -maxdepth \"$2\" -exec stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- {} \\;; fi"
                        .to_owned(), "firkin-list-dir".to_owned(), path.clone(), depth,
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("list", &path, output));
        }
        parse_filesystem_entries(&output.stdout)
    }
    async fn make_dir(&self, path: String) -> Result<EnvdFilesystemEntry, BackendError> {
        self.adapter
            .ensure_filesystem_write_allowed(Some(&self.sandbox_id), "mkdir")
            .await?;
        let output =
            self.adapter
                .run_filesystem_command_for(
                    Some(&self.sandbox_id),
                    "mkdir",
                    EnvdProcessStartRequest {
                        cmd: "/bin/sh".to_owned(),
                        args: vec![
                        "-lc".to_owned(),
                        "mkdir -p -- \"$1\" && stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- \"$1\""
                        .to_owned(), "firkin-make-dir".to_owned(), path.clone(),
                    ],
                        ..EnvdProcessStartRequest::default()
                    },
                )
                .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("mkdir", &path, output));
        }
        parse_single_filesystem_entry(&output.stdout)
    }
    async fn move_entry(
        &self,
        source: String,
        destination: String,
    ) -> Result<EnvdFilesystemEntry, BackendError> {
        self.adapter
            .ensure_filesystem_write_allowed(Some(&self.sandbox_id), "move")
            .await?;
        let output = self
            .adapter
            .run_filesystem_command_for(
                Some(&self.sandbox_id),
                "move",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "mkdir -p -- \"$(dirname -- \"$2\")\" && mv -- \"$1\" \"$2\" && stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- \"$2\""
                        .to_owned(), "firkin-move-entry".to_owned(), source.clone(),
                        destination.clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("move", &source, output));
        }
        parse_single_filesystem_entry(&output.stdout)
    }
    async fn remove_entry(&self, path: String) -> Result<(), BackendError> {
        self.adapter
            .ensure_filesystem_write_allowed(Some(&self.sandbox_id), "remove")
            .await?;
        let output = self
            .adapter
            .run_filesystem_command_for(
                Some(&self.sandbox_id),
                "remove",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "rm -rf -- \"$1\"".to_owned(),
                        "firkin-remove-entry".to_owned(),
                        path.clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("remove", &path, output));
        }
        Ok(())
    }
    async fn stat_entry(&self, path: String) -> Result<EnvdFilesystemEntry, BackendError> {
        let output = self
            .adapter
            .run_filesystem_command_for(
                Some(&self.sandbox_id),
                "stat",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "stat -c '%F\t%s\t%f\t%A\t%U\t%G\t%n\t%N' -- \"$1\"".to_owned(),
                        "firkin-stat-entry".to_owned(),
                        path.clone(),
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(BackendError::NotFound(path));
        }
        parse_single_filesystem_entry(&output.stdout)
    }
    async fn watch_dir(
        &self,
        path: String,
        recursive: bool,
    ) -> Result<Vec<EnvdFilesystemEvent>, BackendError> {
        let recursive = recursive.to_string();
        let output = self
            .adapter
            .run_filesystem_command_for(
                Some(&self.sandbox_id),
                "watch",
                EnvdProcessStartRequest {
                    cmd: "/bin/sh".to_owned(),
                    args: vec![
                        "-lc".to_owned(),
                        "test -e \"$1\"".to_owned(),
                        "firkin-watch-dir".to_owned(),
                        path.clone(),
                        recursive,
                    ],
                    ..EnvdProcessStartRequest::default()
                },
            )
            .await?;
        if output.exit_code != 0 {
            return Err(filesystem_exit_error("watch", &path, output));
        }
        Ok(vec![EnvdFilesystemEvent {
            name: path,
            event_type: EnvdFilesystemEventType::Write,
        }])
    }
}
#[derive(Debug)]
struct FirkinHttpRequest {
    method: String,
    #[allow(missing_docs)]
    pub path: String,
    body: Vec<u8>,
}
#[derive(Debug, serde::Deserialize)]
struct CodeInterpreterExecuteRequest {
    code: String,
    #[serde(default)]
    context_id: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    env_vars: BTreeMap<String, String>,
}
async fn serve_firkin_code_interpreter<A>(listener: TcpListener, sandbox_id: String, adapter: A)
where
    A: EnvdProcessAdapter<Error = BackendError> + Clone + Send + Sync + 'static,
{
    loop {
        let Ok((mut stream, _addr)) = listener.accept().await else {
            break;
        };
        let sandbox_id = sandbox_id.clone();
        let adapter = adapter.clone();
        tokio::spawn(async move {
            let response = match read_firkin_http_request(&mut stream).await {
                Ok(Some(request)) => {
                    handle_code_interpreter_request(&sandbox_id, &adapter, request).await
                }
                Ok(None) => return,
                Err(error) => json_http_response(
                    "400 Bad Request",
                    &serde_json::json!(
                        { "error" : "bad_request", "message" : error, "service" :
                        "code-interpreter", "sandboxID" : sandbox_id, }
                    ),
                ),
            };
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}
async fn handle_code_interpreter_request<A>(
    sandbox_id: &str,
    adapter: &A,
    request: FirkinHttpRequest,
) -> String
where
    A: EnvdProcessAdapter<Error = BackendError> + Clone + Send + Sync + 'static,
{
    if request.method == "GET" && (request.path == "/" || request.path == "/health") {
        return json_http_response(
            "200 OK",
            &serde_json::json!(
                { "status" : "ok", "service" : "code-interpreter", "sandboxID" :
                sandbox_id, }
            ),
        );
    }
    if request.method == "POST" && request.path == "/execute" {
        return execute_code_interpreter_request(adapter, request.body).await;
    }
    json_http_response(
        "404 Not Found",
        &serde_json::json!(
            { "error" : "not_found", "service" : "code-interpreter", "sandboxID" :
            sandbox_id, }
        ),
    )
}
async fn execute_code_interpreter_request<A>(adapter: &A, body: Vec<u8>) -> String
where
    A: EnvdProcessAdapter<Error = BackendError> + Clone + Send + Sync + 'static,
{
    let request = match serde_json::from_slice::<CodeInterpreterExecuteRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return json_http_response(
                "400 Bad Request",
                &serde_json::json!(
                    { "error" : "invalid_json", "message" : error.to_string(), }
                ),
            );
        }
    };
    if request.context_id.is_some() && request.language.is_some() {
        return json_http_response(
            "400 Bad Request",
            &serde_json::json!(
                { "error" : "unsupported_context_language_pair", "message" :
                "context_id and language cannot both be set", }
            ),
        );
    }
    let language = request
        .language
        .clone()
        .unwrap_or_else(|| "python".to_owned());
    let process_request = match code_interpreter_process_request(&language, request) {
        Ok(request) => request,
        Err(message) => {
            return json_http_response(
                "400 Bad Request",
                &serde_json::json!(
                    { "error" : "unsupported_language", "message" : message, }
                ),
            );
        }
    };
    let output = match adapter.start_process(process_request).await {
        Ok(output) => output,
        Err(error) => {
            return code_interpreter_http_response(vec![
                serde_json::json!({ "type" : "error", "name" : "RuntimeError",
                    "value" : error.to_string(), "traceback" : [], "timestamp" :
                    code_interpreter_timestamp_nanos(), }),
            ]);
        }
    };
    let mut events = Vec::new();
    if !output.stdout.is_empty() {
        events.push(serde_json::json!(
            { "type" : "stdout", "text" : String::from_utf8_lossy(& output
            .stdout), "timestamp" : code_interpreter_timestamp_nanos(), }
        ));
    }
    if !output.stderr.is_empty() {
        events.push(serde_json::json!(
            { "type" : "stderr", "text" : String::from_utf8_lossy(& output
            .stderr), "timestamp" : code_interpreter_timestamp_nanos(), }
        ));
    }
    if output.exit_code != 0 || output.error.is_some() {
        events.push(serde_json::json!(
            { "type" : "error", "name" : "ProcessError", "value" : output.error
            .unwrap_or_else(|| format!("process exited with status {}", output
            .exit_code)), "traceback" : [String::from_utf8_lossy(& output.stderr)
            .to_string()], "timestamp" : code_interpreter_timestamp_nanos(), }
        ));
    }
    events.push(serde_json::json!(
        { "type" : "number_of_executions", "execution_count" : 1, }
    ));
    code_interpreter_http_response(events)
}
fn code_interpreter_process_request(
    language: &str,
    request: CodeInterpreterExecuteRequest,
) -> Result<EnvdProcessStartRequest, String> {
    let CodeInterpreterExecuteRequest {
        code,
        context_id,
        env_vars,
        ..
    } = request;
    let mut envs = env_vars;
    let (cmd, args) = match (language, context_id) {
        ("bash" | "sh", None) => ("/bin/sh".to_owned(), vec!["-lc".to_owned(), code]),
        ("python" | "python3", None) => ("python3".to_owned(), vec!["-c".to_owned(), code]),
        ("python" | "python3", Some(context_id)) => {
            envs.insert(
                CODE_INTERPRETER_CONTEXT_CODE_ENV.to_owned(),
                base64::engine::general_purpose::STANDARD.encode(code),
            );
            envs.insert(
                CODE_INTERPRETER_CONTEXT_PATH_ENV.to_owned(),
                code_interpreter_context_path(&context_id),
            );
            (
                "python3".to_owned(),
                vec!["-c".to_owned(), CODE_INTERPRETER_CONTEXT_RUNNER.to_owned()],
            )
        }
        ("bash" | "sh", Some(_)) => {
            return Err("code interpreter contexts are only supported for python".to_owned());
        }
        (other, _) => {
            return Err(format!("unsupported code interpreter language: {other}"));
        }
    };
    Ok(EnvdProcessStartRequest {
        cmd,
        args,
        envs,
        ..EnvdProcessStartRequest::default()
    })
}
fn code_interpreter_context_path(context_id: &str) -> String {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(context_id.as_bytes());
    format!(".firkin/code-interpreter-contexts/{encoded}.pickle")
}
fn code_interpreter_http_response(events: Vec<serde_json::Value>) -> String {
    let mut body = String::new();
    for event in events {
        body.push_str(&event.to_string());
        body.push('\n');
    }
    http_response("200 OK", "application/x-ndjson", &body)
}
fn json_http_response(status: &str, body: &serde_json::Value) -> String {
    let body = body.to_string();
    http_response(status, "application/json", &body)
}
fn http_response(status: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}
async fn read_firkin_http_request(
    stream: &mut TcpStream,
) -> Result<Option<FirkinHttpRequest>, String> {
    let mut bytes = Vec::new();
    let mut buf = [0_u8; 4096];
    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|error| format!("read request: {error}"))?;
        if n == 0 {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err("connection closed before request completed".to_owned())
            };
        }
        bytes.extend_from_slice(&buf[..n]);
        let Some(header_end) = find_http_header_end(&bytes) else {
            if bytes.len() > 1_048_576 {
                return Err("request headers exceed 1 MiB".to_owned());
            }
            continue;
        };
        let headers = std::str::from_utf8(&bytes[..header_end])
            .map_err(|error| format!("request headers are not utf8: {error}"))?;
        let content_length = http_content_length(headers)?;
        if bytes.len() < header_end + 4 + content_length {
            if bytes.len() > 8_388_608 {
                return Err("request body exceeds 8 MiB".to_owned());
            }
            continue;
        }
        let request_line = headers
            .lines()
            .next()
            .ok_or_else(|| "missing request line".to_owned())?;
        let mut parts = request_line.split_whitespace();
        let method = parts
            .next()
            .ok_or_else(|| "missing request method".to_owned())?
            .to_owned();
        let path = parts
            .next()
            .ok_or_else(|| "missing request path".to_owned())?
            .to_owned();
        let body_start = header_end + 4;
        let body = bytes[body_start..body_start + content_length].to_vec();
        return Ok(Some(FirkinHttpRequest { method, path, body }));
    }
}
fn find_http_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}
fn http_content_length(headers: &str) -> Result<usize, String> {
    for line in headers.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .map_err(|error| format!("invalid content-length: {error}"));
        }
    }
    Ok(0)
}
fn code_interpreter_timestamp_nanos() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX)
        })
}
