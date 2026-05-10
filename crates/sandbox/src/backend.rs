use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;

use crate::capability::Capabilities;
use crate::error::Result;
use crate::event::{EventFilter, EventStream};
use crate::filesystem::{
    CopyOptions, FileEntry, FileStat, FilesystemEventStream, ListOptions, ReadOptions, SandboxPath,
    WatchOptions, WriteOptions,
};
use crate::ids::{BackendName, SandboxId, SnapshotId, TemplateId, WarmPoolKey};
use crate::logs::{LogEntry, LogFilter, LogStream};
use crate::metrics::{MetricFilter, MetricSnapshot};
use crate::ports::{
    DomainProxy, DomainProxySpec, GuestPort, PortBinding, PortExposure, PortTarget,
};
use crate::process::{
    Command, CommandOutput, ProcessEventStream, ProcessInfo, ProcessInput, ProcessSelector,
    PtySize, Signal,
};
use crate::runtime::RuntimePreflight;
use crate::sandbox::{
    AttachOptions, DeleteOptions, KillSignal, SandboxDeadline, SandboxFilter, SandboxInfo,
    SandboxSpec, StopMode,
};
use crate::snapshot::{
    PauseOptions, PausedSandbox, RestoreOptions, ResumeOptions, SnapshotExport, SnapshotFilter,
    SnapshotImport, SnapshotInfo, SnapshotOptions, SnapshotRef,
};
use crate::template::{PreparedTemplate, TemplateInfo, TemplateSpec};
use crate::warm_pool::{
    WarmLease, WarmLeasePolicy, WarmMaintainReport, WarmPoolSpec, WarmPoolStatus, WarmPoolTarget,
};

pub type BoxBackend = Arc<dyn SandboxBackend>;

#[async_trait]
pub trait SandboxBackend: Send + Sync {
    async fn capabilities(&self) -> Result<Capabilities>;

    async fn preflight(&self) -> Result<RuntimePreflight>;

    async fn info(&self) -> Result<BackendInfo>;

    fn templates(&self) -> &dyn TemplateControl;

    fn sandboxes(&self) -> &dyn SandboxControl;

    fn snapshots(&self) -> &dyn SnapshotControl;

    fn processes(&self) -> Option<&dyn ProcessControl> {
        None
    }

    fn filesystems(&self) -> Option<&dyn FilesystemControl> {
        None
    }

    fn ports(&self) -> Option<&dyn PortControl> {
        None
    }

    fn pause(&self) -> Option<&dyn PauseControl> {
        None
    }

    fn warm_pool(&self) -> Option<&dyn WarmPoolControl> {
        None
    }

    fn events(&self) -> Option<&dyn EventControl> {
        None
    }

    fn logs(&self) -> Option<&dyn LogControl> {
        None
    }

    fn metrics(&self) -> Option<&dyn MetricControl> {
        None
    }
}

#[async_trait]
pub trait TemplateControl: Send + Sync {
    async fn prepare_template(&self, spec: TemplateSpec) -> Result<PreparedTemplate>;
    async fn get_template(&self, id: &TemplateId) -> Result<TemplateInfo>;
    async fn list_templates(&self) -> Result<Vec<TemplateInfo>>;
    async fn delete_template(&self, id: &TemplateId) -> Result<()>;
}

#[async_trait]
pub trait SandboxControl: Send + Sync {
    async fn create_sandbox(&self, spec: SandboxSpec) -> Result<SandboxInfo>;

    async fn restore_sandbox(
        &self,
        snapshot: SnapshotRef,
        options: RestoreOptions,
    ) -> Result<SandboxInfo>;

    async fn attach_sandbox(&self, id: &SandboxId, options: AttachOptions) -> Result<SandboxInfo>;

    async fn inspect_sandbox(&self, id: &SandboxId) -> Result<SandboxInfo>;

    async fn list_sandboxes(&self, filter: SandboxFilter) -> Result<Vec<SandboxInfo>>;

    async fn stop_sandbox(&self, id: &SandboxId, mode: StopMode) -> Result<()>;

    async fn kill_sandbox(&self, id: &SandboxId, signal: KillSignal) -> Result<()>;

    async fn delete_sandbox(&self, id: &SandboxId, options: DeleteOptions) -> Result<()>;

    async fn update_deadline(&self, id: &SandboxId, deadline: SandboxDeadline) -> Result<()>;
}

#[async_trait]
pub trait ProcessControl: Send + Sync {
    async fn run_process(&self, sandbox: &SandboxId, command: Command) -> Result<CommandOutput>;

    async fn start_process(&self, sandbox: &SandboxId, command: Command) -> Result<ProcessInfo>;

    async fn start_process_stream(
        &self,
        sandbox: &SandboxId,
        command: Command,
    ) -> Result<ProcessEventStream>;

    async fn list_processes(&self, sandbox: &SandboxId) -> Result<Vec<ProcessInfo>>;

    async fn connect_process(
        &self,
        sandbox: &SandboxId,
        selector: ProcessSelector,
    ) -> Result<ProcessInfo>;

    async fn signal_process(
        &self,
        sandbox: &SandboxId,
        selector: ProcessSelector,
        signal: Signal,
    ) -> Result<()>;

    async fn send_process_input(
        &self,
        sandbox: &SandboxId,
        selector: ProcessSelector,
        input: ProcessInput,
    ) -> Result<()>;

    async fn close_process_stdin(
        &self,
        sandbox: &SandboxId,
        selector: ProcessSelector,
    ) -> Result<()>;

    async fn resize_process_pty(
        &self,
        sandbox: &SandboxId,
        selector: ProcessSelector,
        size: PtySize,
    ) -> Result<()>;

    async fn wait_process(
        &self,
        sandbox: &SandboxId,
        selector: ProcessSelector,
    ) -> Result<CommandOutput>;
}

#[async_trait]
pub trait FilesystemControl: Send + Sync {
    async fn read_file(
        &self,
        sandbox: &SandboxId,
        path: SandboxPath,
        options: ReadOptions,
    ) -> Result<Bytes>;

    async fn write_file(
        &self,
        sandbox: &SandboxId,
        path: SandboxPath,
        data: Bytes,
        options: WriteOptions,
    ) -> Result<FileEntry>;

    async fn copy_in(
        &self,
        sandbox: &SandboxId,
        host: &Path,
        guest: SandboxPath,
        options: CopyOptions,
    ) -> Result<FileEntry>;

    async fn copy_out(
        &self,
        sandbox: &SandboxId,
        guest: SandboxPath,
        host: &Path,
        options: CopyOptions,
    ) -> Result<()>;

    async fn list_files(
        &self,
        sandbox: &SandboxId,
        path: SandboxPath,
        options: ListOptions,
    ) -> Result<Vec<FileEntry>>;

    async fn mkdir(
        &self,
        sandbox: &SandboxId,
        path: SandboxPath,
        recursive: bool,
    ) -> Result<FileEntry>;

    async fn rename(
        &self,
        sandbox: &SandboxId,
        from: SandboxPath,
        to: SandboxPath,
    ) -> Result<FileEntry>;

    async fn remove_file(&self, sandbox: &SandboxId, path: SandboxPath) -> Result<()>;

    async fn stat_file(&self, sandbox: &SandboxId, path: SandboxPath) -> Result<FileStat>;

    async fn watch_files(
        &self,
        sandbox: &SandboxId,
        path: SandboxPath,
        options: WatchOptions,
    ) -> Result<FilesystemEventStream>;
}

#[async_trait]
pub trait SnapshotControl: Send + Sync {
    async fn capture_snapshot(
        &self,
        sandbox: &SandboxId,
        options: SnapshotOptions,
    ) -> Result<SnapshotRef>;

    async fn get_snapshot(&self, id: &SnapshotId) -> Result<SnapshotInfo>;

    async fn list_snapshots(&self, filter: SnapshotFilter) -> Result<Vec<SnapshotInfo>>;

    async fn delete_snapshot(&self, id: &SnapshotId) -> Result<()>;

    async fn export_snapshot(&self, id: &SnapshotId) -> Result<SnapshotExport>;

    async fn import_snapshot(&self, import: SnapshotImport) -> Result<SnapshotRef>;

    async fn pause_sandbox(
        &self,
        sandbox: &SandboxId,
        options: PauseOptions,
    ) -> Result<PausedSandbox>;
}

#[async_trait]
pub trait PauseControl: Send + Sync {
    async fn resume_paused(
        &self,
        paused: PausedSandbox,
        options: ResumeOptions,
    ) -> Result<SandboxInfo>;
}

#[async_trait]
pub trait WarmPoolControl: Send + Sync {
    async fn prewarm(
        &self,
        template: &PreparedTemplate,
        spec: WarmPoolSpec,
    ) -> Result<WarmMaintainReport>;

    async fn maintain(&self, targets: Vec<WarmPoolTarget>) -> Result<WarmMaintainReport>;

    async fn status(&self) -> Result<WarmPoolStatus>;

    async fn checkout(
        &self,
        template: &PreparedTemplate,
        policy: WarmLeasePolicy,
    ) -> Result<WarmLease>;

    async fn evict(&self, key: WarmPoolKey, count: usize) -> Result<WarmMaintainReport>;
}

#[async_trait]
pub trait PortControl: Send + Sync {
    async fn list_ports(&self, sandbox: &SandboxId) -> Result<Vec<PortBinding>>;

    async fn connect_port(&self, sandbox: &SandboxId, port: GuestPort) -> Result<PortTarget>;

    async fn expose_port(
        &self,
        sandbox: &SandboxId,
        port: GuestPort,
        spec: PortExposure,
    ) -> Result<PortBinding>;

    async fn unexpose_port(&self, sandbox: &SandboxId, binding: PortBinding) -> Result<()>;

    async fn domain_proxy(&self, sandbox: &SandboxId, spec: DomainProxySpec)
    -> Result<DomainProxy>;
}

#[async_trait]
pub trait EventControl: Send + Sync {
    async fn subscribe_events(&self, filter: EventFilter) -> Result<EventStream>;
}

#[async_trait]
pub trait LogControl: Send + Sync {
    async fn list_logs(
        &self,
        sandbox: Option<&SandboxId>,
        filter: crate::logs::LogFilter,
    ) -> Result<Vec<LogEntry>>;

    async fn stream_logs(
        &self,
        sandbox: Option<&SandboxId>,
        filter: LogFilter,
    ) -> Result<LogStream>;
}

#[async_trait]
pub trait MetricControl: Send + Sync {
    async fn metric_snapshot(
        &self,
        sandbox: Option<&SandboxId>,
        filter: MetricFilter,
    ) -> Result<MetricSnapshot>;
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendInfo {
    pub name: BackendName,
    pub version: Option<String>,
}

impl BackendInfo {
    pub fn new(name: BackendName) -> Self {
        Self {
            name,
            version: None,
        }
    }
}

pub trait LiveSandbox: Send + Sync {
    fn id(&self) -> &SandboxId;

    fn process(&self) -> Option<&dyn ProcessControl> {
        None
    }

    fn filesystem(&self) -> Option<&dyn FilesystemControl> {
        None
    }

    fn ports(&self) -> Option<&dyn PortControl> {
        None
    }

    fn snapshots(&self) -> Option<&dyn SnapshotControl> {
        None
    }
}
