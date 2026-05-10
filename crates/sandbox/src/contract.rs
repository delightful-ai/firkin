use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use base64::Engine as _;
use bytes::Bytes;
use futures_core::Stream;
use futures_util::stream;
use time::OffsetDateTime;
use tokio::sync::mpsc;

use crate::backend::{
    BackendInfo, EventControl, FilesystemControl, LogControl, MetricControl, PortControl,
    ProcessControl, SandboxBackend, SandboxControl, SnapshotControl, TemplateControl,
    WarmPoolControl,
};
use crate::capability::{Capabilities, CapabilityName, CapabilityReason};
use crate::data_plane::{DataPlaneInfo, GuestArch, PreparedEnvdDataPlane, ReservedPort};
use crate::error::{
    BackendFailure, NotFound, ResourceKind, Result, RetryClass, UnsupportedCapability,
};
use crate::event::{EventFilter, EventStream};
use crate::filesystem::{
    CopyOptions, FileEntry, FilePermissions, FileStat, FileType, FilesystemEventStream,
    ListOptions, ReadOptions, SandboxPath, WatchOptions, WriteOptions,
};
use crate::ids::{
    BackendName, ProcessId, ProcessTag, SandboxId, SnapshotId, TemplateId, WarmPoolKey,
};
use crate::logs::{LogEntry, LogFilter, LogStream};
use crate::metrics::{MetricFilter, MetricSnapshot};
use crate::ports::{DomainProxy, GuestPort, HostPort, PortBinding, PortExposure, PortTarget};
use crate::process::{
    Command, CommandMode, CommandOutput, CommandStatus, ProcessEventStream, ProcessInfo,
    ProcessInput, ProcessSelector, ProcessStatus, PtySize, Signal,
};
use crate::runtime::RuntimePreflight;
use crate::sandbox::{
    DeleteOptions, KillSignal, SandboxDeadline, SandboxFilter, SandboxInfo, SandboxSpec,
    SandboxState, StopMode,
};
use crate::snapshot::{
    PauseOptions, PausedSandbox, RestoreOptions, SnapshotExport, SnapshotFilter, SnapshotImport,
    SnapshotInfo, SnapshotIntegrity, SnapshotKind, SnapshotOptions, SnapshotRef,
};
use crate::template::{PreparedTemplate, TemplateInfo, TemplateSpec, TemplateState};
use crate::warm_pool::{
    WarmLease, WarmLeasePolicy, WarmMaintainReport, WarmPoolEntry, WarmPoolSpec, WarmPoolStatus,
    WarmPoolTarget,
};

pub struct BackendContract {
    backend: Arc<dyn SandboxBackend>,
    config: BackendContractConfig,
}

impl BackendContract {
    pub fn new(backend: Arc<dyn SandboxBackend>, config: BackendContractConfig) -> Self {
        Self { backend, config }
    }

    pub async fn run(&self) -> Result<()> {
        run_backend_contract(self.backend.clone(), self.config.clone()).await
    }
}

#[derive(Clone, Debug)]
pub struct BackendContractConfig {
    pub template_reference: String,
}

impl Default for BackendContractConfig {
    fn default() -> Self {
        Self {
            template_reference: "docker.io/library/alpine:latest".to_owned(),
        }
    }
}

pub trait ContractBackendFactory {
    fn make_backend(&self) -> Arc<dyn SandboxBackend>;
}

pub async fn run_backend_contract(
    backend: Arc<dyn SandboxBackend>,
    config: BackendContractConfig,
) -> Result<()> {
    let capabilities = backend.capabilities().await?;
    require_contract(
        capabilities.supports(&CapabilityName::TemplatePrepare),
        "backend must report template.prepare",
    )?;
    require_contract(
        capabilities.supports(&CapabilityName::RuntimeCreate),
        "backend must report runtime.create",
    )?;
    require_contract(
        backend.warm_pool().is_some() == capabilities.supports(&CapabilityName::WarmPoolPrewarm),
        "warm-pool accessor must match warm_pool.prewarm capability",
    )?;

    let prepared = backend
        .templates()
        .prepare_template(TemplateSpec::oci(config.template_reference))
        .await?;

    let sandbox = backend
        .sandboxes()
        .create_sandbox(SandboxSpec::from_template(&prepared))
        .await?;
    require_contract(
        sandbox.state == SandboxState::Running,
        "created sandbox must be running",
    )?;

    let listed = backend
        .sandboxes()
        .list_sandboxes(SandboxFilter::default())
        .await?;
    require_contract(
        listed.iter().any(|item| item.id == sandbox.id),
        "created sandbox must appear in list",
    )?;

    run_process_contract(backend.clone(), &sandbox.id).await?;
    run_retained_shell_contract(backend.clone(), &sandbox.id, &capabilities).await?;
    run_filesystem_contract(backend.clone(), &sandbox.id).await?;
    run_snapshot_contract(backend.clone(), &sandbox.id).await?;

    if backend.warm_pool().is_some() {
        run_warm_pool_contract(backend.clone(), &prepared).await?;
    }

    backend
        .sandboxes()
        .stop_sandbox(&sandbox.id, StopMode::Graceful)
        .await?;
    backend
        .sandboxes()
        .delete_sandbox(&sandbox.id, DeleteOptions::default())
        .await?;
    Ok(())
}

pub async fn run_process_contract(
    backend: Arc<dyn SandboxBackend>,
    sandbox: &SandboxId,
) -> Result<()> {
    let Some(processes) = backend.processes() else {
        return Ok(());
    };
    let output = processes
        .run_process(sandbox, Command::shell("echo contract"))
        .await?;
    require_contract(
        matches!(output.status, CommandStatus::Exited(exit) if exit.is_success()),
        "foreground command success must report zero exit",
    )?;

    let output = processes
        .run_process(sandbox, Command::shell("false"))
        .await?;
    require_contract(
        matches!(output.status, CommandStatus::Exited(exit) if !exit.is_success()),
        "foreground command failure must report nonzero exit",
    )?;
    Ok(())
}

pub async fn run_retained_shell_contract(
    backend: Arc<dyn SandboxBackend>,
    sandbox: &SandboxId,
    capabilities: &Capabilities,
) -> Result<()> {
    if !capabilities.supports(&CapabilityName::ProcessStream) {
        return Ok(());
    }
    require_contract(
        capabilities.supports(&CapabilityName::ProcessStdin),
        "retained shell stream capability requires process stdin capability",
    )?;
    require_contract(
        capabilities.supports(&CapabilityName::ProcessSignal),
        "retained shell stream capability requires process signal capability",
    )?;

    let client = crate::process::ProcessClient::new(backend, sandbox.clone());
    let pool = client.shell_pool(2).await?;
    require_contract(
        pool.len() == 2,
        "retained shell pool must expose requested size",
    )?;
    let slots = pool.slots();
    require_contract(
        slots.len() == 2,
        "retained shell pool slots must expose stable shell assignments",
    )?;
    require_contract(
        pool.slot(2).is_none(),
        "retained shell pool indexed slots must reject out-of-range access",
    )?;
    let first_slot = pool
        .slot(0)
        .ok_or_else(|| contract_failure("retained shell pool indexed slot 0 must exist"))?;
    let first = first_slot.run(Command::shell("printf slot-0")).await?;
    require_contract(
        first.stdout == Bytes::from_static(b"slot-0"),
        "retained shell slot 0 must dispatch command output",
    )?;
    let second = slots[1].run(Command::argv("printf", ["slot-1"])).await?;
    require_contract(
        second.stdout == Bytes::from_static(b"slot-1"),
        "retained shell slot 1 must dispatch argv command output",
    )?;
    let pooled = pool.run(Command::shell("printf pool-run")).await?;
    require_contract(
        pooled.stdout == Bytes::from_static(b"pool-run"),
        "retained shell pool run must dispatch through a leased shell",
    )?;
    pool.close().await?;
    Ok(())
}

fn contract_failure(reason: &'static str) -> crate::error::Error {
    BackendFailure {
        operation: "run sandbox backend contract",
        backend: BackendName::new("contract")
            .unwrap_or_else(|_| unreachable!("static backend name is valid")),
        reason: reason.to_owned(),
        retry: RetryClass::NotRetryable,
    }
    .into()
}

pub async fn run_filesystem_contract(
    backend: Arc<dyn SandboxBackend>,
    sandbox: &SandboxId,
) -> Result<()> {
    let Some(filesystems) = backend.filesystems() else {
        return Ok(());
    };
    let path = SandboxPath::new("/work/contract.txt")?;
    filesystems
        .write_file(
            sandbox,
            path.clone(),
            Bytes::from_static(b"contract"),
            WriteOptions::default(),
        )
        .await?;
    let data = filesystems
        .read_file(sandbox, path.clone(), ReadOptions)
        .await?;
    require_contract(
        data == Bytes::from_static(b"contract"),
        "filesystem read must return written bytes",
    )?;
    filesystems.stat_file(sandbox, path.clone()).await?;
    filesystems.remove_file(sandbox, path).await?;
    Ok(())
}

pub async fn run_snapshot_contract(
    backend: Arc<dyn SandboxBackend>,
    sandbox: &SandboxId,
) -> Result<()> {
    let snapshot = backend
        .snapshots()
        .capture_snapshot(sandbox, SnapshotOptions::named("contract"))
        .await?;
    backend.snapshots().get_snapshot(snapshot.id()).await?;
    backend.snapshots().delete_snapshot(snapshot.id()).await?;

    let pause = backend
        .snapshots()
        .pause_sandbox(sandbox, PauseOptions::default())
        .await;
    if let Err(crate::error::Error::UnsupportedCapability(_)) = pause {
        return Ok(());
    }
    pause.map(|_| ())
}

pub async fn run_warm_pool_contract(
    backend: Arc<dyn SandboxBackend>,
    template: &PreparedTemplate,
) -> Result<()> {
    let Some(warm_pool) = backend.warm_pool() else {
        return Ok(());
    };
    warm_pool.prewarm(template, WarmPoolSpec::depth(1)).await?;
    let status = warm_pool.status().await?;
    require_contract(
        !status.entries.is_empty(),
        "warm-pool status must include prewarmed entry",
    )?;
    Ok(())
}

fn require_contract(condition: bool, reason: &'static str) -> Result<()> {
    if condition {
        return Ok(());
    }
    Err(BackendFailure {
        operation: "run sandbox backend contract",
        backend: BackendName::new("contract")?,
        reason: reason.to_owned(),
        retry: RetryClass::NotRetryable,
    }
    .into())
}

#[derive(Clone, Default)]
pub struct FakeBackend {
    state: Arc<Mutex<FakeState>>,
}

pub type RecordingBackend = FakeBackend;

#[derive(Default)]
struct FakeState {
    next_template: u64,
    next_sandbox: u64,
    next_snapshot: u64,
    next_process: u64,
    templates: BTreeMap<TemplateId, PreparedTemplate>,
    sandboxes: BTreeMap<SandboxId, SandboxInfo>,
    snapshots: BTreeMap<SnapshotId, SnapshotInfo>,
    files: BTreeMap<(SandboxId, SandboxPath), Bytes>,
    process_streams:
        BTreeMap<ProcessTag, mpsc::UnboundedSender<Result<crate::process::ProcessEvent>>>,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().expect("fake backend mutex poisoned")
    }

    fn unsupported(operation: &'static str, capability: CapabilityName) -> crate::error::Error {
        UnsupportedCapability::new(
            operation,
            capability,
            CapabilityReason::Permanent {
                detail: "fake backend intentionally does not implement this capability".to_owned(),
            },
        )
        .into()
    }
}

#[async_trait]
impl SandboxBackend for FakeBackend {
    async fn capabilities(&self) -> Result<Capabilities> {
        Ok(Capabilities::new()
            .with_supported(CapabilityName::TemplatePrepare)
            .with_supported(CapabilityName::RuntimeCreate)
            .with_supported(CapabilityName::RuntimeAttach)
            .with_supported(CapabilityName::RuntimeList)
            .with_supported(CapabilityName::SandboxStop)
            .with_supported(CapabilityName::SandboxKill)
            .with_supported(CapabilityName::SandboxDelete)
            .with_supported(CapabilityName::SnapshotCapture)
            .with_supported(CapabilityName::SnapshotDelete)
            .with_supported(CapabilityName::ProcessRun)
            .with_supported(CapabilityName::ProcessStart)
            .with_supported(CapabilityName::ProcessStream)
            .with_supported(CapabilityName::ProcessStdin)
            .with_supported(CapabilityName::ProcessSignal)
            .with_supported(CapabilityName::FilesystemRead)
            .with_supported(CapabilityName::FilesystemWrite)
            .with_supported(CapabilityName::FilesystemList)
            .with_supported(CapabilityName::PortsConnect)
            .with_supported(CapabilityName::WarmPoolPrewarm)
            .with_supported(CapabilityName::WarmPoolCheckout))
    }

    async fn preflight(&self) -> Result<RuntimePreflight> {
        Ok(RuntimePreflight::ready())
    }

    async fn info(&self) -> Result<BackendInfo> {
        Ok(BackendInfo::new(BackendName::new("fake")?))
    }

    fn templates(&self) -> &dyn TemplateControl {
        self
    }

    fn sandboxes(&self) -> &dyn SandboxControl {
        self
    }

    fn snapshots(&self) -> &dyn SnapshotControl {
        self
    }

    fn processes(&self) -> Option<&dyn ProcessControl> {
        Some(self)
    }

    fn filesystems(&self) -> Option<&dyn FilesystemControl> {
        Some(self)
    }

    fn ports(&self) -> Option<&dyn PortControl> {
        Some(self)
    }

    fn warm_pool(&self) -> Option<&dyn crate::backend::WarmPoolControl> {
        Some(self)
    }

    fn events(&self) -> Option<&dyn EventControl> {
        Some(self)
    }

    fn logs(&self) -> Option<&dyn LogControl> {
        Some(self)
    }

    fn metrics(&self) -> Option<&dyn MetricControl> {
        Some(self)
    }
}

#[async_trait]
impl TemplateControl for FakeBackend {
    async fn prepare_template(&self, spec: TemplateSpec) -> Result<PreparedTemplate> {
        let mut state = self.state();
        state.next_template += 1;
        let id = spec
            .id_ref()
            .cloned()
            .unwrap_or(TemplateId::new(format!("tmpl_{}", state.next_template))?);
        let data_plane = match spec.data_plane_ref() {
            crate::data_plane::DataPlaneSpec::None => DataPlaneInfo::None,
            crate::data_plane::DataPlaneSpec::Envd(_) => {
                DataPlaneInfo::Envd(PreparedEnvdDataPlane {
                    version: "fake-envd".to_owned(),
                    commit: Some("fake".to_owned()),
                    sha256: "sha256:fake".to_owned(),
                    arch: GuestArch::Aarch64,
                    port: ReservedPort::ENVD_DEFAULT,
                    startup: crate::data_plane::EnvdStartup::Supervised,
                    init_mode: crate::data_plane::EnvdInitMode::NonFirecracker,
                    default_user: None,
                    health: crate::data_plane::EnvdHealthProbe::default(),
                    health_checked_at: OffsetDateTime::now_utc(),
                })
            }
        };
        let prepared = PreparedTemplate::new(id.clone(), spec.source().clone(), data_plane);
        state.templates.insert(id, prepared.clone());
        Ok(prepared)
    }

    async fn get_template(&self, id: &TemplateId) -> Result<TemplateInfo> {
        let state = self.state();
        let template = state
            .templates
            .get(id)
            .ok_or_else(|| NotFound::new("get template", ResourceKind::Template, id.to_string()))?;
        Ok(TemplateInfo {
            id: template.id().clone(),
            state: TemplateState::Ready,
            source: template.source().clone(),
            data_plane: template.data_plane().clone(),
            prepared_at: Some(OffsetDateTime::now_utc()),
        })
    }

    async fn list_templates(&self) -> Result<Vec<TemplateInfo>> {
        let ids = self.state().templates.keys().cloned().collect::<Vec<_>>();
        let mut infos = Vec::with_capacity(ids.len());
        for id in ids {
            infos.push(self.get_template(&id).await?);
        }
        Ok(infos)
    }

    async fn delete_template(&self, id: &TemplateId) -> Result<()> {
        self.state().templates.remove(id);
        Ok(())
    }
}

#[async_trait]
impl SandboxControl for FakeBackend {
    async fn create_sandbox(&self, spec: SandboxSpec) -> Result<SandboxInfo> {
        let mut state = self.state();
        state.next_sandbox += 1;
        let id = SandboxId::new(format!("sbx_{}", state.next_sandbox))?;
        let mut info = SandboxInfo::running(id.clone());
        match spec {
            SandboxSpec::Template {
                template_id,
                resources,
                deadline,
                metadata,
                ..
            } => {
                info.template_id = Some(template_id);
                info.resources = resources;
                info.deadline = deadline;
                info.metadata = metadata;
            }
            SandboxSpec::Snapshot {
                snapshot,
                resources,
                deadline,
                metadata,
                ..
            } => {
                info.snapshot_id = Some(snapshot.id().clone());
                info.resources = resources;
                info.deadline = deadline;
                info.metadata = metadata;
            }
        }
        state.sandboxes.insert(id, info.clone());
        Ok(info)
    }

    async fn restore_sandbox(
        &self,
        snapshot: SnapshotRef,
        _options: RestoreOptions,
    ) -> Result<SandboxInfo> {
        self.create_sandbox(SandboxSpec::from_snapshot(snapshot))
            .await
    }

    async fn attach_sandbox(
        &self,
        id: &SandboxId,
        _options: crate::sandbox::AttachOptions,
    ) -> Result<SandboxInfo> {
        self.inspect_sandbox(id).await
    }

    async fn inspect_sandbox(&self, id: &SandboxId) -> Result<SandboxInfo> {
        self.state().sandboxes.get(id).cloned().ok_or_else(|| {
            NotFound::new("inspect sandbox", ResourceKind::Sandbox, id.to_string()).into()
        })
    }

    async fn list_sandboxes(&self, filter: SandboxFilter) -> Result<Vec<SandboxInfo>> {
        Ok(self
            .state()
            .sandboxes
            .values()
            .filter(|info| filter.state.is_none_or(|state| state == info.state))
            .cloned()
            .collect())
    }

    async fn stop_sandbox(&self, id: &SandboxId, _mode: StopMode) -> Result<()> {
        let mut state = self.state();
        let info = state
            .sandboxes
            .get_mut(id)
            .ok_or_else(|| NotFound::new("stop sandbox", ResourceKind::Sandbox, id.to_string()))?;
        info.state = SandboxState::Stopped;
        Ok(())
    }

    async fn kill_sandbox(&self, id: &SandboxId, _signal: KillSignal) -> Result<()> {
        self.stop_sandbox(id, StopMode::Graceful).await
    }

    async fn delete_sandbox(
        &self,
        id: &SandboxId,
        _options: crate::sandbox::DeleteOptions,
    ) -> Result<()> {
        self.state().sandboxes.remove(id);
        Ok(())
    }

    async fn update_deadline(&self, id: &SandboxId, deadline: SandboxDeadline) -> Result<()> {
        let mut state = self.state();
        let info = state.sandboxes.get_mut(id).ok_or_else(|| {
            NotFound::new(
                "update sandbox deadline",
                ResourceKind::Sandbox,
                id.to_string(),
            )
        })?;
        info.deadline = Some(deadline);
        Ok(())
    }
}

#[async_trait]
impl ProcessControl for FakeBackend {
    async fn run_process(&self, _sandbox: &SandboxId, command: Command) -> Result<CommandOutput> {
        let output = match command.mode() {
            CommandMode::Shell(command) if command.trim() == "false" => CommandOutput {
                status: crate::process::CommandStatus::Exited(
                    crate::process::CommandExit::nonzero(1),
                ),
                stdout: Bytes::new(),
                stderr: Bytes::new(),
            },
            CommandMode::Shell(command) if command.trim() == "echo contract" => {
                CommandOutput::success(Bytes::from_static(b"contract\n"))
            }
            CommandMode::Shell(command) => CommandOutput::success(command.clone()),
            CommandMode::Argv { program, .. } => CommandOutput::success(program.clone()),
        };
        Ok(output)
    }

    async fn start_process(&self, _sandbox: &SandboxId, _command: Command) -> Result<ProcessInfo> {
        let mut state = self.state();
        state.next_process += 1;
        Ok(ProcessInfo {
            id: ProcessId::new(format!("proc_{}", state.next_process))?,
            tag: None,
            status: ProcessStatus::Running,
        })
    }

    async fn start_process_stream(
        &self,
        _sandbox: &SandboxId,
        command: Command,
    ) -> Result<ProcessEventStream> {
        let backend = BackendName::new("fake")?;
        let tag = command.tag_ref().cloned().ok_or_else(|| BackendFailure {
            operation: "start process stream",
            backend: backend.clone(),
            reason: "fake retained stream requires a process tag".to_owned(),
            retry: RetryClass::NotRetryable,
        })?;
        let mut state = self.state();
        state.next_process += 1;
        let process = ProcessInfo {
            id: ProcessId::new(format!("proc_{}", state.next_process))?,
            tag: Some(tag.clone()),
            status: ProcessStatus::Running,
        };
        let (sender, receiver) = mpsc::unbounded_channel();
        sender
            .send(Ok(crate::process::ProcessEvent::Started(process)))
            .map_err(|error| BackendFailure {
                operation: "start process stream",
                backend,
                reason: format!("failed to seed fake process stream: {error}"),
                retry: RetryClass::NotRetryable,
            })?;
        state.process_streams.insert(tag, sender);
        Ok(Box::pin(stream::unfold(receiver, |mut receiver| async {
            receiver.recv().await.map(|event| (event, receiver))
        })))
    }

    async fn list_processes(&self, _sandbox: &SandboxId) -> Result<Vec<ProcessInfo>> {
        Ok(Vec::new())
    }

    async fn connect_process(
        &self,
        _sandbox: &SandboxId,
        selector: ProcessSelector,
    ) -> Result<ProcessInfo> {
        let id = match selector {
            ProcessSelector::Id(id) => id,
            ProcessSelector::Tag(tag) => ProcessId::new(tag.as_str())?,
        };
        Ok(ProcessInfo {
            id,
            tag: None,
            status: ProcessStatus::Running,
        })
    }

    async fn signal_process(
        &self,
        _sandbox: &SandboxId,
        _selector: ProcessSelector,
        _signal: Signal,
    ) -> Result<()> {
        Ok(())
    }

    async fn send_process_input(
        &self,
        _sandbox: &SandboxId,
        selector: ProcessSelector,
        input: ProcessInput,
    ) -> Result<()> {
        let ProcessSelector::Tag(tag) = selector else {
            return Ok(());
        };
        let ProcessInput::Bytes(bytes) = input else {
            return Ok(());
        };
        let sender = self.state().process_streams.get(&tag).cloned();
        if let Some(sender) = sender {
            let backend = BackendName::new("fake")?;
            for event in fake_retained_shell_events(&bytes) {
                sender.send(Ok(event)).map_err(|error| BackendFailure {
                    operation: "send process stdin",
                    backend: backend.clone(),
                    reason: format!("failed to write fake retained shell event: {error}"),
                    retry: RetryClass::NotRetryable,
                })?;
            }
        }
        Ok(())
    }

    async fn close_process_stdin(
        &self,
        _sandbox: &SandboxId,
        _selector: ProcessSelector,
    ) -> Result<()> {
        Ok(())
    }

    async fn resize_process_pty(
        &self,
        _sandbox: &SandboxId,
        _selector: ProcessSelector,
        _size: PtySize,
    ) -> Result<()> {
        Ok(())
    }

    async fn wait_process(
        &self,
        _sandbox: &SandboxId,
        _selector: ProcessSelector,
    ) -> Result<CommandOutput> {
        Ok(CommandOutput::success(Bytes::new()))
    }
}

#[async_trait]
impl FilesystemControl for FakeBackend {
    async fn read_file(
        &self,
        sandbox: &SandboxId,
        path: SandboxPath,
        _options: ReadOptions,
    ) -> Result<Bytes> {
        self.state()
            .files
            .get(&(sandbox.clone(), path.clone()))
            .cloned()
            .ok_or_else(|| NotFound::new("read file", ResourceKind::File, path.to_string()).into())
    }

    async fn write_file(
        &self,
        sandbox: &SandboxId,
        path: SandboxPath,
        data: Bytes,
        options: WriteOptions,
    ) -> Result<FileEntry> {
        let mut state = self.state();
        let key = (sandbox.clone(), path.clone());
        if options.append {
            let existing = state.files.entry(key).or_default();
            let mut joined = Vec::with_capacity(existing.len() + data.len());
            joined.extend_from_slice(existing);
            joined.extend_from_slice(&data);
            *existing = Bytes::from(joined);
        } else {
            state.files.insert(key, data.clone());
        }
        Ok(FileEntry {
            path,
            file_type: FileType::File,
            size_bytes: data.len() as u64,
        })
    }

    async fn copy_in(
        &self,
        sandbox: &SandboxId,
        _host: &std::path::Path,
        guest: SandboxPath,
        _options: CopyOptions,
    ) -> Result<FileEntry> {
        self.write_file(sandbox, guest, Bytes::new(), WriteOptions::default())
            .await
    }

    async fn copy_out(
        &self,
        _sandbox: &SandboxId,
        _guest: SandboxPath,
        _host: &std::path::Path,
        _options: CopyOptions,
    ) -> Result<()> {
        Ok(())
    }

    async fn list_files(
        &self,
        sandbox: &SandboxId,
        path: SandboxPath,
        _options: ListOptions,
    ) -> Result<Vec<FileEntry>> {
        Ok(self
            .state()
            .files
            .iter()
            .filter(|((file_sandbox, file_path), _)| {
                file_sandbox == sandbox && file_path.as_str().starts_with(path.as_str())
            })
            .map(|((_, file_path), bytes)| FileEntry {
                path: file_path.clone(),
                file_type: FileType::File,
                size_bytes: bytes.len() as u64,
            })
            .collect())
    }

    async fn mkdir(
        &self,
        _sandbox: &SandboxId,
        path: SandboxPath,
        _recursive: bool,
    ) -> Result<FileEntry> {
        Ok(FileEntry {
            path,
            file_type: FileType::Directory,
            size_bytes: 0,
        })
    }

    async fn rename(
        &self,
        sandbox: &SandboxId,
        from: SandboxPath,
        to: SandboxPath,
    ) -> Result<FileEntry> {
        let mut state = self.state();
        if let Some(bytes) = state.files.remove(&(sandbox.clone(), from)) {
            let size_bytes = bytes.len() as u64;
            state.files.insert((sandbox.clone(), to.clone()), bytes);
            Ok(FileEntry {
                path: to,
                file_type: FileType::File,
                size_bytes,
            })
        } else {
            Err(NotFound::new("rename file", ResourceKind::File, to_string_lossy(&to)).into())
        }
    }

    async fn remove_file(&self, sandbox: &SandboxId, path: SandboxPath) -> Result<()> {
        self.state().files.remove(&(sandbox.clone(), path));
        Ok(())
    }

    async fn stat_file(&self, sandbox: &SandboxId, path: SandboxPath) -> Result<FileStat> {
        let data = self.read_file(sandbox, path.clone(), ReadOptions).await?;
        Ok(FileStat {
            entry: FileEntry {
                path,
                file_type: FileType::File,
                size_bytes: data.len() as u64,
            },
            permissions: FilePermissions { mode: 0o644 },
        })
    }

    async fn watch_files(
        &self,
        _sandbox: &SandboxId,
        _path: SandboxPath,
        _options: WatchOptions,
    ) -> Result<FilesystemEventStream> {
        Ok(empty_stream())
    }
}

#[async_trait]
impl SnapshotControl for FakeBackend {
    async fn capture_snapshot(
        &self,
        sandbox: &SandboxId,
        _options: SnapshotOptions,
    ) -> Result<SnapshotRef> {
        let mut state = self.state();
        state.next_snapshot += 1;
        let id = SnapshotId::new(format!("snap_{}", state.next_snapshot))?;
        state.snapshots.insert(
            id.clone(),
            SnapshotInfo {
                id: id.clone(),
                kind: SnapshotKind::Continuation,
                source_sandbox_id: Some(sandbox.clone()),
                source_template_id: None,
                created_at: OffsetDateTime::now_utc(),
                integrity: Some(SnapshotIntegrity {
                    sha256: "sha256:fake".to_owned(),
                    size_bytes: 0,
                }),
            },
        );
        Ok(SnapshotRef::new(id, SnapshotKind::Continuation))
    }

    async fn get_snapshot(&self, id: &SnapshotId) -> Result<SnapshotInfo> {
        self.state().snapshots.get(id).cloned().ok_or_else(|| {
            NotFound::new("get snapshot", ResourceKind::Snapshot, id.to_string()).into()
        })
    }

    async fn list_snapshots(&self, filter: SnapshotFilter) -> Result<Vec<SnapshotInfo>> {
        Ok(self
            .state()
            .snapshots
            .values()
            .filter(|info| filter.kind.is_none_or(|kind| kind == info.kind))
            .cloned()
            .collect())
    }

    async fn delete_snapshot(&self, id: &SnapshotId) -> Result<()> {
        self.state().snapshots.remove(id);
        Ok(())
    }

    async fn export_snapshot(&self, id: &SnapshotId) -> Result<SnapshotExport> {
        let info = self.get_snapshot(id).await?;
        Ok(SnapshotExport {
            id: info.id,
            media_type: "application/vnd.firkin.snapshot.fake".to_owned(),
            bytes: Bytes::new(),
            integrity: info.integrity.unwrap_or(SnapshotIntegrity {
                sha256: "sha256:fake".to_owned(),
                size_bytes: 0,
            }),
        })
    }

    async fn import_snapshot(&self, import: SnapshotImport) -> Result<SnapshotRef> {
        let id = import.id.unwrap_or(SnapshotId::new("snap_imported")?);
        Ok(SnapshotRef::new(id, SnapshotKind::Continuation))
    }

    async fn pause_sandbox(
        &self,
        _sandbox: &SandboxId,
        _options: PauseOptions,
    ) -> Result<PausedSandbox> {
        Err(Self::unsupported(
            "pause sandbox",
            CapabilityName::PauseCapture,
        ))
    }
}

#[async_trait]
impl PortControl for FakeBackend {
    async fn list_ports(&self, _sandbox: &SandboxId) -> Result<Vec<PortBinding>> {
        Ok(Vec::new())
    }

    async fn connect_port(&self, _sandbox: &SandboxId, port: GuestPort) -> Result<PortTarget> {
        Ok(PortTarget::Tcp {
            host: "127.0.0.1".to_owned(),
            port: HostPort::new(port.get())?,
        })
    }

    async fn expose_port(
        &self,
        _sandbox: &SandboxId,
        port: GuestPort,
        spec: PortExposure,
    ) -> Result<PortBinding> {
        Ok(PortBinding {
            guest: port,
            host: spec.host_port,
            protocol: spec.protocol,
        })
    }

    async fn unexpose_port(&self, _sandbox: &SandboxId, _binding: PortBinding) -> Result<()> {
        Ok(())
    }

    async fn domain_proxy(
        &self,
        _sandbox: &SandboxId,
        spec: crate::ports::DomainProxySpec,
    ) -> Result<DomainProxy> {
        Ok(DomainProxy {
            domain: spec.domain,
        })
    }
}

#[async_trait]
impl WarmPoolControl for FakeBackend {
    async fn prewarm(
        &self,
        template: &PreparedTemplate,
        spec: WarmPoolSpec,
    ) -> Result<WarmMaintainReport> {
        let _ = (template, spec);
        Ok(WarmMaintainReport {
            created: 1,
            evicted: 0,
            ready: 1,
        })
    }

    async fn maintain(&self, _targets: Vec<WarmPoolTarget>) -> Result<WarmMaintainReport> {
        Ok(WarmMaintainReport::default())
    }

    async fn status(&self) -> Result<WarmPoolStatus> {
        Ok(WarmPoolStatus {
            entries: vec![WarmPoolEntry {
                key: WarmPoolKey::new("fake")?,
                ready: 1,
                total: 1,
            }],
        })
    }

    async fn checkout(
        &self,
        _template: &PreparedTemplate,
        _policy: WarmLeasePolicy,
    ) -> Result<WarmLease> {
        Ok(WarmLease {
            key: WarmPoolKey::new("fake")?,
            sandbox_id: SandboxId::new("sbx_warm")?,
        })
    }

    async fn evict(&self, _key: WarmPoolKey, count: usize) -> Result<WarmMaintainReport> {
        Ok(WarmMaintainReport {
            created: 0,
            evicted: count,
            ready: 0,
        })
    }
}

#[async_trait]
impl EventControl for FakeBackend {
    async fn subscribe_events(&self, _filter: EventFilter) -> Result<EventStream> {
        Ok(empty_stream())
    }
}

#[async_trait]
impl LogControl for FakeBackend {
    async fn list_logs(
        &self,
        sandbox: Option<&SandboxId>,
        _filter: LogFilter,
    ) -> Result<Vec<LogEntry>> {
        Ok(vec![LogEntry {
            sandbox_id: sandbox.cloned(),
            source: crate::logs::LogSource::Runtime,
            level: crate::logs::LogLevel::Info,
            message: "fake log".to_owned(),
            observed_at: OffsetDateTime::now_utc(),
        }])
    }

    async fn stream_logs(
        &self,
        _sandbox: Option<&SandboxId>,
        _filter: LogFilter,
    ) -> Result<LogStream> {
        Ok(empty_stream())
    }
}

#[async_trait]
impl MetricControl for FakeBackend {
    async fn metric_snapshot(
        &self,
        _sandbox: Option<&SandboxId>,
        _filter: MetricFilter,
    ) -> Result<MetricSnapshot> {
        Ok(MetricSnapshot::default())
    }
}

fn fake_retained_shell_events(script: &[u8]) -> Vec<crate::process::ProcessEvent> {
    let script = String::from_utf8_lossy(script);
    let nonce = script
        .split("FK_STDOUT:")
        .nth(1)
        .and_then(|tail| tail.split(':').next())
        .unwrap_or("missing-nonce");
    let (stdout, status) = if script.contains("slot-0") {
        (b"slot-0".as_slice(), 0)
    } else if script.contains("slot-1") {
        (b"slot-1".as_slice(), 0)
    } else if script.contains("pool-run") {
        (b"pool-run".as_slice(), 0)
    } else if script.contains("false") {
        (b"".as_slice(), 1)
    } else {
        (b"contract-shell".as_slice(), 0)
    };
    let stdout = base64::engine::general_purpose::STANDARD.encode(stdout);
    let stderr = base64::engine::general_purpose::STANDARD.encode([]);
    vec![crate::process::ProcessEvent::Stdout(Bytes::from(format!(
        "\x1eFK_STDOUT:{nonce}:{stdout}\n\x1eFK_STDERR:{nonce}:{stderr}\n\x1eFK_END:{nonce}:{status}\n"
    )))]
}

struct EmptyStream<T> {
    _marker: std::marker::PhantomData<T>,
}

impl<T> Stream for EmptyStream<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(None)
    }
}

fn empty_stream<T: Send + 'static>() -> Pin<Box<dyn Stream<Item = Result<T>> + Send + 'static>> {
    Box::pin(EmptyStream {
        _marker: std::marker::PhantomData,
    })
}

fn to_string_lossy(path: &SandboxPath) -> String {
    path.to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::backend::SandboxBackend;
    use crate::capability::CapabilityName;
    use crate::contract::{BackendContractConfig, FakeBackend, run_backend_contract};
    use crate::error::Error;
    use crate::sandbox::SandboxFilter;
    use crate::snapshot::PauseOptions;

    #[tokio::test]
    async fn fake_backend_passes_contract() {
        let backend: Arc<dyn SandboxBackend> = Arc::new(FakeBackend::new());
        run_backend_contract(backend, BackendContractConfig::default())
            .await
            .expect("contract passes");
    }

    #[tokio::test]
    async fn unsupported_pause_is_structured() {
        let backend = FakeBackend::new();
        let prepared = backend
            .templates()
            .prepare_template(crate::template::TemplateSpec::oci("example"))
            .await
            .expect("template");
        let sandbox = backend
            .sandboxes()
            .create_sandbox(crate::sandbox::SandboxSpec::from_template(&prepared))
            .await
            .expect("sandbox");
        let error = backend
            .snapshots()
            .pause_sandbox(&sandbox.id, PauseOptions::default())
            .await
            .expect_err("pause unsupported");
        assert!(matches!(
            error,
            Error::UnsupportedCapability(ref unsupported)
                if unsupported.capability == CapabilityName::PauseCapture
        ));
        assert_eq!(
            backend
                .sandboxes()
                .list_sandboxes(SandboxFilter::default())
                .await
                .expect("list")
                .len(),
            1
        );
    }
}
