//! Raw sandbox event trace primitives.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error as ThisError;

/// Default raw event cap for one sandbox event trace.
pub const DEFAULT_EVENT_TRACE_CAP: usize = 1024;

/// Canonical sandbox event name.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum SandboxEventName {
    /// External API request accepted.
    RequestStart,
    /// Agent-computer product request accepted.
    AgentComputerRequestStart,
    /// Agent-computer sandbox create/followup call returned.
    AgentComputerSandboxCreated,
    /// Agent-computer product readiness probes started.
    AgentComputerProbeStart,
    /// Template lookup completed.
    TemplateLookupDone,
    /// Pool lease request starts.
    PoolLeaseRequested,
    /// Pool lease acquired.
    PoolLeaseAcquired,
    /// Snapshot restore starts.
    SnapshotRestoreStart,
    /// Snapshot restore completed.
    SnapshotRestoreDone,
    /// Virtualization start call issued.
    VzStartCalled,
    /// Guest agent connected to the host.
    GuestAgentConnected,
    /// Local ready flag checked.
    LocalReadyFlagChecked,
    /// Guest agent ping passed.
    GuestAgentPingPassed,
    /// Guest network is ready.
    NetworkReady,
    /// DNS probe passed.
    DnsReady,
    /// Workspace is ready.
    WorkspaceReady,
    /// Cgroup state is ready.
    CgroupsReady,
    /// Real readiness probe passed.
    ReadyProbePassed,
    /// Exec request sent.
    ExecRequestSent,
    /// Guest process started.
    ProcessStarted,
    /// First stdout byte observed by the host.
    FirstStdoutByte,
    /// CLI first useful stdout observed for product readiness.
    CliFirstUsefulStdout,
    /// Browser control endpoint is ready.
    BrowserReady,
    /// Database healthcheck is ready.
    DatabaseReady,
    /// Browser, database, CLI, workspace, network, and limits are ready.
    AgentComputerReady,
    /// Agent computer was suspended.
    AgentComputerSuspended,
    /// Agent computer was resumed.
    AgentComputerResumed,
    /// Autoscale pressure was detected.
    PressureDetected,
    /// Configured reserve floors are satisfied after pressure.
    SafeFloorRestored,
    /// Ready queue target is restored.
    ReadyTargetRestored,
    /// Autoscale controller made a decision.
    AutoscaleDecisionMade,
    /// Autoscale actuator started an action.
    AutoscaleActionStarted,
    /// Autoscale actuator completed an action.
    AutoscaleActionDone,
    /// Process exited.
    ProcessExited,
    /// Cleanup starts.
    CleanupStart,
    /// Fstrim starts.
    FstrimStart,
    /// Fstrim completed.
    FstrimDone,
    /// Cleanup completed.
    CleanupDone,
}

/// Lifecycle class attached to one sandbox trace event.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum LifecycleClass {
    /// Image/template/rootfs are not ready.
    ColdUnprepared,
    /// Template/rootfs are ready but no VM/pool is ready.
    ColdPrepared,
    /// Warm path that is not already leased as a ready sandbox.
    Warm,
    /// Clean sandbox already pooled.
    Hot,
    /// Snapshot or paused state restored.
    Resumed,
}

/// Workload class attached to one sandbox trace event.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum WorkloadClass {
    /// Tiny command that writes a known stdout payload.
    TinyExec,
    /// Direct argv tiny command without shell startup.
    DirectExec,
    /// Shell command startup workload.
    ShellExec,
    /// One hundred tiny commands in one ready sandbox.
    Batch100Execs,
    /// Small workspace import workload.
    WorkspaceImportSmall,
    /// Repository git status workload.
    RepoGitStatus,
    /// Small cargo build workload.
    CargoBuildSmall,
    /// Small npm install workload.
    NpmInstallSmall,
    /// Disk bloat and reclaim workload.
    DiskBloatReclaim,
    /// Concurrent sandbox create workload.
    ConcurrentCreate,
    /// Readiness probe workload.
    ReadinessProbe,
    /// Full browser + database + CLI agent-computer workload.
    AgentComputer,
    /// Autoscale scenario workload.
    AutoscaleScenario,
}

/// Runtime profile attached to one sandbox trace event.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum RuntimeProfile {
    /// Fast local agent profile.
    FastAgent,
    /// Disk reclaim profile.
    DiskReclaim,
    /// Density sweep profile.
    Density,
    /// Full browser + database + CLI product profile.
    BrowserDbCli,
}

/// Event outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum TraceOutcome {
    /// Event completed successfully.
    Success,
    /// Event failed.
    Error,
    /// Event is blocked by an unavailable live capability.
    Blocked,
}

impl TraceOutcome {
    const fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }
}

/// Stable sandbox failure class.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize)]
pub enum SandboxFailureClass {
    /// Boot failed.
    BootFailure,
    /// Guest agent failed generically.
    AgentFailure,
    /// Guest agent crashed.
    GuestAgentCrash,
    /// Workspace probe failed.
    WorkspaceMissing,
    /// Exec probe failed.
    ExecProbeFailed,
    /// DNS probe failed.
    DnsProbeFailed,
    /// Cgroup OOM kill.
    OomKill,
    /// Browser, database, or CLI sidecar failed.
    SidecarFailure,
    /// Admission rejected capacity.
    CapacityRejected,
    /// Pressure policy refused or delayed work.
    PressureRefusal,
    /// Operation timed out.
    Timeout,
    /// Failure could not be classified.
    Unknown,
}

/// Whether an event can be used by headline metric derivation.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, Serialize,
)]
pub enum SandboxEventRole {
    /// First successful event for this endpoint.
    #[default]
    Headline,
    /// Duplicate successful event retained as debug data only.
    DebugDuplicate,
}

/// One raw sandbox trace event.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SandboxTraceEvent {
    name: SandboxEventName,
    host_monotonic_ns: u128,
    lifecycle: LifecycleClass,
    workload: WorkloadClass,
    profile: RuntimeProfile,
    outcome: TraceOutcome,
    failure_class: Option<SandboxFailureClass>,
    #[serde(default)]
    role: SandboxEventRole,
}

impl SandboxTraceEvent {
    /// Construct a successful event.
    #[must_use]
    pub const fn new(
        name: SandboxEventName,
        host_monotonic_ns: u128,
        lifecycle: LifecycleClass,
        workload: WorkloadClass,
        profile: RuntimeProfile,
    ) -> Self {
        Self {
            name,
            host_monotonic_ns,
            lifecycle,
            workload,
            profile,
            outcome: TraceOutcome::Success,
            failure_class: None,
            role: SandboxEventRole::Headline,
        }
    }

    /// Construct an event with explicit outcome and failure class.
    #[must_use]
    pub const fn with_outcome(
        name: SandboxEventName,
        host_monotonic_ns: u128,
        lifecycle: LifecycleClass,
        workload: WorkloadClass,
        profile: RuntimeProfile,
        outcome: TraceOutcome,
        failure_class: Option<SandboxFailureClass>,
    ) -> Self {
        Self {
            name,
            host_monotonic_ns,
            lifecycle,
            workload,
            profile,
            outcome,
            failure_class,
            role: SandboxEventRole::Headline,
        }
    }

    /// Return event name.
    #[must_use]
    pub const fn name(&self) -> SandboxEventName {
        self.name
    }

    /// Return host monotonic offset in nanoseconds from trace origin.
    #[must_use]
    pub const fn host_monotonic_ns(&self) -> u128 {
        self.host_monotonic_ns
    }

    /// Return lifecycle class.
    #[must_use]
    pub const fn lifecycle(&self) -> LifecycleClass {
        self.lifecycle
    }

    /// Return workload class.
    #[must_use]
    pub const fn workload(&self) -> WorkloadClass {
        self.workload
    }

    /// Return runtime profile.
    #[must_use]
    pub const fn profile(&self) -> RuntimeProfile {
        self.profile
    }

    /// Return event outcome.
    #[must_use]
    pub const fn outcome(&self) -> TraceOutcome {
        self.outcome
    }

    /// Return optional failure class.
    #[must_use]
    pub const fn failure_class(&self) -> Option<SandboxFailureClass> {
        self.failure_class
    }

    /// Return event role.
    #[must_use]
    pub const fn role(&self) -> SandboxEventRole {
        self.role
    }

    fn mark_debug_duplicate(&mut self) {
        self.role = SandboxEventRole::DebugDuplicate;
    }
}

/// One raw event timeline for a sandbox operation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct SandboxEventTrace {
    #[serde(default)]
    events: Vec<SandboxTraceEvent>,
    #[serde(default)]
    overflowed: u64,
    #[serde(skip, default = "default_event_trace_cap")]
    event_cap: usize,
}

impl SandboxEventTrace {
    /// Construct an empty trace.
    #[must_use]
    pub fn new() -> Self {
        Self::with_event_cap(DEFAULT_EVENT_TRACE_CAP)
    }

    /// Construct an empty trace with an explicit event cap.
    #[must_use]
    pub fn with_event_cap(event_cap: usize) -> Self {
        Self {
            events: Vec::new(),
            overflowed: 0,
            event_cap,
        }
    }

    /// Push one event, classifying duplicate successful endpoints as debug data.
    pub fn push(&mut self, mut event: SandboxTraceEvent) {
        if self.events.len() >= self.event_cap {
            self.overflowed = self.overflowed.saturating_add(1);
            return;
        }
        if event.outcome.is_success()
            && self.events.iter().any(|existing| {
                existing.name == event.name
                    && existing.outcome.is_success()
                    && existing.role == SandboxEventRole::Headline
            })
        {
            event.mark_debug_duplicate();
        }
        self.events.push(event);
    }

    /// Return retained events.
    #[must_use]
    pub fn events(&self) -> &[SandboxTraceEvent] {
        &self.events
    }

    /// Return dropped event count.
    #[must_use]
    pub const fn overflowed(&self) -> u64 {
        self.overflowed
    }

    /// Return the first successful headline event for a name.
    #[must_use]
    pub fn headline_event(&self, name: SandboxEventName) -> Option<&SandboxTraceEvent> {
        self.events.iter().find(|event| {
            event.name == name
                && event.outcome.is_success()
                && event.role == SandboxEventRole::Headline
        })
    }

    /// Return host-observed duration between two headline events.
    ///
    /// # Errors
    ///
    /// Returns [`SandboxEventTraceError`] when either endpoint is missing, the
    /// end precedes the start, or the duration exceeds [`Duration`] limits.
    pub fn duration_between(
        &self,
        start: SandboxEventName,
        end: SandboxEventName,
    ) -> Result<Duration, SandboxEventTraceError> {
        let start_event = self
            .headline_event(start)
            .ok_or(SandboxEventTraceError::MissingEvent { name: start })?;
        let end_event = self
            .headline_event(end)
            .ok_or(SandboxEventTraceError::MissingEvent { name: end })?;
        let delta = end_event
            .host_monotonic_ns
            .checked_sub(start_event.host_monotonic_ns)
            .ok_or(SandboxEventTraceError::EventOrder { start, end })?;
        let nanos = u64::try_from(delta).map_err(|_| SandboxEventTraceError::DurationOverflow {
            start,
            end,
            nanos: delta,
        })?;
        Ok(Duration::from_nanos(nanos))
    }
}

impl Default for SandboxEventTrace {
    fn default() -> Self {
        Self::new()
    }
}

/// Event trace recorder using one host monotonic origin.
#[derive(Clone, Debug)]
pub struct EventTraceRecorder {
    origin: Instant,
    trace: SandboxEventTrace,
    lifecycle: LifecycleClass,
    workload: WorkloadClass,
    profile: RuntimeProfile,
}

impl EventTraceRecorder {
    /// Construct a recorder with the default event cap.
    #[must_use]
    pub fn new(
        lifecycle: LifecycleClass,
        workload: WorkloadClass,
        profile: RuntimeProfile,
    ) -> Self {
        Self::with_event_cap(lifecycle, workload, profile, DEFAULT_EVENT_TRACE_CAP)
    }

    /// Construct a recorder with an explicit event cap.
    #[must_use]
    pub fn with_event_cap(
        lifecycle: LifecycleClass,
        workload: WorkloadClass,
        profile: RuntimeProfile,
        event_cap: usize,
    ) -> Self {
        Self {
            origin: Instant::now(),
            trace: SandboxEventTrace::with_event_cap(event_cap),
            lifecycle,
            workload,
            profile,
        }
    }

    /// Record one successful event.
    pub fn record(&mut self, name: SandboxEventName) {
        self.record_with_outcome(name, TraceOutcome::Success, None);
    }

    /// Return elapsed host monotonic time since recorder creation.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.origin.elapsed()
    }

    /// Record one successful event at an explicit elapsed host monotonic time.
    pub fn record_at_elapsed(&mut self, name: SandboxEventName, elapsed: Duration) {
        self.record_with_outcome_at_elapsed(name, elapsed, TraceOutcome::Success, None);
    }

    /// Record one event with explicit outcome and failure class.
    pub fn record_with_outcome(
        &mut self,
        name: SandboxEventName,
        outcome: TraceOutcome,
        failure_class: Option<SandboxFailureClass>,
    ) {
        self.trace.push(SandboxTraceEvent::with_outcome(
            name,
            self.origin.elapsed().as_nanos(),
            self.lifecycle,
            self.workload,
            self.profile,
            outcome,
            failure_class,
        ));
    }

    /// Record one event with explicit outcome at an explicit elapsed host monotonic time.
    pub fn record_with_outcome_at_elapsed(
        &mut self,
        name: SandboxEventName,
        elapsed: Duration,
        outcome: TraceOutcome,
        failure_class: Option<SandboxFailureClass>,
    ) {
        self.trace.push(SandboxTraceEvent::with_outcome(
            name,
            elapsed.as_nanos(),
            self.lifecycle,
            self.workload,
            self.profile,
            outcome,
            failure_class,
        ));
    }

    /// Return lifecycle class used for future recorded events.
    #[must_use]
    pub const fn lifecycle(&self) -> LifecycleClass {
        self.lifecycle
    }

    /// Return workload class used for future recorded events.
    #[must_use]
    pub const fn workload(&self) -> WorkloadClass {
        self.workload
    }

    /// Return whether this recorder already contains events.
    #[must_use]
    pub fn has_recorded_events(&self) -> bool {
        !self.trace.events().is_empty()
    }

    /// Return a recorder whose future events use a different workload class.
    ///
    /// Events already recorded keep their original labels.
    #[must_use]
    pub fn with_future_workload(mut self, workload: WorkloadClass) -> Self {
        self.workload = workload;
        self
    }

    /// Return runtime profile used for future recorded events.
    #[must_use]
    pub const fn profile(&self) -> RuntimeProfile {
        self.profile
    }

    /// Finish and return the raw event trace.
    #[must_use]
    pub fn finish(self) -> SandboxEventTrace {
        self.trace
    }
}

/// Event trace lookup error.
#[derive(Clone, Debug, Eq, PartialEq, ThisError)]
pub enum SandboxEventTraceError {
    /// The requested endpoint was missing.
    #[error("missing sandbox event {name:?} in event trace")]
    MissingEvent {
        /// Missing event name.
        name: SandboxEventName,
    },
    /// The requested end event occurred before the start event.
    #[error("sandbox event {end:?} occurred before {start:?}")]
    EventOrder {
        /// Start event name.
        start: SandboxEventName,
        /// End event name.
        end: SandboxEventName,
    },
    /// Duration does not fit in [`Duration`].
    #[error("sandbox event duration from {start:?} to {end:?} overflowed: {nanos}ns")]
    DurationOverflow {
        /// Start event name.
        start: SandboxEventName,
        /// End event name.
        end: SandboxEventName,
        /// Nanoseconds that did not fit.
        nanos: u128,
    },
}

const fn default_event_trace_cap() -> usize {
    DEFAULT_EVENT_TRACE_CAP
}
