//! hygiene — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::restore::{RuntimeCubeSandboxCreateError, SnapshotRestoreError};
#[allow(unused_imports)]
use crate::template_build::prepared_template_artifact_integrity;
#[allow(unused_imports)]
use firkin_artifacts::SnapshotArtifactIntegrity;
#[allow(unused_imports)]
use firkin_e2b_contract::BackendError;
#[allow(unused_imports)]
use firkin_e2b_contract::PreparedTemplateArtifactIntegrity;
#[allow(unused_imports)]
use firkin_envd::EnvdFilesystemEntry;
#[allow(unused_imports)]
use firkin_envd::EnvdFilesystemFileType;
#[allow(unused_imports)]
use firkin_envd::EnvdProcessOutput;
#[allow(unused_imports)]
use firkin_hygiene::HostRuntimeScan;
#[allow(unused_imports)]
use firkin_hygiene::RestartResourceKind;
#[allow(unused_imports)]
use firkin_hygiene::RestartStateRecord;
#[allow(unused_imports)]
use firkin_hygiene::{LogRotationError, LogRotationPlan, LogRotationReport};
#[allow(unused_imports)]
use firkin_hygiene::{ReconciliationDecision, ReconciliationPlan};
#[allow(unused_imports)]
use firkin_hygiene::{StuckVmCleanupDecision, StuckVmCleanupPlan};
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::path::Component;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::process::Command;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::UNIX_EPOCH;
#[allow(unused_imports)]
use std::time::{Duration, SystemTime};
#[allow(unused_imports)]
use thiserror::Error as ThisError;
#[allow(unused_imports)]
use time::format_description::well_known::Rfc3339;
#[allow(unused_imports)]
use time::{Duration as TimeDuration, OffsetDateTime};
#[allow(unused_imports)]
use tokio::sync::{Mutex, oneshot};
#[allow(unused_imports)]
use tokio::task::JoinHandle;
#[allow(unused_imports)]
use {
    firkin_artifacts::SnapshotArtifactManifest,
    firkin_hygiene::{ArtifactGcError, ArtifactGcPlan, ArtifactGcReport},
};
/// Runtime-owned adapter for applying restart reconciliation decisions.
pub trait ReconciliationExecutor {
    /// Error returned by the executor implementation.
    type Error;
    /// Recover a previously live resource.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific recovery errors.
    fn recover(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error>;
    /// Clean up a stale or disposable resource.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific cleanup errors.
    fn cleanup(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error>;
    /// Quarantine ambiguous state for operator inspection.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific quarantine errors.
    fn quarantine(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error>;
}
/// Runtime-owned adapter for applying stuck-VM cleanup decisions.
pub trait StuckVmCleaner {
    /// Error returned by the cleaner implementation.
    type Error;
    /// Preserve a VM that is not considered stuck.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific preserve errors.
    fn preserve(&mut self, vm_id: &str) -> Result<(), Self::Error>;
    /// Clean up a VM whose heartbeat exceeded the timeout.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific cleanup errors.
    fn cleanup(&mut self, vm_id: &str) -> Result<(), Self::Error>;
    /// Quarantine an ambiguous VM record for operator inspection.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific quarantine errors.
    fn quarantine(&mut self, vm_id: &str) -> Result<(), Self::Error>;
}
/// Request to terminate a host process that owns VM state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostProcessTerminationRequest {
    pub(crate) pid: u32,
    expected_executable: PathBuf,
}
impl HostProcessTerminationRequest {
    /// Construct a host process termination request.
    #[must_use]
    pub fn new(pid: u32, expected_executable: impl Into<PathBuf>) -> Self {
        Self {
            pid,
            expected_executable: expected_executable.into(),
        }
    }
    /// Return the target PID.
    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.pid
    }
    /// Return the expected executable path recorded by the runtime marker.
    #[must_use]
    pub fn expected_executable(&self) -> &Path {
        &self.expected_executable
    }
}
/// Runtime-owned adapter for terminating a host process that owns VM state.
pub trait HostProcessTerminator {
    /// Error returned by the terminator implementation.
    type Error;
    /// Terminate one host process by PID.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific termination errors.
    fn terminate_process(
        &mut self,
        request: &HostProcessTerminationRequest,
    ) -> Result<(), Self::Error>;
}
impl<T> HostProcessTerminator for &mut T
where
    T: HostProcessTerminator + ?Sized,
{
    type Error = T::Error;
    fn terminate_process(
        &mut self,
        request: &HostProcessTerminationRequest,
    ) -> Result<(), Self::Error> {
        (**self).terminate_process(request)
    }
}
/// Runtime host scan filesystem adapter error.
#[derive(Debug, ThisError)]
pub enum RuntimeHostScanError {
    /// Filesystem operation failed.
    #[error("runtime host scan filesystem operation failed while {operation} `{path}`: {source}")]
    Io {
        /// Operation being attempted.
        operation: &'static str,
        /// Path being scanned.
        path: PathBuf,
        /// Source I/O error.
        #[source]
        source: io::Error,
    },
    /// Active VM heartbeat marker could not be parsed.
    #[error("runtime host scan heartbeat marker `{path}` is invalid: {source}")]
    InvalidHeartbeat {
        /// Heartbeat marker path.
        path: PathBuf,
        /// Parse source error.
        #[source]
        source: std::num::ParseIntError,
    },
    /// Active VM runtime PID marker could not be parsed.
    #[error("runtime host scan runtime pid marker `{path}` is invalid: {source}")]
    InvalidRuntimePid {
        /// Runtime PID marker path.
        path: PathBuf,
        /// Parse source error.
        #[source]
        source: std::num::ParseIntError,
    },
}
/// Filesystem roots used to discover runtime host state after restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeHostScanner {
    active_vms: PathBuf,
    snapshots: PathBuf,
    pub(crate) logs: PathBuf,
    pub(crate) processes: PathBuf,
    now: SystemTime,
}
impl RuntimeHostScanner {
    /// Construct a runtime host scanner from marker roots.
    #[must_use]
    pub fn new(
        active_vm_root: impl Into<PathBuf>,
        snapshot_root: impl Into<PathBuf>,
        log_root: impl Into<PathBuf>,
        process_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            active_vms: active_vm_root.into(),
            snapshots: snapshot_root.into(),
            logs: log_root.into(),
            processes: process_root.into(),
            now: SystemTime::now(),
        }
    }
    /// Override the scan clock used to calculate heartbeat ages.
    #[must_use]
    pub fn with_now(mut self, now: SystemTime) -> Self {
        self.now = now;
        self
    }
    /// Discover host runtime state from configured filesystem roots.
    ///
    /// Active VM marker files contain the last heartbeat timestamp as Unix
    /// epoch seconds. File or directory names are used as stable resource ids
    /// for all roots.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHostScanError`] when a root cannot be read or an active
    /// VM heartbeat marker is invalid.
    pub fn scan(&self) -> Result<HostRuntimeScan, RuntimeHostScanError> {
        let mut scan = HostRuntimeScan::new();
        for (id, path) in read_marker_entries(&self.active_vms)? {
            let heartbeat_path = path.join("heartbeat");
            let heartbeat = std::fs::read_to_string(&heartbeat_path).map_err(|source| {
                RuntimeHostScanError::Io {
                    operation: "read active VM heartbeat",
                    path: heartbeat_path.clone(),
                    source,
                }
            })?;
            let heartbeat_epoch = heartbeat.trim().parse::<u64>().map_err(|source| {
                RuntimeHostScanError::InvalidHeartbeat {
                    path: heartbeat_path.clone(),
                    source,
                }
            })?;
            let runtime_pid_path = path.join("runtime.pid");
            let runtime_pid = std::fs::read_to_string(&runtime_pid_path)
                .map_err(|source| RuntimeHostScanError::Io {
                    operation: "read active VM runtime pid",
                    path: runtime_pid_path.clone(),
                    source,
                })?
                .trim()
                .parse::<u32>()
                .map_err(|source| RuntimeHostScanError::InvalidRuntimePid {
                    path: runtime_pid_path,
                    source,
                })?;
            let heartbeat_age = Duration::from_secs(
                system_time_epoch_seconds(self.now).saturating_sub(heartbeat_epoch),
            );
            scan = scan.active_vm_with_runtime_pid(id, heartbeat_age, runtime_pid);
        }
        for (id, _) in read_marker_entries(&self.snapshots)? {
            scan = scan.snapshot_artifact(id);
        }
        for (id, _) in read_marker_entries(&self.logs)? {
            scan = scan.log_stream(id);
        }
        for (id, _) in read_marker_entries(&self.processes)? {
            scan = scan.stale_runtime_process(id);
        }
        Ok(scan)
    }
}
/// Runtime-level restart recovery owner for one set of filesystem marker roots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeRestartRecovery {
    active_vm_root: PathBuf,
    pub(crate) snapshot_root: PathBuf,
    pub(crate) log_root: PathBuf,
    process_root: PathBuf,
    quarantine_root: PathBuf,
    heartbeat_timeout: Duration,
}
impl RuntimeRestartRecovery {
    /// Construct restart recovery from runtime marker roots.
    #[must_use]
    pub fn new(
        active_vm_root: impl Into<PathBuf>,
        snapshot_root: impl Into<PathBuf>,
        log_root: impl Into<PathBuf>,
        process_root: impl Into<PathBuf>,
        quarantine_root: impl Into<PathBuf>,
        heartbeat_timeout: Duration,
    ) -> Self {
        Self {
            active_vm_root: active_vm_root.into(),
            snapshot_root: snapshot_root.into(),
            log_root: log_root.into(),
            process_root: process_root.into(),
            quarantine_root: quarantine_root.into(),
            heartbeat_timeout,
        }
    }
    /// Scan host markers, apply restart reconciliation, then clean stuck VMs.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeRestartRecoveryError`] when scanning, marker
    /// reconciliation, or host-process stuck-VM cleanup fails.
    pub fn execute_with_terminator<T>(
        &self,
        terminator: &mut T,
    ) -> Result<RuntimeRestartRecoveryReport, RuntimeRestartRecoveryError<T::Error>>
    where
        T: HostProcessTerminator,
    {
        let scan = RuntimeHostScanner::new(
            &self.active_vm_root,
            &self.snapshot_root,
            &self.log_root,
            &self.process_root,
        )
        .scan()
        .map_err(RuntimeRestartRecoveryError::Scan)?;
        let reconciliation = scan.reconciliation_plan();
        let stuck_vms = scan.stuck_vm_cleanup_plan(self.heartbeat_timeout);
        let mut restart_executor = RuntimeFilesystemReconciler::new(
            &self.active_vm_root,
            &self.snapshot_root,
            &self.log_root,
            &self.process_root,
            &self.quarantine_root,
        );
        let restart = RestartReconciliation::new(&reconciliation)
            .execute(&mut restart_executor)
            .map_err(RuntimeRestartRecoveryError::Reconcile)?;
        let marker_cleaner = RuntimeFilesystemReconciler::new(
            &self.active_vm_root,
            &self.snapshot_root,
            &self.log_root,
            &self.process_root,
            &self.quarantine_root,
        );
        let mut stuck_vm_cleaner =
            RuntimeHostProcessStuckVmCleaner::new(marker_cleaner, terminator);
        let stuck_vm = RuntimeStuckVmCleanup::new(&stuck_vms)
            .execute(&mut stuck_vm_cleaner)
            .map_err(RuntimeRestartRecoveryError::StuckVm)?;
        Ok(RuntimeRestartRecoveryReport { restart, stuck_vm })
    }
}
/// Report from a complete runtime restart recovery pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeRestartRecoveryReport {
    restart: ReconciliationRuntimeReport,
    stuck_vm: StuckVmRuntimeReport,
}
impl RuntimeRestartRecoveryReport {
    /// Return restart reconciliation counts.
    #[must_use]
    pub const fn restart(&self) -> &ReconciliationRuntimeReport {
        &self.restart
    }
    /// Return stuck-VM cleanup counts.
    #[must_use]
    pub const fn stuck_vm(&self) -> &StuckVmRuntimeReport {
        &self.stuck_vm
    }
}
/// Error returned by a complete runtime restart recovery pass.
#[derive(Debug, ThisError)]
pub enum RuntimeRestartRecoveryError<E> {
    /// Host marker scan failed.
    #[error("runtime restart recovery host scan failed: {0}")]
    Scan(#[source] RuntimeHostScanError),
    /// Restart reconciliation failed.
    #[error("runtime restart recovery reconciliation failed: {0}")]
    Reconcile(#[source] ReconciliationRuntimeError<RuntimeFilesystemReconcilerError>),
    /// Stuck-VM cleanup failed.
    #[error("runtime restart recovery stuck VM cleanup failed: {0}")]
    StuckVm(#[source] StuckVmRuntimeError<RuntimeHostProcessStuckVmCleanerError<E>>),
}
/// Filesystem-backed reconciler for runtime marker cleanup and quarantine.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeFilesystemReconciler {
    active_vm_root: PathBuf,
    pub(crate) snapshot_root: PathBuf,
    pub(crate) log_root: PathBuf,
    process_root: PathBuf,
    quarantine_root: PathBuf,
}
impl RuntimeFilesystemReconciler {
    /// Construct a filesystem reconciler from runtime marker roots.
    #[must_use]
    pub fn new(
        active_vm_root: impl Into<PathBuf>,
        snapshot_root: impl Into<PathBuf>,
        log_root: impl Into<PathBuf>,
        process_root: impl Into<PathBuf>,
        quarantine_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            active_vm_root: active_vm_root.into(),
            snapshot_root: snapshot_root.into(),
            log_root: log_root.into(),
            process_root: process_root.into(),
            quarantine_root: quarantine_root.into(),
        }
    }
    fn marker_root(&self, kind: RestartResourceKind) -> &Path {
        match kind {
            RestartResourceKind::ActiveVm => &self.active_vm_root,
            RestartResourceKind::SnapshotArtifact => &self.snapshot_root,
            RestartResourceKind::LogStream => &self.log_root,
            RestartResourceKind::StaleRuntimeProcess => &self.process_root,
        }
    }
    fn marker_path(
        &self,
        kind: RestartResourceKind,
        id: &str,
    ) -> Result<PathBuf, RuntimeFilesystemReconcilerError> {
        validate_marker_id(kind, id)?;
        Ok(self.marker_root(kind).join(id))
    }
    /// Return runtime process metadata recorded in one active VM marker.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeFilesystemReconcilerError`] when the marker id is
    /// invalid or the runtime metadata cannot be read.
    pub fn active_vm_runtime_metadata(
        &self,
        vm_id: &str,
    ) -> Result<ActiveVmRuntimeMetadata, RuntimeFilesystemReconcilerError> {
        let marker = self.marker_path(RestartResourceKind::ActiveVm, vm_id)?;
        let pid_path = marker.join("runtime.pid");
        let pid = std::fs::read_to_string(&pid_path)
            .map_err(|source| RuntimeFilesystemReconcilerError::Io {
                operation: "read active VM runtime pid marker",
                path: pid_path.clone(),
                source,
            })?
            .trim()
            .parse::<u32>()
            .map_err(
                |source| RuntimeFilesystemReconcilerError::InvalidRuntimePid {
                    path: pid_path,
                    source,
                },
            )?;
        let executable_path = marker.join("runtime.executable");
        let executable = std::fs::read_to_string(&executable_path)
            .map_err(|source| RuntimeFilesystemReconcilerError::Io {
                operation: "read active VM runtime executable marker",
                path: executable_path,
                source,
            })?
            .trim()
            .to_owned();
        Ok(ActiveVmRuntimeMetadata::new(pid, executable))
    }
    /// Return the runtime PID recorded in one active VM marker.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeFilesystemReconcilerError`] when the marker metadata
    /// cannot be read.
    pub fn active_vm_runtime_pid(
        &self,
        vm_id: &str,
    ) -> Result<u32, RuntimeFilesystemReconcilerError> {
        Ok(self.active_vm_runtime_metadata(vm_id)?.pid())
    }
    fn preserve_marker(
        &self,
        kind: RestartResourceKind,
        id: &str,
    ) -> Result<(), RuntimeFilesystemReconcilerError> {
        let path = self.marker_path(kind, id)?;
        if path.exists() {
            Ok(())
        } else {
            Err(RuntimeFilesystemReconcilerError::MissingMarker {
                kind: restart_resource_kind_label(kind),
                id: id.to_owned(),
                path,
            })
        }
    }
    fn cleanup_marker(
        &self,
        kind: RestartResourceKind,
        id: &str,
    ) -> Result<(), RuntimeFilesystemReconcilerError> {
        let path = self.marker_path(kind, id)?;
        remove_marker_path(path)
    }
    fn quarantine_marker(
        &self,
        kind: RestartResourceKind,
        id: &str,
    ) -> Result<(), RuntimeFilesystemReconcilerError> {
        let quarantine_dir = self.quarantine_root.join(restart_resource_kind_label(kind));
        std::fs::create_dir_all(&quarantine_dir).map_err(|source| {
            RuntimeFilesystemReconcilerError::Io {
                operation: "create quarantine directory",
                path: quarantine_dir.clone(),
                source,
            }
        })?;
        if id.is_empty() {
            let path = quarantine_dir.join("ambiguous");
            std::fs::write(&path, b"ambiguous runtime marker\n").map_err(|source| {
                RuntimeFilesystemReconcilerError::Io {
                    operation: "write quarantine marker",
                    path,
                    source,
                }
            })?;
            return Ok(());
        }
        let source = self.marker_path(kind, id)?;
        let destination = quarantine_dir.join(id);
        if destination.exists() {
            return Err(
                RuntimeFilesystemReconcilerError::QuarantineDestinationExists {
                    kind: restart_resource_kind_label(kind),
                    id: id.to_owned(),
                    path: destination,
                },
            );
        }
        std::fs::rename(&source, &destination).map_err(|source_error| {
            RuntimeFilesystemReconcilerError::Io {
                operation: "move marker to quarantine",
                path: source,
                source: source_error,
            }
        })
    }
}
impl ReconciliationExecutor for RuntimeFilesystemReconciler {
    type Error = RuntimeFilesystemReconcilerError;
    fn recover(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error> {
        self.preserve_marker(record.kind(), record.id())
    }
    fn cleanup(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error> {
        self.cleanup_marker(record.kind(), record.id())
    }
    fn quarantine(&mut self, record: &RestartStateRecord) -> Result<(), Self::Error> {
        self.quarantine_marker(record.kind(), record.id())
    }
}
impl StuckVmCleaner for RuntimeFilesystemReconciler {
    type Error = RuntimeFilesystemReconcilerError;
    fn preserve(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        self.preserve_marker(RestartResourceKind::ActiveVm, vm_id)
    }
    fn cleanup(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        self.cleanup_marker(RestartResourceKind::ActiveVm, vm_id)
    }
    fn quarantine(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        self.quarantine_marker(RestartResourceKind::ActiveVm, vm_id)
    }
}
/// Runtime process metadata recorded by one active-VM marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveVmRuntimeMetadata {
    pub(crate) pid: u32,
    executable: PathBuf,
}
impl ActiveVmRuntimeMetadata {
    /// Construct runtime process metadata.
    #[must_use]
    pub fn new(pid: u32, executable: impl Into<PathBuf>) -> Self {
        Self {
            pid,
            executable: executable.into(),
        }
    }
    /// Return the runtime process id.
    #[must_use]
    pub const fn pid(&self) -> u32 {
        self.pid
    }
    /// Return the recorded runtime executable path.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }
}
/// Filesystem marker cleaner that terminates the host process owning a stuck VM first.
#[derive(Debug)]
pub struct RuntimeHostProcessStuckVmCleaner<T> {
    marker_cleaner: RuntimeFilesystemReconciler,
    terminator: T,
}
impl<T> RuntimeHostProcessStuckVmCleaner<T> {
    /// Construct a host-process backed stuck-VM cleaner.
    #[must_use]
    pub const fn new(marker_cleaner: RuntimeFilesystemReconciler, terminator: T) -> Self {
        Self {
            marker_cleaner,
            terminator,
        }
    }
    /// Return the inner process terminator.
    #[must_use]
    pub const fn terminator(&self) -> &T {
        &self.terminator
    }
}
impl<T> StuckVmCleaner for RuntimeHostProcessStuckVmCleaner<T>
where
    T: HostProcessTerminator,
{
    type Error = RuntimeHostProcessStuckVmCleanerError<T::Error>;
    fn preserve(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        self.marker_cleaner
            .preserve(vm_id)
            .map_err(RuntimeHostProcessStuckVmCleanerError::Marker)
    }
    fn cleanup(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        let metadata = self
            .marker_cleaner
            .active_vm_runtime_metadata(vm_id)
            .map_err(RuntimeHostProcessStuckVmCleanerError::Marker)?;
        let request =
            HostProcessTerminationRequest::new(metadata.pid(), metadata.executable().to_path_buf());
        self.terminator
            .terminate_process(&request)
            .map_err(|source| RuntimeHostProcessStuckVmCleanerError::Terminate {
                pid: request.pid(),
                source,
            })?;
        StuckVmCleaner::cleanup(&mut self.marker_cleaner, vm_id)
            .map_err(RuntimeHostProcessStuckVmCleanerError::Marker)
    }
    fn quarantine(&mut self, vm_id: &str) -> Result<(), Self::Error> {
        StuckVmCleaner::quarantine(&mut self.marker_cleaner, vm_id)
            .map_err(RuntimeHostProcessStuckVmCleanerError::Marker)
    }
}
/// Error returned by host-process backed stuck-VM cleanup.
#[derive(Debug, ThisError)]
pub enum RuntimeHostProcessStuckVmCleanerError<E> {
    /// Marker cleanup or lookup failed.
    #[error("{0}")]
    Marker(#[from] RuntimeFilesystemReconcilerError),
    /// Host process termination failed.
    #[error("host process termination failed for pid {pid}: {source}")]
    Terminate {
        /// PID being terminated.
        pid: u32,
        /// Source terminator error.
        #[source]
        source: E,
    },
}
/// `/bin/kill` backed host process terminator for operator cleanup paths.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandHostProcessTerminator;
impl HostProcessTerminator for CommandHostProcessTerminator {
    type Error = CommandHostProcessTerminateError;
    fn terminate_process(
        &mut self,
        request: &HostProcessTerminationRequest,
    ) -> Result<(), Self::Error> {
        let actual = host_process_command(request.pid())?;
        if !process_command_matches_executable(&actual, request.expected_executable()) {
            return Err(CommandHostProcessTerminateError::ExecutableMismatch {
                pid: request.pid(),
                expected: request.expected_executable().to_path_buf(),
                actual,
            });
        }
        let status = Command::new("/bin/kill")
            .arg("-TERM")
            .arg(request.pid().to_string())
            .status()
            .map_err(CommandHostProcessTerminateError::Io)?;
        if status.success() {
            Ok(())
        } else {
            Err(CommandHostProcessTerminateError::Status {
                pid: request.pid(),
                status,
            })
        }
    }
}
fn host_process_command(pid: u32) -> Result<String, CommandHostProcessTerminateError> {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .map_err(CommandHostProcessTerminateError::PsIo)?;
    if !output.status.success() {
        return Err(CommandHostProcessTerminateError::PsStatus {
            pid,
            status: output.status,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn process_command_matches_executable(actual: &str, expected: &Path) -> bool {
    let actual_path = Path::new(actual);
    actual_path == expected
        || actual_path
            .file_name()
            .zip(expected.file_name())
            .is_some_and(|(actual, expected)| actual == expected)
}
/// Error returned by [`CommandHostProcessTerminator`].
#[derive(Debug, ThisError)]
pub enum CommandHostProcessTerminateError {
    /// `ps` could not be executed.
    #[error("failed to execute /bin/ps: {0}")]
    PsIo(#[source] io::Error),
    /// `ps` exited unsuccessfully.
    #[error("/bin/ps -p {pid} exited with {status}")]
    PsStatus {
        /// PID passed to ps.
        pid: u32,
        /// Process exit status.
        status: std::process::ExitStatus,
    },
    /// Live process command did not match the executable recorded in the marker.
    #[error(
        "refusing to terminate pid {pid}: expected executable `{expected:?}`, found `{actual}`"
    )]
    ExecutableMismatch {
        /// PID being checked.
        pid: u32,
        /// Expected executable from the active VM marker.
        expected: PathBuf,
        /// Actual command reported by ps.
        actual: String,
    },
    /// `kill` could not be executed.
    #[error("failed to execute /bin/kill: {0}")]
    Io(#[source] io::Error),
    /// `kill` exited unsuccessfully.
    #[error("/bin/kill -TERM {pid} exited with {status}")]
    Status {
        /// PID passed to kill.
        pid: u32,
        /// Process exit status.
        status: std::process::ExitStatus,
    },
}
/// Error returned by the filesystem runtime reconciler.
#[derive(Debug, ThisError)]
pub enum RuntimeFilesystemReconcilerError {
    /// A cleanup or preserve operation received an empty resource id.
    #[error("runtime marker id for `{kind}` is empty")]
    EmptyMarkerId {
        /// Resource kind being acted on.
        kind: &'static str,
    },
    /// A marker id contained path separators or special path components.
    #[error("runtime marker id `{id}` for `{kind}` is not a single path component")]
    InvalidMarkerId {
        /// Resource kind being acted on.
        kind: &'static str,
        /// Invalid marker id.
        id: String,
    },
    /// A marker expected by the reconciliation plan was missing.
    #[error("runtime marker `{id}` for `{kind}` is missing at `{path}`")]
    MissingMarker {
        /// Resource kind being acted on.
        kind: &'static str,
        /// Missing marker id.
        id: String,
        /// Expected marker path.
        path: PathBuf,
    },
    /// Runtime PID marker could not be parsed.
    #[error("runtime pid marker at `{path}` is invalid: {source}")]
    InvalidRuntimePid {
        /// Runtime pid marker path.
        path: PathBuf,
        /// Parse source error.
        #[source]
        source: std::num::ParseIntError,
    },
    /// A quarantine destination already existed.
    #[error("runtime marker quarantine destination for `{id}` `{kind}` already exists at `{path}`")]
    QuarantineDestinationExists {
        /// Resource kind being acted on.
        kind: &'static str,
        /// Marker id being quarantined.
        id: String,
        /// Existing quarantine path.
        path: PathBuf,
    },
    /// Filesystem operation failed.
    #[error("runtime marker filesystem operation `{operation}` failed at `{path}`: {source}")]
    Io {
        /// Operation name.
        operation: &'static str,
        /// Path being acted on.
        path: PathBuf,
        /// Source IO error.
        source: io::Error,
    },
}
pub(crate) fn validate_marker_id(
    kind: RestartResourceKind,
    id: &str,
) -> Result<(), RuntimeFilesystemReconcilerError> {
    if id.is_empty() {
        return Err(RuntimeFilesystemReconcilerError::EmptyMarkerId {
            kind: restart_resource_kind_label(kind),
        });
    }
    let mut components = Path::new(id).components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        Ok(())
    } else {
        Err(RuntimeFilesystemReconcilerError::InvalidMarkerId {
            kind: restart_resource_kind_label(kind),
            id: id.to_owned(),
        })
    }
}
fn remove_marker_path(path: PathBuf) -> Result<(), RuntimeFilesystemReconcilerError> {
    let metadata = std::fs::symlink_metadata(&path).map_err(|source| {
        RuntimeFilesystemReconcilerError::Io {
            operation: "stat marker",
            path: path.clone(),
            source,
        }
    })?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(&path).map_err(|source| RuntimeFilesystemReconcilerError::Io {
            operation: "remove marker directory",
            path,
            source,
        })
    } else {
        std::fs::remove_file(&path).map_err(|source| RuntimeFilesystemReconcilerError::Io {
            operation: "remove marker file",
            path,
            source,
        })
    }
}
fn restart_resource_kind_label(kind: RestartResourceKind) -> &'static str {
    match kind {
        RestartResourceKind::ActiveVm => "active_vm",
        RestartResourceKind::SnapshotArtifact => "snapshot_artifact",
        RestartResourceKind::LogStream => "log_stream",
        RestartResourceKind::StaleRuntimeProcess => "stale_runtime_process",
    }
}
fn read_marker_entries(root: &Path) -> Result<Vec<(String, PathBuf)>, RuntimeHostScanError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(RuntimeHostScanError::Io {
                operation: "read marker root",
                path: root.to_path_buf(),
                source,
            });
        }
    };
    let mut markers = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| RuntimeHostScanError::Io {
            operation: "read marker entry",
            path: root.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let id = entry.file_name().to_string_lossy().into_owned();
        markers.push((id, path));
    }
    markers.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(markers)
}
fn system_time_epoch_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}
pub(crate) fn write_active_vm_heartbeat(path: &Path) -> Result<(), BackendError> {
    std::fs::create_dir_all(path).map_err(|source| {
        BackendError::Runtime(format!(
            "Firkin active VM marker directory create failed at `{}`: {source}",
            path.display()
        ))
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| {
            BackendError::Runtime(format!(
                "Firkin active VM marker root create failed at `{}`: {source}",
                parent.display()
            ))
        })?;
    }
    let heartbeat = format!("{}\n", system_time_epoch_seconds(SystemTime::now()));
    std::fs::write(path.join("heartbeat"), heartbeat).map_err(|source| {
        BackendError::Runtime(format!(
            "Firkin active VM marker publish failed at `{}`: {source}",
            path.display()
        ))
    })?;
    std::fs::write(
        path.join("runtime.pid"),
        format!("{}\n", std::process::id()),
    )
    .map_err(|source| {
        BackendError::Runtime(format!(
            "Firkin active VM runtime pid marker publish failed at `{}`: {source}",
            path.display()
        ))
    })?;
    let executable = std::env::current_exe().map_err(|source| {
        BackendError::Runtime(format!(
            "Firkin active VM runtime executable discover failed: {source}"
        ))
    })?;
    std::fs::write(
        path.join("runtime.executable"),
        executable.display().to_string(),
    )
    .map_err(|source| {
        BackendError::Runtime(format!(
            "Firkin active VM runtime executable marker publish failed at `{}`: {source}",
            path.display()
        ))
    })
}
/// Runtime-level snapshot artifact garbage collection executor.
#[derive(Clone, Debug)]
pub struct RuntimeSnapshotArtifactGc<I> {
    pub(crate) root: PathBuf,
    manifests: I,
    min_unreferenced_age: Duration,
}
impl<I> RuntimeSnapshotArtifactGc<I> {
    /// Construct a runtime snapshot artifact GC operation.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, manifests: I) -> Self {
        Self {
            root: root.into(),
            manifests,
            min_unreferenced_age: Duration::ZERO,
        }
    }
    /// Retain unreferenced snapshot artifacts newer than `min_unreferenced_age`.
    #[must_use]
    pub const fn with_min_unreferenced_age(mut self, min_unreferenced_age: Duration) -> Self {
        self.min_unreferenced_age = min_unreferenced_age;
        self
    }
}
impl RuntimeSnapshotArtifactGc<Vec<SnapshotArtifactManifest>> {
    /// Construct runtime artifact GC from manifest sidecars in `manifest_root`.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactGcError`] when manifest sidecar discovery fails.
    pub fn from_manifest_dir(
        root: impl Into<PathBuf>,
        manifest_root: impl AsRef<Path>,
    ) -> Result<Self, ArtifactGcError> {
        let manifests = SnapshotArtifactManifest::read_json_dir(manifest_root)
            .map_err(ArtifactGcError::from)?;
        Ok(Self::new(root, manifests))
    }
}
impl<I> RuntimeSnapshotArtifactGc<I>
where
    I: IntoIterator<Item = SnapshotArtifactManifest>,
{
    /// Execute snapshot artifact GC through the substrate plan.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactGcError`] when planning or deleting artifacts fails.
    pub fn execute(self) -> Result<ArtifactGcReport, ArtifactGcError> {
        ArtifactGcPlan::for_snapshot_dir_with_retention(
            &self.root,
            self.manifests,
            self.min_unreferenced_age,
            SystemTime::now(),
        )?
        .execute()
    }
}
/// Runtime-level log rotation executor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeLogRotation {
    pub(crate) root: PathBuf,
    max_bytes: u64,
    max_rotated_files: u32,
    gzip_compression: bool,
}
impl RuntimeLogRotation {
    /// Construct a runtime log rotation operation.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            root: root.into(),
            max_bytes,
            max_rotated_files: 1,
            gzip_compression: false,
        }
    }
    /// Retain up to `max_rotated_files` rotated generations per active log.
    #[must_use]
    pub const fn with_max_rotated_files(mut self, max_rotated_files: u32) -> Self {
        self.max_rotated_files = max_rotated_files;
        self
    }
    /// Compress rotated log files using gzip.
    #[must_use]
    pub const fn with_gzip_compression(mut self) -> Self {
        self.gzip_compression = true;
        self
    }
    /// Execute log rotation through the substrate plan.
    ///
    /// # Errors
    ///
    /// Returns [`LogRotationError`] when planning or rotating logs fails.
    pub fn execute(self) -> Result<LogRotationReport, LogRotationError> {
        let plan = LogRotationPlan::for_log_dir_with_retention(
            &self.root,
            self.max_bytes,
            self.max_rotated_files,
        )?;
        if self.gzip_compression {
            plan.with_gzip_compression().execute()
        } else {
            plan.execute()
        }
    }
}
/// Report from one runtime hygiene maintenance tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeHygieneMaintenanceReport {
    artifact_gc: ArtifactGcReport,
    log_rotation: LogRotationReport,
}
impl RuntimeHygieneMaintenanceReport {
    /// Construct a runtime hygiene maintenance report.
    #[must_use]
    pub const fn new(artifact_gc: ArtifactGcReport, log_rotation: LogRotationReport) -> Self {
        Self {
            artifact_gc,
            log_rotation,
        }
    }
    /// Return snapshot artifact GC results.
    #[must_use]
    pub const fn artifact_gc(&self) -> &ArtifactGcReport {
        &self.artifact_gc
    }
    /// Return log rotation results.
    #[must_use]
    pub const fn log_rotation(&self) -> &LogRotationReport {
        &self.log_rotation
    }
}
/// Runtime hygiene maintenance error.
#[derive(Debug, ThisError)]
pub enum RuntimeHygieneMaintenanceError {
    /// Snapshot artifact GC failed.
    #[error("runtime hygiene snapshot artifact GC failed: {0}")]
    ArtifactGc(#[from] ArtifactGcError),
    /// Log rotation failed.
    #[error("runtime hygiene log rotation failed: {0}")]
    LogRotation(#[from] LogRotationError),
    /// Spawned maintenance task failed.
    #[error("runtime hygiene maintenance task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}
/// Long-lived owner for runtime snapshot/log hygiene tasks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeHygieneMaintenance {
    pub(crate) snapshot_root: PathBuf,
    manifests: Vec<SnapshotArtifactManifest>,
    manifest_root: Option<PathBuf>,
    min_unreferenced_snapshot_age: Duration,
    pub(crate) log_root: PathBuf,
    max_log_bytes: u64,
    max_rotated_logs: u32,
    gzip_log_compression: bool,
    pub(crate) interval: Duration,
}
impl RuntimeHygieneMaintenance {
    /// Construct a runtime hygiene maintenance owner.
    #[must_use]
    pub fn new(
        snapshot_root: impl Into<PathBuf>,
        manifests: impl IntoIterator<Item = SnapshotArtifactManifest>,
        log_root: impl Into<PathBuf>,
        max_log_bytes: u64,
        interval: Duration,
    ) -> Self {
        Self {
            snapshot_root: snapshot_root.into(),
            manifests: manifests.into_iter().collect(),
            manifest_root: None,
            min_unreferenced_snapshot_age: Duration::ZERO,
            log_root: log_root.into(),
            max_log_bytes,
            max_rotated_logs: 1,
            gzip_log_compression: false,
            interval,
        }
    }
    /// Retain unreferenced snapshot artifacts newer than `min_age`.
    #[must_use]
    pub const fn with_min_unreferenced_snapshot_age(mut self, min_age: Duration) -> Self {
        self.min_unreferenced_snapshot_age = min_age;
        self
    }
    /// Discover snapshot manifests from direct `*.manifest.json` sidecars under
    /// `manifest_root` on every maintenance tick.
    #[must_use]
    pub fn with_manifest_dir(mut self, manifest_root: impl Into<PathBuf>) -> Self {
        self.manifest_root = Some(manifest_root.into());
        self
    }
    /// Retain up to `max_rotated_logs` rotated generations per active log.
    #[must_use]
    pub const fn with_max_rotated_logs(mut self, max_rotated_logs: u32) -> Self {
        self.max_rotated_logs = max_rotated_logs;
        self
    }
    /// Compress rotated logs using gzip.
    #[must_use]
    pub const fn with_gzip_log_compression(mut self) -> Self {
        self.gzip_log_compression = true;
        self
    }
    /// Return the intended maintenance cadence.
    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }
    /// Run one snapshot/log hygiene maintenance pass.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHygieneMaintenanceError`] when artifact GC or log
    /// rotation fails.
    pub fn tick(&self) -> Result<RuntimeHygieneMaintenanceReport, RuntimeHygieneMaintenanceError> {
        let manifests = if let Some(manifest_root) = &self.manifest_root {
            SnapshotArtifactManifest::read_json_dir(manifest_root).map_err(ArtifactGcError::from)?
        } else {
            self.manifests.clone()
        };
        let artifact_gc = RuntimeSnapshotArtifactGc::new(&self.snapshot_root, manifests)
            .with_min_unreferenced_age(self.min_unreferenced_snapshot_age)
            .execute()?;
        let log_rotation = RuntimeLogRotation::new(&self.log_root, self.max_log_bytes)
            .with_max_rotated_files(self.max_rotated_logs);
        let log_rotation = if self.gzip_log_compression {
            log_rotation.with_gzip_compression()
        } else {
            log_rotation
        }
        .execute()?;
        Ok(RuntimeHygieneMaintenanceReport::new(
            artifact_gc,
            log_rotation,
        ))
    }
    /// Spawn periodic hygiene maintenance until the returned handle shuts it down.
    #[must_use]
    pub fn spawn(self) -> RuntimeHygieneMaintenanceHandle {
        let interval = self.interval;
        let state = Arc::new(Mutex::new(self));
        let task_state = Arc::clone(&state);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                {
                    let maintenance = task_state.lock().await;
                    maintenance.tick()?;
                }
                tokio::select! {
                    _ = & mut shutdown_rx => return Ok(()), () =
                    tokio::time::sleep(interval) => {}
                }
            }
        });
        RuntimeHygieneMaintenanceHandle {
            state,
            shutdown: Some(shutdown_tx),
            task,
        }
    }
}
/// Handle for a spawned runtime hygiene maintenance loop.
#[derive(Debug)]
pub struct RuntimeHygieneMaintenanceHandle {
    pub(crate) state: Arc<Mutex<RuntimeHygieneMaintenance>>,
    pub(crate) shutdown: Option<oneshot::Sender<()>>,
    pub(crate) task: JoinHandle<Result<(), RuntimeHygieneMaintenanceError>>,
}
impl RuntimeHygieneMaintenanceHandle {
    /// Stop the maintenance loop and return the owned maintenance state.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeHygieneMaintenanceError`] if a tick failed or the task
    /// panicked.
    ///
    /// # Panics
    ///
    /// Panics if another reference to the owned maintenance state survives
    /// after the maintenance task has stopped.
    pub async fn shutdown(
        mut self,
    ) -> Result<RuntimeHygieneMaintenance, RuntimeHygieneMaintenanceError> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.await??;
        let mutex = Arc::try_unwrap(self.state).unwrap_or_else(|_| {
            panic!("hygiene maintenance handle owns the only remaining state reference")
        });
        Ok(mutex.into_inner())
    }
}
pub(crate) fn unique_process_match<T>(
    mut matches: impl Iterator<Item = T>,
    missing: impl FnOnce() -> BackendError,
    ambiguous: impl FnOnce() -> BackendError,
) -> Result<T, BackendError> {
    let first = matches.next().ok_or_else(missing)?;
    if matches.next().is_some() {
        return Err(ambiguous());
    }
    Ok(first)
}
pub(crate) fn ambiguous_process_error(selector: &str, value: &str) -> BackendError {
    BackendError::Runtime(format!(
        "Firkin RuntimeAdapter process {selector} `{value}` matches multiple active sandboxes"
    ))
}
pub(crate) fn finite_process_error(pid: u32, detail: &str) -> BackendError {
    BackendError::Runtime(format!(
        "Firkin RuntimeAdapter process {pid} is finite and {detail}"
    ))
}
pub(crate) fn filesystem_exit_error(
    operation: &str,
    path: &str,
    output: EnvdProcessOutput,
) -> BackendError {
    BackendError::Runtime(format!(
        "Firkin filesystem {operation} `{path}` failed: {}",
        output
            .error
            .unwrap_or_else(|| format!("process exited {}", output.exit_code))
    ))
}
pub(crate) fn parse_single_filesystem_entry(
    bytes: &[u8],
) -> Result<EnvdFilesystemEntry, BackendError> {
    let mut entries = parse_filesystem_entries(bytes)?;
    if entries.len() != 1 {
        return Err(BackendError::Runtime(format!(
            "Firkin filesystem metadata expected one entry, got {}",
            entries.len()
        )));
    }
    Ok(entries.remove(0))
}
pub(crate) fn parse_filesystem_entries(
    bytes: &[u8],
) -> Result<Vec<EnvdFilesystemEntry>, BackendError> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        BackendError::Runtime(format!("Firkin filesystem metadata is not UTF-8: {error}"))
    })?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_filesystem_entry_line)
        .collect()
}
fn parse_filesystem_entry_line(line: &str) -> Result<EnvdFilesystemEntry, BackendError> {
    let mut fields = line.split('\t');
    let kind = fields.next().ok_or_else(|| {
        BackendError::Runtime("Firkin filesystem metadata missing kind".to_owned())
    })?;
    let size = fields
        .next()
        .ok_or_else(|| BackendError::Runtime("Firkin filesystem metadata missing size".to_owned()))?
        .parse::<i64>()
        .map_err(|error| BackendError::Runtime(format!("invalid filesystem size: {error}")))?;
    let mode = parse_filesystem_mode(fields.next().ok_or_else(|| {
        BackendError::Runtime("Firkin filesystem metadata missing mode".to_owned())
    })?)?;
    let permissions = fields
        .next()
        .ok_or_else(|| {
            BackendError::Runtime("Firkin filesystem metadata missing permissions".to_owned())
        })?
        .to_owned();
    let owner = fields
        .next()
        .ok_or_else(|| {
            BackendError::Runtime("Firkin filesystem metadata missing owner".to_owned())
        })?
        .to_owned();
    let group = fields
        .next()
        .ok_or_else(|| {
            BackendError::Runtime("Firkin filesystem metadata missing group".to_owned())
        })?
        .to_owned();
    let path = fields
        .next()
        .ok_or_else(|| BackendError::Runtime("Firkin filesystem metadata missing path".to_owned()))?
        .to_owned();
    let symlink_target = fields
        .next()
        .filter(|target| !target.is_empty() && *target != path)
        .map(ToOwned::to_owned);
    let file_type = if kind == "directory" || kind == "Directory" {
        EnvdFilesystemFileType::Directory
    } else {
        EnvdFilesystemFileType::File
    };
    let name = path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or("/")
        .to_owned();
    Ok(EnvdFilesystemEntry {
        name,
        path,
        file_type,
        size,
        mode,
        permissions,
        owner,
        group,
        symlink_target,
    })
}
fn parse_filesystem_mode(value: &str) -> Result<u32, BackendError> {
    value
        .parse::<u32>()
        .or_else(|_| u32::from_str_radix(value, 16))
        .map_err(|error| {
            BackendError::Runtime(format!("invalid filesystem mode `{value}`: {error}"))
        })
}
pub(crate) fn runtime_timestamps(timeout_seconds: u64) -> (String, String) {
    let started_at = OffsetDateTime::now_utc();
    let timeout = i64::try_from(timeout_seconds).unwrap_or(i64::MAX);
    let end_at = started_at + TimeDuration::seconds(timeout);
    (
        started_at
            .format(&Rfc3339)
            .expect("RFC3339 formatting current UTC time is infallible"),
        end_at
            .format(&Rfc3339)
            .expect("RFC3339 formatting current UTC time is infallible"),
    )
}
pub(crate) fn snapshot_artifact_integrity<E>(
    integrity: Option<&PreparedTemplateArtifactIntegrity>,
    manifest: &SnapshotArtifactManifest,
) -> Result<SnapshotArtifactIntegrity, RuntimeCubeSandboxCreateError<E>> {
    if let Some(integrity) = integrity {
        return Ok(prepared_template_artifact_integrity(integrity));
    }
    let sidecar = SnapshotArtifactIntegrity::sidecar_path_for_artifact(manifest.path());
    SnapshotArtifactIntegrity::read_json(&sidecar).map_err(|error| {
        RuntimeCubeSandboxCreateError::Restore(SnapshotRestoreError::Integrity {
            reason: error.to_string(),
        })
    })
}
#[cfg(feature = "snapshot")]
pub(crate) async fn delete_firkin_snapshot_file(path: &Path) -> Result<(), BackendError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(BackendError::Runtime(format!(
            "delete Firkin snapshot artifact {}: {error}",
            path.display()
        ))),
    }
}
/// Runtime restart reconciliation error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum ReconciliationRuntimeError<E> {
    /// Applying one reconciliation decision failed.
    #[error("restart reconciliation failed for record `{record_id}`: {source}")]
    Apply {
        /// Source record id.
        record_id: String,
        /// Source executor error.
        source: E,
    },
}
/// Runtime stuck-VM cleanup error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum StuckVmRuntimeError<E> {
    /// Applying one stuck-VM cleanup decision failed.
    #[error("stuck VM cleanup failed for `{vm_id}`: {source}")]
    Apply {
        /// Source VM id.
        vm_id: String,
        /// Source cleaner error.
        source: E,
    },
}
/// Report from applying restart reconciliation decisions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconciliationRuntimeReport {
    recovered: usize,
    cleaned: usize,
    quarantined: usize,
}
impl ReconciliationRuntimeReport {
    /// Return recovered resource count.
    #[must_use]
    pub const fn recovered_count(&self) -> usize {
        self.recovered
    }
    /// Return cleaned resource count.
    #[must_use]
    pub const fn cleaned_count(&self) -> usize {
        self.cleaned
    }
    /// Return quarantined resource count.
    #[must_use]
    pub const fn quarantined_count(&self) -> usize {
        self.quarantined
    }
}
/// Report from applying stuck-VM cleanup decisions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StuckVmRuntimeReport {
    preserved: usize,
    cleaned: usize,
    quarantined: usize,
}
impl StuckVmRuntimeReport {
    /// Return preserved VM count.
    #[must_use]
    pub const fn preserved_count(&self) -> usize {
        self.preserved
    }
    /// Return cleaned VM count.
    #[must_use]
    pub const fn cleaned_count(&self) -> usize {
        self.cleaned
    }
    /// Return quarantined VM count.
    #[must_use]
    pub const fn quarantined_count(&self) -> usize {
        self.quarantined
    }
}
/// Runtime-level restart reconciliation executor.
#[derive(Debug)]
pub struct RestartReconciliation<'a> {
    pub(crate) plan: &'a ReconciliationPlan,
}
impl<'a> RestartReconciliation<'a> {
    /// Construct a restart reconciliation operation.
    #[must_use]
    pub const fn new(plan: &'a ReconciliationPlan) -> Self {
        Self { plan }
    }
    /// Apply every reconciliation decision in plan order.
    ///
    /// # Errors
    ///
    /// Returns [`ReconciliationRuntimeError`] when applying a decision fails.
    pub fn execute<E>(
        self,
        executor: &mut E,
    ) -> Result<ReconciliationRuntimeReport, ReconciliationRuntimeError<E::Error>>
    where
        E: ReconciliationExecutor,
    {
        let mut report = ReconciliationRuntimeReport::default();
        for entry in self.plan.decisions() {
            let record = entry.record();
            match entry.action() {
                ReconciliationDecision::Recover => {
                    executor.recover(record).map_err(|source| {
                        ReconciliationRuntimeError::Apply {
                            record_id: record.id().to_owned(),
                            source,
                        }
                    })?;
                    report.recovered += 1;
                }
                ReconciliationDecision::Cleanup => {
                    executor.cleanup(record).map_err(|source| {
                        ReconciliationRuntimeError::Apply {
                            record_id: record.id().to_owned(),
                            source,
                        }
                    })?;
                    report.cleaned += 1;
                }
                ReconciliationDecision::Quarantine => {
                    executor.quarantine(record).map_err(|source| {
                        ReconciliationRuntimeError::Apply {
                            record_id: record.id().to_owned(),
                            source,
                        }
                    })?;
                    report.quarantined += 1;
                }
            }
        }
        Ok(report)
    }
}
/// Runtime-level stuck-VM cleanup executor.
#[derive(Debug)]
pub struct RuntimeStuckVmCleanup<'a> {
    pub(crate) plan: &'a StuckVmCleanupPlan,
}
impl<'a> RuntimeStuckVmCleanup<'a> {
    /// Construct a stuck-VM cleanup operation.
    #[must_use]
    pub const fn new(plan: &'a StuckVmCleanupPlan) -> Self {
        Self { plan }
    }
    /// Apply every stuck-VM cleanup decision in plan order.
    ///
    /// # Errors
    ///
    /// Returns [`StuckVmRuntimeError`] when applying a decision fails.
    pub fn execute<C>(
        self,
        cleaner: &mut C,
    ) -> Result<StuckVmRuntimeReport, StuckVmRuntimeError<C::Error>>
    where
        C: StuckVmCleaner,
    {
        let mut report = StuckVmRuntimeReport::default();
        for entry in self.plan.decisions() {
            let vm_id = entry.observation().id();
            match entry.decision() {
                StuckVmCleanupDecision::Preserve => {
                    cleaner
                        .preserve(vm_id)
                        .map_err(|source| StuckVmRuntimeError::Apply {
                            vm_id: vm_id.to_owned(),
                            source,
                        })?;
                    report.preserved += 1;
                }
                StuckVmCleanupDecision::Cleanup => {
                    cleaner
                        .cleanup(vm_id)
                        .map_err(|source| StuckVmRuntimeError::Apply {
                            vm_id: vm_id.to_owned(),
                            source,
                        })?;
                    report.cleaned += 1;
                }
                StuckVmCleanupDecision::Quarantine => {
                    cleaner
                        .quarantine(vm_id)
                        .map_err(|source| StuckVmRuntimeError::Apply {
                            vm_id: vm_id.to_owned(),
                            source,
                        })?;
                    report.quarantined += 1;
                }
            }
        }
        Ok(report)
    }
}
