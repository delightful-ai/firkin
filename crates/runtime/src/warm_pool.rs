//! warm pool — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::DEFAULT_RUNTIME_WARM_POOL_MINIMUM_FREE_DISK;
#[allow(unused_imports)]
use crate::disk::{DiskPressureProbe, HostDiskPressureProbe, RuntimeDiskPressureGuard};
#[allow(unused_imports)]
use crate::restore::{
    ActiveSessionReservation, SnapshotRestoreRequest, SnapshotSessionLauncher,
    disk_pressure_to_capacity_error,
};
#[allow(unused_imports)]
use firkin_admission::CapacityError;
#[allow(unused_imports)]
use firkin_admission::WarmPoolEntry;
#[allow(unused_imports)]
use firkin_admission::WarmPoolReplenishmentSkip;
#[allow(unused_imports)]
use firkin_admission::{CapacityLedger, WarmPoolLedger};
#[allow(unused_imports)]
use firkin_admission::{WarmPoolReplenishmentPlan, WarmPoolReplenishmentTarget};
#[allow(unused_imports)]
use firkin_trace::BenchmarkSample;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use std::collections::VecDeque;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
#[allow(unused_imports)]
use tokio::sync::{Mutex, oneshot};
#[allow(unused_imports)]
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use {
    firkin_admission::{ResourceBudget, WarmPoolKey},
    firkin_artifacts::SnapshotArtifactManifest,
};
/// Request passed to the runtime warm-pool launcher.
#[derive(Clone, Copy, Debug)]
pub struct WarmPoolRestoreRequest<'a> {
    key: &'a WarmPoolKey,
    pub(crate) manifest: &'a SnapshotArtifactManifest,
    pub(crate) budget: ResourceBudget,
}
impl<'a> WarmPoolRestoreRequest<'a> {
    /// Construct a warm-pool restore request.
    #[must_use]
    pub const fn new(
        key: &'a WarmPoolKey,
        manifest: &'a SnapshotArtifactManifest,
        budget: ResourceBudget,
    ) -> Self {
        Self {
            key,
            manifest,
            budget,
        }
    }
    /// Return the warm-pool key.
    #[must_use]
    pub const fn key(self) -> &'a WarmPoolKey {
        self.key
    }
    /// Return the snapshot manifest used as the restore source.
    #[must_use]
    pub const fn manifest(self) -> &'a SnapshotArtifactManifest {
        self.manifest
    }
    /// Return resources reserved for the warm-pool entry.
    #[must_use]
    pub const fn budget(self) -> ResourceBudget {
        self.budget
    }
}
/// Runtime-owned launcher for a pre-restored warm-pool entry.
pub trait WarmPoolSessionLauncher {
    /// Error returned by the launcher implementation.
    type Error;
    /// Restored warm entry session handle.
    type Session;
    /// Restore a warm-pool entry from the snapshot described by `request`.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific restore errors.
    fn restore_warm_pool_entry(
        &mut self,
        request: &WarmPoolRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error>;
}
/// Request passed to the runtime warm-pool checkout adapter.
#[derive(Clone, Copy, Debug)]
pub struct WarmPoolCheckoutRequest<'a> {
    entry: &'a WarmPoolEntry,
}
impl<'a> WarmPoolCheckoutRequest<'a> {
    /// Construct a warm-pool checkout request.
    #[must_use]
    pub const fn new(entry: &'a WarmPoolEntry) -> Self {
        Self { entry }
    }
    /// Return the warm-pool entry being checked out.
    #[must_use]
    pub const fn entry(self) -> &'a WarmPoolEntry {
        self.entry
    }
}
/// Runtime-owned adapter for checking out an already-restored warm-pool entry.
pub trait WarmPoolSessionCheckout {
    /// Error returned by the checkout implementation.
    type Error;
    /// Active session handle.
    type Session;
    /// Convert a warm-pool entry into an active session.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific checkout errors.
    fn checkout_warm_pool_entry(
        &mut self,
        request: &WarmPoolCheckoutRequest<'_>,
    ) -> Result<Self::Session, Self::Error>;
}
/// Runtime warm-pool maintenance error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum WarmPoolMaintenanceError<E> {
    /// Capacity admission failed after warm-pool restore.
    #[error("warm-pool capacity admission failed: {0}")]
    Capacity(#[from] CapacityError),
    /// Runtime launcher failed before warm-pool entry was recorded.
    #[error("warm-pool launcher failed: {source}")]
    Launch {
        /// Source launcher error.
        source: E,
    },
}
/// Runtime warm-pool checkout error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum WarmPoolCheckoutError<E> {
    /// Capacity promotion failed.
    #[error("warm-pool checkout capacity promotion failed: {0}")]
    Capacity(#[from] CapacityError),
    /// Runtime checkout failed after capacity promotion.
    #[error("warm-pool checkout failed: {source}")]
    Checkout {
        /// Source checkout error.
        source: E,
    },
}
/// Report from maintaining a runtime warm-pool entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarmPoolMaintenanceReport<S> {
    pub(crate) session: S,
    entry: WarmPoolEntry,
}
impl<S> WarmPoolMaintenanceReport<S> {
    /// Construct a warm-pool maintenance report.
    #[must_use]
    pub const fn new(session: S, entry: WarmPoolEntry) -> Self {
        Self { session, entry }
    }
    /// Return the restored warm entry session handle.
    #[must_use]
    pub const fn session(&self) -> &S {
        &self.session
    }
    /// Return the recorded warm-pool entry.
    #[must_use]
    pub const fn entry(&self) -> &WarmPoolEntry {
        &self.entry
    }
    /// Consume the report and return its session and entry.
    #[must_use]
    pub fn into_parts(self) -> (S, WarmPoolEntry) {
        (self.session, self.entry)
    }
}
/// Report from checking out a runtime warm-pool entry.
#[derive(Clone, Debug, PartialEq)]
pub struct WarmPoolCheckoutReport<S> {
    pub(crate) session: S,
    entry: WarmPoolEntry,
    pub(crate) reservation: ActiveSessionReservation,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
impl<S> WarmPoolCheckoutReport<S> {
    /// Construct a warm-pool checkout report.
    #[must_use]
    pub fn new(
        session: S,
        entry: WarmPoolEntry,
        reservation: ActiveSessionReservation,
        benchmark_samples: Vec<BenchmarkSample>,
    ) -> Self {
        Self {
            session,
            entry,
            reservation,
            benchmark_samples,
        }
    }
    /// Return the active session handle.
    #[must_use]
    pub const fn session(&self) -> &S {
        &self.session
    }
    /// Return the checked-out warm-pool entry.
    #[must_use]
    pub const fn entry(&self) -> &WarmPoolEntry {
        &self.entry
    }
    /// Return the active capacity reservation.
    #[must_use]
    pub const fn reservation(&self) -> &ActiveSessionReservation {
        &self.reservation
    }
    /// Return benchmark samples recorded during checkout.
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
/// Result shape for a warm-pool checkout attempt.
pub type WarmPoolCheckoutResult<S, E> =
    Option<Result<WarmPoolCheckoutReport<S>, WarmPoolCheckoutError<E>>>;
/// Runtime-level warm-pool maintenance orchestrator.
#[derive(Debug)]
pub struct RuntimeWarmPoolMaintain<'a> {
    capacity: &'a mut CapacityLedger,
    pool: &'a mut WarmPoolLedger,
    key: WarmPoolKey,
    pub(crate) manifest: &'a SnapshotArtifactManifest,
    pub(crate) budget: ResourceBudget,
}
impl<'a> RuntimeWarmPoolMaintain<'a> {
    /// Construct a runtime-level warm-pool maintenance operation.
    pub fn new(
        capacity: &'a mut CapacityLedger,
        pool: &'a mut WarmPoolLedger,
        key: WarmPoolKey,
        manifest: &'a SnapshotArtifactManifest,
        budget: ResourceBudget,
    ) -> Self {
        Self {
            capacity,
            pool,
            key,
            manifest,
            budget,
        }
    }
    /// Restore and record one warm-pool entry.
    ///
    /// # Errors
    ///
    /// Returns [`WarmPoolMaintenanceError`] when launch or warm capacity
    /// admission fails.
    pub fn execute<L>(
        self,
        launcher: &mut L,
    ) -> Result<WarmPoolMaintenanceReport<L::Session>, WarmPoolMaintenanceError<L::Error>>
    where
        L: WarmPoolSessionLauncher,
    {
        let request = WarmPoolRestoreRequest::new(&self.key, self.manifest, self.budget);
        let session = launcher
            .restore_warm_pool_entry(&request)
            .map_err(|source| WarmPoolMaintenanceError::Launch { source })?;
        let entry =
            WarmPoolEntry::new(self.key, self.manifest.logical_id().to_owned(), self.budget);
        self.pool.maintain(entry.clone(), self.capacity)?;
        Ok(WarmPoolMaintenanceReport::new(session, entry))
    }
}
/// Runtime-level warm-pool checkout orchestrator.
#[derive(Debug)]
pub struct RuntimeWarmPoolCheckout<'a> {
    capacity: &'a mut CapacityLedger,
    pool: &'a mut WarmPoolLedger,
    key: &'a WarmPoolKey,
}
impl<'a> RuntimeWarmPoolCheckout<'a> {
    /// Construct a runtime-level warm-pool checkout operation.
    pub fn new(
        capacity: &'a mut CapacityLedger,
        pool: &'a mut WarmPoolLedger,
        key: &'a WarmPoolKey,
    ) -> Self {
        Self {
            capacity,
            pool,
            key,
        }
    }
    /// Checkout a warm-pool entry and record checkout latency.
    ///
    /// # Errors
    ///
    /// Returns [`WarmPoolCheckoutError`] when capacity promotion or runtime
    /// checkout fails.
    pub fn execute_with_elapsed<C>(
        self,
        checkout: &mut C,
        elapsed: Duration,
    ) -> WarmPoolCheckoutResult<C::Session, C::Error>
    where
        C: WarmPoolSessionCheckout,
    {
        let entry = match self.pool.checkout(self.key, self.capacity) {
            Ok(Some(entry)) => entry,
            Ok(None) => return None,
            Err(error) => return Some(Err(WarmPoolCheckoutError::Capacity(error))),
        };
        let request = WarmPoolCheckoutRequest::new(&entry);
        let session = match checkout.checkout_warm_pool_entry(&request) {
            Ok(session) => session,
            Err(source) => {
                self.capacity.release_active(entry.budget());
                let _ = self.pool.maintain(entry.clone(), self.capacity);
                return Some(Err(WarmPoolCheckoutError::Checkout { source }));
            }
        };
        let reservation = ActiveSessionReservation::new(entry.budget());
        let sample = BenchmarkSample::new(
            "warm_pool_checkout",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            elapsed.as_secs_f64() * 1000.0,
        );
        Some(Ok(WarmPoolCheckoutReport::new(
            session,
            entry,
            reservation,
            vec![sample],
        )))
    }
}
/// Error returned when checking out a retained snapshot warm-pool session.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum SnapshotWarmPoolCheckoutError {
    /// Capacity promotion failed.
    #[error("warm-pool checkout capacity promotion failed: {0}")]
    Capacity(#[from] CapacityError),
    /// Warm-pool metadata existed without a retained restored session.
    #[error("warm-pool metadata existed without a retained restored session")]
    MissingRetainedSession,
}
/// Runtime-owned warm pool for already-restored snapshot sessions.
#[derive(Clone, Debug)]
pub struct RuntimeSnapshotWarmPool<S> {
    capacity: CapacityLedger,
    pub(crate) ledger: WarmPoolLedger,
    sessions: BTreeMap<WarmPoolKey, VecDeque<WarmPoolMaintenanceReport<S>>>,
}
impl<S> RuntimeSnapshotWarmPool<S> {
    /// Construct an empty snapshot warm pool with the given host capacity.
    #[must_use]
    pub fn new(capacity: CapacityLedger) -> Self {
        Self {
            capacity,
            ledger: WarmPoolLedger::default(),
            sessions: BTreeMap::new(),
        }
    }
    /// Return the capacity ledger for active and warm-pool reservations.
    #[must_use]
    pub const fn capacity(&self) -> CapacityLedger {
        self.capacity
    }
    /// Return whether a retained warm entry exists for `key`.
    #[must_use]
    pub fn contains(&self, key: &WarmPoolKey) -> bool {
        self.sessions
            .get(key)
            .is_some_and(|entries| !entries.is_empty())
    }
    /// Restore one snapshot into the warm pool and retain its live session.
    ///
    /// # Errors
    ///
    /// Returns [`WarmPoolMaintenanceError`] when disk pressure, capacity
    /// admission, or snapshot restore fails.
    pub async fn maintain_with_elapsed<L>(
        &mut self,
        key: WarmPoolKey,
        manifest: &SnapshotArtifactManifest,
        budget: ResourceBudget,
        launcher: &mut L,
        elapsed: Duration,
    ) -> Result<&WarmPoolMaintenanceReport<S>, WarmPoolMaintenanceError<L::Error>>
    where
        L: SnapshotSessionLauncher<Session = S>,
    {
        let mut probe = HostDiskPressureProbe::new();
        self.maintain_with_disk_probe_elapsed(key, manifest, budget, launcher, elapsed, &mut probe)
            .await
    }
    /// Restore one snapshot into the warm pool with a caller-provided disk probe.
    ///
    /// # Errors
    ///
    /// Returns [`WarmPoolMaintenanceError`] when disk pressure, capacity
    /// admission, or snapshot restore fails.
    ///
    /// # Panics
    ///
    /// Panics only if the just-inserted retained session cannot be read back
    /// from the in-memory warm-pool map.
    pub async fn maintain_with_disk_probe_elapsed<L, P>(
        &mut self,
        key: WarmPoolKey,
        manifest: &SnapshotArtifactManifest,
        budget: ResourceBudget,
        launcher: &mut L,
        _elapsed: Duration,
        disk_probe: &mut P,
    ) -> Result<&WarmPoolMaintenanceReport<S>, WarmPoolMaintenanceError<L::Error>>
    where
        L: SnapshotSessionLauncher<Session = S>,
        P: DiskPressureProbe,
    {
        let disk_root = manifest.path().parent().unwrap_or(Path::new("/"));
        RuntimeDiskPressureGuard::new(disk_root, DEFAULT_RUNTIME_WARM_POOL_MINIMUM_FREE_DISK)
            .check(disk_probe)
            .map_err(|error| {
                WarmPoolMaintenanceError::Capacity(disk_pressure_to_capacity_error(&error))
            })?;
        let request = SnapshotRestoreRequest::new(manifest, budget);
        let session = launcher
            .restore_from_snapshot(&request)
            .await
            .map_err(|source| WarmPoolMaintenanceError::Launch { source })?;
        let entry = WarmPoolEntry::new(key.clone(), manifest.logical_id().to_owned(), budget);
        if let Err(error) = self.ledger.maintain(entry.clone(), &mut self.capacity) {
            return Err(WarmPoolMaintenanceError::Capacity(error));
        }
        self.sessions
            .entry(key.clone())
            .or_default()
            .push_back(WarmPoolMaintenanceReport::new(session, entry));
        Ok(self
            .sessions
            .get(&key)
            .and_then(|entries| entries.back())
            .expect("warm-pool session was just inserted"))
    }
    /// Promote one retained warm session to active use.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotWarmPoolCheckoutError`] when capacity promotion fails
    /// or the retained live session is missing.
    pub fn checkout_with_elapsed(
        &mut self,
        key: &WarmPoolKey,
        elapsed: Duration,
    ) -> Result<Option<WarmPoolCheckoutReport<S>>, SnapshotWarmPoolCheckoutError> {
        let Some(entry) = self.ledger.checkout(key, &mut self.capacity)? else {
            return Ok(None);
        };
        let Some(reports) = self.sessions.get_mut(key) else {
            self.capacity.release_active(entry.budget());
            return Err(SnapshotWarmPoolCheckoutError::MissingRetainedSession);
        };
        let Some(report) = reports.pop_front() else {
            self.sessions.remove(key);
            self.capacity.release_active(entry.budget());
            return Err(SnapshotWarmPoolCheckoutError::MissingRetainedSession);
        };
        if reports.is_empty() {
            self.sessions.remove(key);
        }
        let (session, retained_entry) = report.into_parts();
        let reservation = ActiveSessionReservation::new(entry.budget());
        let sample = BenchmarkSample::new(
            "warm_pool_checkout",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            elapsed.as_secs_f64() * 1000.0,
        );
        Ok(Some(WarmPoolCheckoutReport::new(
            session,
            retained_entry,
            reservation,
            vec![sample],
        )))
    }
    /// Execute a replenishment pass for desired warm-pool targets.
    pub async fn replenish_with_elapsed<L>(
        &mut self,
        targets: &[WarmPoolReplenishmentTarget],
        launcher: &mut L,
        elapsed: Duration,
    ) -> RuntimeWarmPoolReplenishmentReport<L::Error>
    where
        L: SnapshotSessionLauncher<Session = S>,
    {
        let mut disk_probe = HostDiskPressureProbe::new();
        self.replenish_with_disk_probe_elapsed(targets, launcher, elapsed, &mut disk_probe)
            .await
    }
    /// Execute a replenishment pass with a caller-provided disk probe.
    pub async fn replenish_with_disk_probe_elapsed<L, P>(
        &mut self,
        targets: &[WarmPoolReplenishmentTarget],
        launcher: &mut L,
        elapsed: Duration,
        disk_probe: &mut P,
    ) -> RuntimeWarmPoolReplenishmentReport<L::Error>
    where
        L: SnapshotSessionLauncher<Session = S>,
        P: DiskPressureProbe,
    {
        let plan = WarmPoolReplenishmentPlan::from_targets(targets, &self.ledger, self.capacity);
        let mut maintained = Vec::new();
        let skipped = plan.skipped().to_vec();
        let mut failed = Vec::new();
        for target in plan.maintain() {
            match self
                .maintain_with_disk_probe_elapsed(
                    target.key().clone(),
                    target.manifest(),
                    target.budget(),
                    launcher,
                    elapsed,
                    disk_probe,
                )
                .await
            {
                Ok(report) => maintained.push(report.entry().key().clone()),
                Err(error) => failed.push(RuntimeWarmPoolReplenishmentFailure::new(
                    target.key().clone(),
                    error,
                )),
            }
        }
        RuntimeWarmPoolReplenishmentReport::new(maintained, skipped, failed)
    }
}
/// Warm-pool target that failed during runtime replenishment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeWarmPoolReplenishmentFailure<E> {
    key: WarmPoolKey,
    pub(crate) error: WarmPoolMaintenanceError<E>,
}
impl<E> RuntimeWarmPoolReplenishmentFailure<E> {
    /// Construct a runtime warm-pool replenishment failure.
    #[must_use]
    pub const fn new(key: WarmPoolKey, error: WarmPoolMaintenanceError<E>) -> Self {
        Self { key, error }
    }
    /// Return the failed warm-pool key.
    #[must_use]
    pub const fn key(&self) -> &WarmPoolKey {
        &self.key
    }
    /// Return the failure reason.
    #[must_use]
    pub const fn error(&self) -> &WarmPoolMaintenanceError<E> {
        &self.error
    }
}
/// Report from executing a warm-pool replenishment plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeWarmPoolReplenishmentReport<E> {
    pub(crate) maintained: Vec<WarmPoolKey>,
    skipped: Vec<WarmPoolReplenishmentSkip>,
    failed: Vec<RuntimeWarmPoolReplenishmentFailure<E>>,
}
impl<E> RuntimeWarmPoolReplenishmentReport<E> {
    /// Construct a runtime warm-pool replenishment report.
    #[must_use]
    pub const fn new(
        maintained: Vec<WarmPoolKey>,
        skipped: Vec<WarmPoolReplenishmentSkip>,
        failed: Vec<RuntimeWarmPoolReplenishmentFailure<E>>,
    ) -> Self {
        Self {
            maintained,
            skipped,
            failed,
        }
    }
    /// Return warm-pool keys that were restored.
    #[must_use]
    pub fn maintained(&self) -> &[WarmPoolKey] {
        &self.maintained
    }
    /// Return targets skipped before restore.
    #[must_use]
    pub fn skipped(&self) -> &[WarmPoolReplenishmentSkip] {
        &self.skipped
    }
    /// Return targets that failed during restore.
    #[must_use]
    pub fn failed(&self) -> &[RuntimeWarmPoolReplenishmentFailure<E>] {
        &self.failed
    }
}
/// Runtime warm-pool supervisor configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeWarmPoolSupervisor {
    pub(crate) targets: Vec<WarmPoolReplenishmentTarget>,
    pub(crate) interval: Duration,
}
impl RuntimeWarmPoolSupervisor {
    /// Construct a warm-pool supervisor for the desired target set.
    #[must_use]
    pub fn new(targets: Vec<WarmPoolReplenishmentTarget>, interval: Duration) -> Self {
        Self { targets, interval }
    }
    /// Return the desired warm-pool target set.
    #[must_use]
    pub fn targets(&self) -> &[WarmPoolReplenishmentTarget] {
        &self.targets
    }
    /// Return the delay between replenishment cycles.
    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }
    /// Run a bounded number of replenishment cycles.
    ///
    /// This is the testable core of the background supervisor loop; callers can
    /// run it repeatedly or spawn it inside a longer-lived service owner.
    pub async fn run_cycles<S, L>(
        &self,
        pool: &mut RuntimeSnapshotWarmPool<S>,
        launcher: &mut L,
        cycles: usize,
    ) -> Vec<RuntimeWarmPoolReplenishmentReport<L::Error>>
    where
        L: SnapshotSessionLauncher<Session = S>,
    {
        let mut disk_probe = HostDiskPressureProbe::new();
        self.run_cycles_with_disk_probe(pool, launcher, cycles, &mut disk_probe)
            .await
    }
    /// Run a bounded number of replenishment cycles with a caller-provided disk
    /// probe.
    pub async fn run_cycles_with_disk_probe<S, L, P>(
        &self,
        pool: &mut RuntimeSnapshotWarmPool<S>,
        launcher: &mut L,
        cycles: usize,
        disk_probe: &mut P,
    ) -> Vec<RuntimeWarmPoolReplenishmentReport<L::Error>>
    where
        L: SnapshotSessionLauncher<Session = S>,
        P: DiskPressureProbe,
    {
        let mut reports = Vec::with_capacity(cycles);
        for cycle in 0..cycles {
            if cycle > 0 && !self.interval.is_zero() {
                tokio::time::sleep(self.interval).await;
            }
            reports.push(
                pool.replenish_with_disk_probe_elapsed(
                    &self.targets,
                    launcher,
                    Duration::ZERO,
                    disk_probe,
                )
                .await,
            );
        }
        reports
    }
}
/// Long-lived runtime owner for warm-pool replenishment.
#[derive(Clone, Debug)]
pub struct RuntimeWarmPoolService<S, L> {
    pool: RuntimeSnapshotWarmPool<S>,
    supervisor: RuntimeWarmPoolSupervisor,
    pub(crate) launcher: L,
}
impl<S, L> RuntimeWarmPoolService<S, L> {
    /// Construct a warm-pool service from its retained pool, supervisor, and launcher.
    #[must_use]
    pub const fn new(
        pool: RuntimeSnapshotWarmPool<S>,
        supervisor: RuntimeWarmPoolSupervisor,
        launcher: L,
    ) -> Self {
        Self {
            pool,
            supervisor,
            launcher,
        }
    }
    /// Return current capacity accounting for active and warm sessions.
    #[must_use]
    pub const fn capacity(&self) -> CapacityLedger {
        self.pool.capacity()
    }
    /// Return the configured supervisor.
    #[must_use]
    pub const fn supervisor(&self) -> &RuntimeWarmPoolSupervisor {
        &self.supervisor
    }
    /// Return the owned launcher.
    #[must_use]
    pub const fn launcher(&self) -> &L {
        &self.launcher
    }
    /// Return whether a warm entry is currently retained for `key`.
    #[must_use]
    pub fn contains(&self, key: &WarmPoolKey) -> bool {
        self.pool.contains(key)
    }
    /// Run one replenishment cycle through the owned launcher.
    pub async fn tick(&mut self, elapsed: Duration) -> RuntimeWarmPoolReplenishmentReport<L::Error>
    where
        L: SnapshotSessionLauncher<Session = S>,
    {
        let mut disk_probe = HostDiskPressureProbe::new();
        self.tick_with_disk_probe(elapsed, &mut disk_probe).await
    }
    /// Run one replenishment cycle through the owned launcher with a
    /// caller-provided disk probe.
    pub async fn tick_with_disk_probe<P>(
        &mut self,
        elapsed: Duration,
        disk_probe: &mut P,
    ) -> RuntimeWarmPoolReplenishmentReport<L::Error>
    where
        L: SnapshotSessionLauncher<Session = S>,
        P: DiskPressureProbe,
    {
        self.pool
            .replenish_with_disk_probe_elapsed(
                self.supervisor.targets(),
                &mut self.launcher,
                elapsed,
                disk_probe,
            )
            .await
    }
    /// Promote a retained warm session to active use.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotWarmPoolCheckoutError`] when capacity promotion fails
    /// or retained session state is inconsistent.
    pub fn checkout_with_elapsed(
        &mut self,
        key: &WarmPoolKey,
        elapsed: Duration,
    ) -> Result<Option<WarmPoolCheckoutReport<S>>, SnapshotWarmPoolCheckoutError> {
        self.pool.checkout_with_elapsed(key, elapsed)
    }
    /// Spawn the warm-pool refill loop as an owned runtime task.
    #[must_use]
    pub fn spawn(self) -> RuntimeWarmPoolServiceHandle<S, L>
    where
        S: Send + 'static,
        L: SnapshotSessionLauncher<Session = S> + Send + 'static,
        L::Error: Send,
    {
        self.spawn_with_disk_probe(HostDiskPressureProbe::new())
    }
    /// Spawn the warm-pool refill loop with a caller-provided disk probe.
    #[must_use]
    pub fn spawn_with_disk_probe<P>(self, mut disk_probe: P) -> RuntimeWarmPoolServiceHandle<S, L>
    where
        S: Send + 'static,
        L: SnapshotSessionLauncher<Session = S> + Send + 'static,
        L::Error: Send,
        P: DiskPressureProbe + Send + 'static,
    {
        let interval = self.supervisor.interval();
        let state = Arc::new(Mutex::new(self));
        let task_state = Arc::clone(&state);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                {
                    let mut service = task_state.lock().await;
                    service
                        .tick_with_disk_probe(Duration::ZERO, &mut disk_probe)
                        .await;
                }
                tokio::select! {
                    _ = & mut shutdown_rx => break, () = tokio::time::sleep(interval) =>
                    {}
                }
            }
        });
        RuntimeWarmPoolServiceHandle {
            state,
            shutdown: Some(shutdown_tx),
            task,
        }
    }
}
/// Handle for a spawned warm-pool service task.
#[derive(Debug)]
pub struct RuntimeWarmPoolServiceHandle<S, L> {
    pub(crate) state: Arc<Mutex<RuntimeWarmPoolService<S, L>>>,
    pub(crate) shutdown: Option<oneshot::Sender<()>>,
    pub(crate) task: JoinHandle<()>,
}
impl<S, L> RuntimeWarmPoolServiceHandle<S, L> {
    /// Return current capacity accounting for active and warm sessions.
    pub async fn capacity(&self) -> CapacityLedger {
        self.state.lock().await.capacity()
    }
    /// Return whether a warm entry is currently retained for `key`.
    pub async fn contains(&self, key: &WarmPoolKey) -> bool {
        self.state.lock().await.contains(key)
    }
    /// Promote a retained warm session to active use while the service is running.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotWarmPoolCheckoutError`] when capacity promotion fails
    /// or retained session state is inconsistent.
    pub async fn checkout_with_elapsed(
        &self,
        key: &WarmPoolKey,
        elapsed: Duration,
    ) -> Result<Option<WarmPoolCheckoutReport<S>>, SnapshotWarmPoolCheckoutError> {
        self.state.lock().await.checkout_with_elapsed(key, elapsed)
    }
    /// Stop the background refill loop and return the owned service state.
    ///
    /// # Errors
    ///
    /// Returns [`tokio::task::JoinError`] if the service task panicked.
    ///
    /// # Panics
    ///
    /// Panics if another clone of the internal service state still exists after
    /// the background task has exited. The public handle does not expose a state
    /// clone, so this indicates an internal ownership bug.
    pub async fn shutdown(
        mut self,
    ) -> Result<RuntimeWarmPoolService<S, L>, tokio::task::JoinError> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await?;
        let mutex = Arc::try_unwrap(self.state).unwrap_or_else(|_| {
            panic!("warm-pool service handle owns the only remaining state reference")
        });
        Ok(mutex.into_inner())
    }
}
