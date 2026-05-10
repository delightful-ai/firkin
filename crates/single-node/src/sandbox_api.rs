#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use firkin_envd::{
    EnvdProcessInput, EnvdProcessSelector, EnvdProcessSignal, EnvdProcessStartRequest,
    EnvdProcessStreamEvent, EnvdPtySize,
};
use firkin_sandbox as sandbox;
use firkin_types::Size;
use futures_util::stream;
use time::OffsetDateTime;

use super::{
    AppleVzLocalRuntimeDriver, CommandRequest, Error, LogStore, PortRegistry, SingleNodeBackend,
    SingleNodeConfig, SingleNodeCreateRequest, SnapshotRecord, StateStore,
};

#[derive(Clone)]
pub struct AppleVzBackend {
    inner: SingleNodeBackend,
    templates: Arc<Mutex<BTreeMap<sandbox::TemplateId, sandbox::PreparedTemplate>>>,
    warm_pools: Arc<Mutex<BTreeMap<sandbox::WarmPoolKey, Vec<sandbox::SandboxId>>>>,
    next_sandbox: Arc<Mutex<u64>>,
}

impl AppleVzBackend {
    #[must_use]
    pub fn from_single_node(inner: SingleNodeBackend) -> Self {
        Self {
            inner,
            templates: Arc::new(Mutex::new(BTreeMap::new())),
            warm_pools: Arc::new(Mutex::new(BTreeMap::new())),
            next_sandbox: Arc::new(Mutex::new(0)),
        }
    }

    #[must_use]
    pub fn from_config(config: SingleNodeConfig) -> Self {
        let snapshot_dir = config.root().join("snapshots");
        let driver = AppleVzLocalRuntimeDriver::with_snapshot_dir(
            "base",
            PortRegistry::default(),
            LogStore::default(),
            snapshot_dir,
        );
        Self::from_single_node(SingleNodeBackend::with_driver_and_state(
            config,
            Arc::new(driver),
            StateStore::new(),
        ))
    }

    #[must_use]
    pub const fn inner(&self) -> &SingleNodeBackend {
        &self.inner
    }

    fn backend_name() -> sandbox::Result<sandbox::BackendName> {
        sandbox::BackendName::new("apple-vz").map_err(Into::into)
    }

    fn map_error(operation: &'static str, error: Error) -> sandbox::Error {
        match error {
            Error::UnsupportedCapability(reason) => sandbox::UnsupportedCapability::new(
                operation,
                sandbox::CapabilityName::RuntimeCreate,
                sandbox::CapabilityReason::Permanent { detail: reason },
            )
            .into(),
            Error::SandboxNotFound(id) => {
                sandbox::NotFound::new(operation, sandbox::ResourceKind::Sandbox, id).into()
            }
            Error::SnapshotNotFound(id) => {
                sandbox::NotFound::new(operation, sandbox::ResourceKind::Snapshot, id).into()
            }
            Error::CapacityRejected(reason) => sandbox::CapacityRejected {
                operation,
                backend: Self::backend_name()
                    .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
                reason,
                retry: sandbox::RetryClass::Unknown,
            }
            .into(),
            Error::DiskPressure(reason) => sandbox::CapacityRejected {
                operation,
                backend: Self::backend_name()
                    .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
                reason,
                retry: sandbox::RetryClass::Retryable,
            }
            .into(),
            Error::Conflict(id) => sandbox::AlreadyExists {
                operation,
                resource: sandbox::ResourceKind::Sandbox,
                id,
            }
            .into(),
            other => sandbox::BackendFailure {
                operation,
                backend: Self::backend_name()
                    .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
                reason: other.to_string(),
                retry: sandbox::RetryClass::Unknown,
            }
            .into(),
        }
    }

    fn next_sandbox_id(&self) -> sandbox::Result<sandbox::SandboxId> {
        let mut next = self
            .next_sandbox
            .lock()
            .map_err(|_| sandbox::BackendFailure {
                operation: "allocate sandbox id",
                backend: Self::backend_name()
                    .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
                reason: "sandbox id lock poisoned".to_owned(),
                retry: sandbox::RetryClass::Unknown,
            })?;
        *next += 1;
        sandbox::SandboxId::new(format!("sbx_firkin_{next}")).map_err(Into::into)
    }
}

#[async_trait]
impl sandbox::SandboxBackend for AppleVzBackend {
    async fn capabilities(&self) -> sandbox::Result<sandbox::Capabilities> {
        let unsupported = sandbox::CapabilityReason::Permanent {
            detail: "Apple/VZ single-node adapter does not implement this capability yet"
                .to_owned(),
        };
        Ok(sandbox::Capabilities::all_unsupported(&unsupported)
            .with_supported(sandbox::CapabilityName::TemplatePrepare)
            .with_supported(sandbox::CapabilityName::TemplateDataPlaneNone)
            .with_supported(sandbox::CapabilityName::RuntimeCreate)
            .with_supported(sandbox::CapabilityName::RuntimeAttach)
            .with_supported(sandbox::CapabilityName::RuntimeList)
            .with_supported(sandbox::CapabilityName::RuntimeDeadline)
            .with_supported(sandbox::CapabilityName::SandboxStop)
            .with_supported(sandbox::CapabilityName::SandboxKill)
            .with_supported(sandbox::CapabilityName::SandboxDelete)
            .with_supported(sandbox::CapabilityName::SnapshotCapture)
            .with_supported(sandbox::CapabilityName::SnapshotDelete)
            .with_supported(sandbox::CapabilityName::ProcessRun)
            .with_supported(sandbox::CapabilityName::ProcessStart)
            .with_supported(sandbox::CapabilityName::ProcessStream)
            .with_supported(sandbox::CapabilityName::ProcessStdin)
            .with_supported(sandbox::CapabilityName::ProcessSignal)
            .with_supported(sandbox::CapabilityName::FilesystemRead)
            .with_supported(sandbox::CapabilityName::FilesystemWrite)
            .with_supported(sandbox::CapabilityName::FilesystemCopyIn)
            .with_supported(sandbox::CapabilityName::FilesystemCopyOut)
            .with_supported(sandbox::CapabilityName::FilesystemList)
            .with_supported(sandbox::CapabilityName::WarmPoolPrewarm)
            .with_supported(sandbox::CapabilityName::WarmPoolCheckout))
    }

    async fn preflight(&self) -> sandbox::Result<sandbox::RuntimePreflight> {
        Ok(sandbox::RuntimePreflight::ready())
    }

    async fn info(&self) -> sandbox::Result<sandbox::BackendInfo> {
        Ok(sandbox::BackendInfo::new(Self::backend_name()?))
    }

    fn templates(&self) -> &dyn sandbox::TemplateControl {
        self
    }

    fn sandboxes(&self) -> &dyn sandbox::SandboxControl {
        self
    }

    fn snapshots(&self) -> &dyn sandbox::SnapshotControl {
        self
    }

    fn processes(&self) -> Option<&dyn sandbox::ProcessControl> {
        Some(self)
    }

    fn filesystems(&self) -> Option<&dyn sandbox::FilesystemControl> {
        Some(self)
    }

    fn warm_pool(&self) -> Option<&dyn sandbox::WarmPoolControl> {
        Some(self)
    }
}

#[async_trait]
impl sandbox::TemplateControl for AppleVzBackend {
    async fn prepare_template(
        &self,
        spec: sandbox::TemplateSpec,
    ) -> sandbox::Result<sandbox::PreparedTemplate> {
        match spec.data_plane_ref() {
            sandbox::DataPlaneSpec::None => {}
            sandbox::DataPlaneSpec::Envd(envd)
                if envd.provisioning() == sandbox::DataPlaneProvisioning::Inject =>
            {
                return Err(sandbox::TemplatePrepareFailure::EnvdMissing {
                    reference: source_reference(spec.source()),
                }
                .into());
            }
            sandbox::DataPlaneSpec::Envd(_) => {
                return Err(sandbox::TemplatePrepareFailure::UnsupportedImageConfig {
                    reason: "single-node Apple/VZ adapter cannot verify preinstalled envd yet"
                        .to_owned(),
                }
                .into());
            }
        }

        let id =
            spec.id_ref()
                .cloned()
                .unwrap_or(sandbox::TemplateId::new(template_id_from_source(
                    spec.source(),
                ))?);
        let prepared = sandbox::PreparedTemplate::new(
            id.clone(),
            spec.source().clone(),
            sandbox::DataPlaneInfo::None,
        );
        self.templates
            .lock()
            .map_err(|_| sandbox::BackendFailure {
                operation: "prepare template",
                backend: Self::backend_name()
                    .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
                reason: "template registry lock poisoned".to_owned(),
                retry: sandbox::RetryClass::Unknown,
            })?
            .insert(id, prepared.clone());
        Ok(prepared)
    }

    async fn get_template(
        &self,
        id: &sandbox::TemplateId,
    ) -> sandbox::Result<sandbox::TemplateInfo> {
        let templates = self.templates.lock().map_err(|_| sandbox::BackendFailure {
            operation: "get template",
            backend: Self::backend_name()
                .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
            reason: "template registry lock poisoned".to_owned(),
            retry: sandbox::RetryClass::Unknown,
        })?;
        let template = templates.get(id).ok_or_else(|| {
            sandbox::NotFound::new(
                "get template",
                sandbox::ResourceKind::Template,
                id.to_string(),
            )
        })?;
        Ok(sandbox::TemplateInfo {
            id: template.id().clone(),
            state: sandbox::TemplateState::Ready,
            source: template.source().clone(),
            data_plane: template.data_plane().clone(),
            prepared_at: Some(OffsetDateTime::now_utc()),
        })
    }

    async fn list_templates(&self) -> sandbox::Result<Vec<sandbox::TemplateInfo>> {
        let ids = self
            .templates
            .lock()
            .map_err(|_| sandbox::BackendFailure {
                operation: "list templates",
                backend: Self::backend_name()
                    .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
                reason: "template registry lock poisoned".to_owned(),
                retry: sandbox::RetryClass::Unknown,
            })?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let mut templates = Vec::with_capacity(ids.len());
        for id in ids {
            templates.push(self.get_template(&id).await?);
        }
        Ok(templates)
    }

    async fn delete_template(&self, id: &sandbox::TemplateId) -> sandbox::Result<()> {
        self.templates
            .lock()
            .map_err(|_| sandbox::BackendFailure {
                operation: "delete template",
                backend: Self::backend_name()
                    .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
                reason: "template registry lock poisoned".to_owned(),
                retry: sandbox::RetryClass::Unknown,
            })?
            .remove(id);
        Ok(())
    }
}

#[async_trait]
impl sandbox::SandboxControl for AppleVzBackend {
    async fn create_sandbox(
        &self,
        spec: sandbox::SandboxSpec,
    ) -> sandbox::Result<sandbox::SandboxInfo> {
        let sandbox_id = self.next_sandbox_id()?;
        let request = self.create_request_from_spec(&sandbox_id, spec)?;
        self.inner
            .create(request)
            .await
            .map_err(|error| Self::map_error("create sandbox", error))?;
        self.inspect_sandbox(&sandbox_id).await
    }

    async fn restore_sandbox(
        &self,
        snapshot: sandbox::SnapshotRef,
        _options: sandbox::RestoreOptions,
    ) -> sandbox::Result<sandbox::SandboxInfo> {
        let sandbox_id = self.next_sandbox_id()?;
        let record = find_snapshot(self.inner.snapshot_records(), snapshot.id())?;
        let request = SingleNodeCreateRequest::new(
            sandbox_id.to_string(),
            record
                .template_metadata
                .start_command()
                .unwrap_or("snapshot")
                .to_owned(),
            super::SandboxResources::new(2, Size::gib(8)),
        );
        self.inner
            .restore(request, record)
            .await
            .map_err(|error| Self::map_error("restore sandbox", error))?;
        self.inspect_sandbox(&sandbox_id).await
    }

    async fn attach_sandbox(
        &self,
        id: &sandbox::SandboxId,
        _options: sandbox::AttachOptions,
    ) -> sandbox::Result<sandbox::SandboxInfo> {
        self.inspect_sandbox(id).await
    }

    async fn inspect_sandbox(
        &self,
        id: &sandbox::SandboxId,
    ) -> sandbox::Result<sandbox::SandboxInfo> {
        let record = self
            .inner
            .active_records()
            .map_err(|error| Self::map_error("inspect sandbox", error))?
            .into_iter()
            .find(|record| record.sandbox_id == id.as_str())
            .ok_or_else(|| {
                sandbox::NotFound::new(
                    "inspect sandbox",
                    sandbox::ResourceKind::Sandbox,
                    id.to_string(),
                )
            })?;
        self.active_to_info(record)
    }

    async fn list_sandboxes(
        &self,
        filter: sandbox::SandboxFilter,
    ) -> sandbox::Result<Vec<sandbox::SandboxInfo>> {
        self.inner
            .active_records()
            .map_err(|error| Self::map_error("list sandboxes", error))?
            .into_iter()
            .map(|record| self.active_to_info(record))
            .filter(|result| {
                result.as_ref().map_or(true, |info| {
                    filter.state.is_none_or(|state| state == info.state)
                })
            })
            .collect()
    }

    async fn stop_sandbox(
        &self,
        id: &sandbox::SandboxId,
        _mode: sandbox::StopMode,
    ) -> sandbox::Result<()> {
        self.inner
            .delete(id.as_str())
            .await
            .map_err(|error| Self::map_error("stop sandbox", error))
    }

    async fn kill_sandbox(
        &self,
        id: &sandbox::SandboxId,
        _signal: sandbox::KillSignal,
    ) -> sandbox::Result<()> {
        self.stop_sandbox(id, sandbox::StopMode::Graceful).await
    }

    async fn delete_sandbox(
        &self,
        id: &sandbox::SandboxId,
        _options: sandbox::DeleteOptions,
    ) -> sandbox::Result<()> {
        self.inner
            .delete(id.as_str())
            .await
            .map_err(|error| Self::map_error("delete sandbox", error))
    }

    async fn update_deadline(
        &self,
        id: &sandbox::SandboxId,
        deadline: sandbox::SandboxDeadline,
    ) -> sandbox::Result<()> {
        let sandbox::SandboxDeadline::Timeout(timeout) = deadline else {
            return Err(sandbox::InvalidSpec::new(
                "update sandbox deadline",
                sandbox::InvalidSpecReason::InvalidTimeout,
            )
            .into());
        };
        self.inner
            .update_deadline(id.as_str(), timeout)
            .map_err(|error| Self::map_error("update sandbox deadline", error))
    }
}

#[async_trait]
impl sandbox::ProcessControl for AppleVzBackend {
    async fn run_process(
        &self,
        sandbox_id: &sandbox::SandboxId,
        command: sandbox::Command,
    ) -> sandbox::Result<sandbox::CommandOutput> {
        let request = command_request(command)?;
        self.inner
            .run_command(sandbox_id.as_str(), request)
            .await
            .map(command_output)
            .map_err(|error| Self::map_error("run process", error))
    }

    async fn start_process(
        &self,
        sandbox_id: &sandbox::SandboxId,
        command: sandbox::Command,
    ) -> sandbox::Result<sandbox::ProcessInfo> {
        let mut stream = self
            .inner
            .start_process_stream(sandbox_id.as_str(), process_start_request(command)?)
            .await
            .map_err(|error| Self::map_error("start process", error))?;
        while let Some(event) = stream.recv().await {
            match event.map_err(|error| process_backend_failure("start process", error))? {
                EnvdProcessStreamEvent::Start { pid } => return process_info_from_pid(pid),
                EnvdProcessStreamEvent::Stdout(_)
                | EnvdProcessStreamEvent::Stderr(_)
                | EnvdProcessStreamEvent::Pty(_) => {}
                EnvdProcessStreamEvent::End { exit_code, .. } => {
                    return Err(sandbox::ProcessFailure {
                        operation: "start process",
                        sandbox_id: Some(sandbox_id.clone()),
                        process_id: None,
                        reason: format!("process exited before start event: {exit_code}"),
                        retry: sandbox::RetryClass::NotRetryable,
                    }
                    .into());
                }
            }
        }
        Err(sandbox::ProcessFailure {
            operation: "start process",
            sandbox_id: Some(sandbox_id.clone()),
            process_id: None,
            reason: "process stream ended before start event".to_owned(),
            retry: sandbox::RetryClass::NotRetryable,
        }
        .into())
    }

    async fn start_process_stream(
        &self,
        sandbox_id: &sandbox::SandboxId,
        command: sandbox::Command,
    ) -> sandbox::Result<sandbox::ProcessEventStream> {
        let stream = self
            .inner
            .start_process_stream(sandbox_id.as_str(), process_start_request(command)?)
            .await
            .map_err(|error| Self::map_error("stream process", error))?;
        Ok(Box::pin(stream::unfold(stream, |mut stream| async {
            stream.recv().await.map(|event| {
                (
                    event
                        .map_err(|error| process_backend_failure("stream process", error))
                        .and_then(process_event),
                    stream,
                )
            })
        })))
    }

    async fn list_processes(
        &self,
        sandbox_id: &sandbox::SandboxId,
    ) -> sandbox::Result<Vec<sandbox::ProcessInfo>> {
        self.inner
            .list_processes(sandbox_id.as_str())
            .await
            .map_err(|error| Self::map_error("list processes", error))?
            .into_iter()
            .map(process_info)
            .collect()
    }

    async fn connect_process(
        &self,
        sandbox_id: &sandbox::SandboxId,
        selector: sandbox::ProcessSelector,
    ) -> sandbox::Result<sandbox::ProcessInfo> {
        let output = self
            .inner
            .connect_process(sandbox_id.as_str(), process_selector(selector))
            .await
            .map_err(|error| Self::map_error("connect process", error))?;
        process_info_from_pid(output.pid)
    }

    async fn signal_process(
        &self,
        sandbox_id: &sandbox::SandboxId,
        selector: sandbox::ProcessSelector,
        signal: sandbox::Signal,
    ) -> sandbox::Result<()> {
        self.inner
            .signal_process(
                sandbox_id.as_str(),
                process_selector(selector),
                process_signal(signal),
            )
            .await
            .map_err(|error| Self::map_error("signal process", error))
    }

    async fn send_process_input(
        &self,
        sandbox_id: &sandbox::SandboxId,
        selector: sandbox::ProcessSelector,
        input: sandbox::ProcessInput,
    ) -> sandbox::Result<()> {
        self.inner
            .send_process_input(
                sandbox_id.as_str(),
                process_selector(selector),
                process_input(input),
            )
            .await
            .map_err(|error| Self::map_error("process stdin", error))
    }

    async fn close_process_stdin(
        &self,
        sandbox_id: &sandbox::SandboxId,
        selector: sandbox::ProcessSelector,
    ) -> sandbox::Result<()> {
        self.inner
            .close_process_stdin(sandbox_id.as_str(), process_selector(selector))
            .await
            .map_err(|error| Self::map_error("close process stdin", error))
    }

    async fn resize_process_pty(
        &self,
        sandbox_id: &sandbox::SandboxId,
        selector: sandbox::ProcessSelector,
        size: sandbox::PtySize,
    ) -> sandbox::Result<()> {
        self.inner
            .resize_process_pty(
                sandbox_id.as_str(),
                process_selector(selector),
                Some(EnvdPtySize {
                    rows: u32::from(size.rows),
                    cols: u32::from(size.cols),
                }),
            )
            .await
            .map_err(|error| Self::map_error("resize pty", error))
    }

    async fn wait_process(
        &self,
        sandbox_id: &sandbox::SandboxId,
        selector: sandbox::ProcessSelector,
    ) -> sandbox::Result<sandbox::CommandOutput> {
        self.inner
            .connect_process(sandbox_id.as_str(), process_selector(selector))
            .await
            .map(command_output_from_envd)
            .map_err(|error| Self::map_error("wait process", error))
    }
}

#[async_trait]
impl sandbox::FilesystemControl for AppleVzBackend {
    async fn read_file(
        &self,
        sandbox_id: &sandbox::SandboxId,
        path: sandbox::SandboxPath,
        _options: sandbox::ReadOptions,
    ) -> sandbox::Result<Bytes> {
        let output = self
            .run_filesystem_command(
                "read file",
                sandbox_id,
                Some(&path),
                CommandRequest::new(
                    r#"if [ ! -f "$FIRKIN_FS_PATH" ]; then exit 44; fi
cat -- "$FIRKIN_FS_PATH""#,
                )
                .with_env("FIRKIN_FS_PATH", path.to_string()),
            )
            .await?;
        Ok(Bytes::from(output.stdout))
    }

    async fn write_file(
        &self,
        sandbox_id: &sandbox::SandboxId,
        path: sandbox::SandboxPath,
        data: Bytes,
        options: sandbox::WriteOptions,
    ) -> sandbox::Result<sandbox::FileEntry> {
        self.run_filesystem_command(
            "write file",
            sandbox_id,
            Some(&path),
            CommandRequest::new(
                r#"mkdir -p -- "$(dirname -- "$FIRKIN_FS_PATH")"
if [ "$FIRKIN_FS_APPEND" = "1" ]; then
  cat >> "$FIRKIN_FS_PATH"
else
  cat > "$FIRKIN_FS_PATH"
fi"#,
            )
            .with_env("FIRKIN_FS_PATH", path.to_string())
            .with_env("FIRKIN_FS_APPEND", if options.append { "1" } else { "0" })
            .with_stdin(data.to_vec()),
        )
        .await?;
        self.stat_file(sandbox_id, path)
            .await
            .map(|stat| stat.entry)
    }

    async fn copy_in(
        &self,
        sandbox_id: &sandbox::SandboxId,
        host: &Path,
        guest: sandbox::SandboxPath,
        _options: sandbox::CopyOptions,
    ) -> sandbox::Result<sandbox::FileEntry> {
        let data = tokio::fs::read(host)
            .await
            .map_err(|error| sandbox::IoFailure {
                operation: "copy into sandbox",
                resource: host.display().to_string(),
                reason: error.to_string(),
                retry: sandbox::RetryClass::Unknown,
            })?;
        self.write_file(
            sandbox_id,
            guest,
            Bytes::from(data),
            sandbox::WriteOptions::default(),
        )
        .await
    }

    async fn copy_out(
        &self,
        sandbox_id: &sandbox::SandboxId,
        guest: sandbox::SandboxPath,
        host: &Path,
        options: sandbox::CopyOptions,
    ) -> sandbox::Result<()> {
        if !options.overwrite && tokio::fs::try_exists(host).await.unwrap_or(false) {
            return Err(sandbox::AlreadyExists {
                operation: "copy out of sandbox",
                resource: sandbox::ResourceKind::File,
                id: host.display().to_string(),
            }
            .into());
        }
        let data = self
            .read_file(sandbox_id, guest, sandbox::ReadOptions)
            .await?;
        if let Some(parent) = host.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| sandbox::IoFailure {
                    operation: "copy out of sandbox",
                    resource: parent.display().to_string(),
                    reason: error.to_string(),
                    retry: sandbox::RetryClass::Unknown,
                })?;
        }
        tokio::fs::write(host, data)
            .await
            .map_err(|error| sandbox::IoFailure {
                operation: "copy out of sandbox",
                resource: host.display().to_string(),
                reason: error.to_string(),
                retry: sandbox::RetryClass::Unknown,
            })?;
        Ok(())
    }

    async fn list_files(
        &self,
        sandbox_id: &sandbox::SandboxId,
        path: sandbox::SandboxPath,
        options: sandbox::ListOptions,
    ) -> sandbox::Result<Vec<sandbox::FileEntry>> {
        let command = if options.recursive {
            r#"if [ ! -d "$FIRKIN_FS_PATH" ]; then exit 44; fi
find "$FIRKIN_FS_PATH" -mindepth 1 -exec stat -c '%n	%F	%s	%f' -- {} +"#
        } else {
            r#"if [ ! -d "$FIRKIN_FS_PATH" ]; then exit 44; fi
find "$FIRKIN_FS_PATH" -mindepth 1 -maxdepth 1 -exec stat -c '%n	%F	%s	%f' -- {} +"#
        };
        let output = self
            .run_filesystem_command(
                "list files",
                sandbox_id,
                Some(&path),
                CommandRequest::new(command).with_env("FIRKIN_FS_PATH", path.to_string()),
            )
            .await?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| filesystem_entry_from_stat_line("list files", Some(&path), line))
            .collect()
    }

    async fn mkdir(
        &self,
        sandbox_id: &sandbox::SandboxId,
        path: sandbox::SandboxPath,
        recursive: bool,
    ) -> sandbox::Result<sandbox::FileEntry> {
        let command = if recursive {
            r#"mkdir -p -- "$FIRKIN_FS_PATH""#
        } else {
            r#"mkdir -- "$FIRKIN_FS_PATH""#
        };
        self.run_filesystem_command(
            "make directory",
            sandbox_id,
            Some(&path),
            CommandRequest::new(command).with_env("FIRKIN_FS_PATH", path.to_string()),
        )
        .await?;
        self.stat_file(sandbox_id, path)
            .await
            .map(|stat| stat.entry)
    }

    async fn rename(
        &self,
        sandbox_id: &sandbox::SandboxId,
        from: sandbox::SandboxPath,
        to: sandbox::SandboxPath,
    ) -> sandbox::Result<sandbox::FileEntry> {
        self.run_filesystem_command(
            "rename file",
            sandbox_id,
            Some(&from),
            CommandRequest::new(
                r#"if [ ! -e "$FIRKIN_FS_SOURCE" ]; then exit 44; fi
mkdir -p -- "$(dirname -- "$FIRKIN_FS_DESTINATION")"
mv -- "$FIRKIN_FS_SOURCE" "$FIRKIN_FS_DESTINATION""#,
            )
            .with_env("FIRKIN_FS_SOURCE", from.to_string())
            .with_env("FIRKIN_FS_DESTINATION", to.to_string()),
        )
        .await?;
        self.stat_file(sandbox_id, to).await.map(|stat| stat.entry)
    }

    async fn remove_file(
        &self,
        sandbox_id: &sandbox::SandboxId,
        path: sandbox::SandboxPath,
    ) -> sandbox::Result<()> {
        self.run_filesystem_command(
            "remove file",
            sandbox_id,
            Some(&path),
            CommandRequest::new(
                r#"if [ ! -e "$FIRKIN_FS_PATH" ]; then exit 44; fi
rm -rf -- "$FIRKIN_FS_PATH""#,
            )
            .with_env("FIRKIN_FS_PATH", path.to_string()),
        )
        .await?;
        Ok(())
    }

    async fn stat_file(
        &self,
        sandbox_id: &sandbox::SandboxId,
        path: sandbox::SandboxPath,
    ) -> sandbox::Result<sandbox::FileStat> {
        let output = self
            .run_filesystem_command(
                "stat file",
                sandbox_id,
                Some(&path),
                CommandRequest::new(
                    r#"if [ ! -e "$FIRKIN_FS_PATH" ]; then exit 44; fi
stat -c '%n	%F	%s	%f' -- "$FIRKIN_FS_PATH""#,
                )
                .with_env("FIRKIN_FS_PATH", path.to_string()),
            )
            .await?;
        let line = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .ok_or_else(|| {
                filesystem_failure(
                    "stat file",
                    Some(&path),
                    "missing filesystem stat output".to_owned(),
                )
            })?
            .to_owned();
        let entry = filesystem_entry_from_stat_line("stat file", Some(&path), &line)?;
        let permissions = filesystem_permissions_from_stat_line("stat file", Some(&path), &line)?;
        Ok(sandbox::FileStat { entry, permissions })
    }

    async fn watch_files(
        &self,
        _sandbox_id: &sandbox::SandboxId,
        _path: sandbox::SandboxPath,
        _options: sandbox::WatchOptions,
    ) -> sandbox::Result<sandbox::FilesystemEventStream> {
        Err(sandbox::UnsupportedCapability::new(
            "watch files",
            sandbox::CapabilityName::FilesystemWatch,
            sandbox::CapabilityReason::Permanent {
                detail: "single-node adapter does not expose file watch events yet".to_owned(),
            },
        )
        .into())
    }
}

impl AppleVzBackend {
    async fn run_filesystem_command(
        &self,
        operation: &'static str,
        sandbox_id: &sandbox::SandboxId,
        path: Option<&sandbox::SandboxPath>,
        request: CommandRequest,
    ) -> sandbox::Result<super::CommandOutput> {
        let output = self
            .inner
            .run_command(sandbox_id.as_str(), request)
            .await
            .map_err(|error| Self::map_error(operation, error))?;
        if output.exit_code() == 44 {
            return Err(sandbox::NotFound::new(
                operation,
                sandbox::ResourceKind::File,
                path.map_or_else(|| sandbox_id.to_string(), ToString::to_string),
            )
            .into());
        }
        if output.exit_code() != 0 {
            let stderr = String::from_utf8_lossy(output.stderr()).trim().to_owned();
            return Err(filesystem_failure(
                operation,
                path,
                if stderr.is_empty() {
                    format!("guest command exited with {}", output.exit_code())
                } else {
                    format!("guest command exited with {}: {stderr}", output.exit_code())
                },
            ));
        }
        Ok(output)
    }
}

#[async_trait]
impl sandbox::SnapshotControl for AppleVzBackend {
    async fn capture_snapshot(
        &self,
        sandbox_id: &sandbox::SandboxId,
        options: sandbox::SnapshotOptions,
    ) -> sandbox::Result<sandbox::SnapshotRef> {
        let snapshot = self
            .inner
            .snapshot(sandbox_id.as_str(), options.name)
            .await
            .map_err(|error| Self::map_error("capture snapshot", error))?;
        Ok(sandbox::SnapshotRef::new(
            sandbox::SnapshotId::new(snapshot.snapshot_id)?,
            sandbox::SnapshotKind::Continuation,
        ))
    }

    async fn get_snapshot(
        &self,
        id: &sandbox::SnapshotId,
    ) -> sandbox::Result<sandbox::SnapshotInfo> {
        let record = find_snapshot(self.inner.snapshot_records(), id)?;
        snapshot_info(record)
    }

    async fn list_snapshots(
        &self,
        filter: sandbox::SnapshotFilter,
    ) -> sandbox::Result<Vec<sandbox::SnapshotInfo>> {
        self.inner
            .snapshot_records()
            .map_err(|error| Self::map_error("list snapshots", error))?
            .into_iter()
            .map(snapshot_info)
            .filter(|result| {
                result.as_ref().map_or(true, |info| {
                    filter.kind.is_none_or(|kind| kind == info.kind)
                })
            })
            .collect()
    }

    async fn delete_snapshot(&self, id: &sandbox::SnapshotId) -> sandbox::Result<()> {
        self.inner
            .delete_snapshot(id.as_str())
            .await
            .map_err(|error| Self::map_error("delete snapshot", error))
    }

    async fn export_snapshot(
        &self,
        _id: &sandbox::SnapshotId,
    ) -> sandbox::Result<sandbox::SnapshotExport> {
        Err(sandbox::UnsupportedCapability::new(
            "export snapshot",
            sandbox::CapabilityName::SnapshotExport,
            sandbox::CapabilityReason::Permanent {
                detail: "single-node adapter has not exposed portable snapshot export yet"
                    .to_owned(),
            },
        )
        .into())
    }

    async fn import_snapshot(
        &self,
        _import: sandbox::SnapshotImport,
    ) -> sandbox::Result<sandbox::SnapshotRef> {
        Err(sandbox::UnsupportedCapability::new(
            "import snapshot",
            sandbox::CapabilityName::SnapshotImport,
            sandbox::CapabilityReason::Permanent {
                detail: "single-node adapter has not exposed portable snapshot import yet"
                    .to_owned(),
            },
        )
        .into())
    }

    async fn pause_sandbox(
        &self,
        _sandbox: &sandbox::SandboxId,
        _options: sandbox::PauseOptions,
    ) -> sandbox::Result<sandbox::PausedSandbox> {
        Err(sandbox::UnsupportedCapability::new(
            "pause sandbox",
            sandbox::CapabilityName::PauseCapture,
            sandbox::CapabilityReason::Permanent {
                detail:
                    "single-node adapter supports continuation snapshots, not same-identity pause"
                        .to_owned(),
            },
        )
        .into())
    }
}

#[async_trait]
impl sandbox::WarmPoolControl for AppleVzBackend {
    async fn prewarm(
        &self,
        template: &sandbox::PreparedTemplate,
        spec: sandbox::WarmPoolSpec,
    ) -> sandbox::Result<sandbox::WarmMaintainReport> {
        let key = warm_pool_key(template.id())?;
        let current = self.warm_ready_count(&key)?;
        let needed = spec.depth.saturating_sub(current);
        for _ in 0..needed {
            let sandbox_id = self.create_warm_sandbox(template.id()).await?;
            self.record_warm_sandbox(&key, sandbox_id)?;
        }
        Ok(sandbox::WarmMaintainReport {
            created: needed,
            evicted: 0,
            ready: self.warm_ready_count(&key)?,
        })
    }

    async fn maintain(
        &self,
        targets: Vec<sandbox::WarmPoolTarget>,
    ) -> sandbox::Result<sandbox::WarmMaintainReport> {
        let mut report = sandbox::WarmMaintainReport::default();
        for target in targets {
            let key = warm_pool_key(&target.template_id)?;
            let current = self.warm_ready_count(&key)?;
            let needed = target.spec.depth.saturating_sub(current);
            for _ in 0..needed {
                let sandbox_id = self.create_warm_sandbox(&target.template_id).await?;
                self.record_warm_sandbox(&key, sandbox_id)?;
            }
            report.created += needed;
            report.ready += self.warm_ready_count(&key)?;
        }
        Ok(report)
    }

    async fn status(&self) -> sandbox::Result<sandbox::WarmPoolStatus> {
        let entries = self
            .warm_pools
            .lock()
            .map_err(|_| self.lock_failure("warm-pool status", "warm-pool registry"))?
            .iter()
            .map(|(key, sandboxes)| sandbox::WarmPoolEntry {
                key: key.clone(),
                ready: sandboxes.len(),
                total: sandboxes.len(),
            })
            .collect();
        Ok(sandbox::WarmPoolStatus { entries })
    }

    async fn checkout(
        &self,
        template: &sandbox::PreparedTemplate,
        policy: sandbox::WarmLeasePolicy,
    ) -> sandbox::Result<sandbox::WarmLease> {
        let key = warm_pool_key(template.id())?;
        if matches!(policy, sandbox::WarmLeasePolicy::CreateIfEmpty)
            && self.warm_ready_count(&key)? == 0
        {
            let sandbox_id = self.create_warm_sandbox(template.id()).await?;
            self.record_warm_sandbox(&key, sandbox_id)?;
        }
        let sandbox_id = self
            .warm_pools
            .lock()
            .map_err(|_| self.lock_failure("checkout warm sandbox", "warm-pool registry"))?
            .get_mut(&key)
            .and_then(Vec::pop)
            .ok_or_else(|| {
                sandbox::NotFound::new(
                    "checkout warm sandbox",
                    sandbox::ResourceKind::WarmPool,
                    key.to_string(),
                )
            })?;
        Ok(sandbox::WarmLease { key, sandbox_id })
    }

    async fn evict(
        &self,
        key: sandbox::WarmPoolKey,
        count: usize,
    ) -> sandbox::Result<sandbox::WarmMaintainReport> {
        let evicted = {
            let mut warm_pools = self
                .warm_pools
                .lock()
                .map_err(|_| self.lock_failure("evict warm sandboxes", "warm-pool registry"))?;
            let sandboxes = warm_pools.entry(key.clone()).or_default();
            let remove_count = count.min(sandboxes.len());
            sandboxes
                .split_off(sandboxes.len().saturating_sub(remove_count))
                .into_iter()
                .collect::<Vec<_>>()
        };
        for sandbox_id in &evicted {
            self.inner
                .delete(sandbox_id.as_str())
                .await
                .map_err(|error| Self::map_error("evict warm sandboxes", error))?;
        }
        Ok(sandbox::WarmMaintainReport {
            created: 0,
            evicted: evicted.len(),
            ready: self.warm_ready_count(&key)?,
        })
    }
}

impl AppleVzBackend {
    fn lock_failure(&self, operation: &'static str, resource: &'static str) -> sandbox::Error {
        sandbox::BackendFailure {
            operation,
            backend: Self::backend_name()
                .unwrap_or_else(|_| sandbox::BackendName::new("apple-vz").expect("valid")),
            reason: format!("{resource} lock poisoned"),
            retry: sandbox::RetryClass::Unknown,
        }
        .into()
    }

    fn warm_ready_count(&self, key: &sandbox::WarmPoolKey) -> sandbox::Result<usize> {
        Ok(self
            .warm_pools
            .lock()
            .map_err(|_| self.lock_failure("read warm-pool status", "warm-pool registry"))?
            .get(key)
            .map_or(0, Vec::len))
    }

    fn record_warm_sandbox(
        &self,
        key: &sandbox::WarmPoolKey,
        sandbox_id: sandbox::SandboxId,
    ) -> sandbox::Result<()> {
        self.warm_pools
            .lock()
            .map_err(|_| self.lock_failure("record warm sandbox", "warm-pool registry"))?
            .entry(key.clone())
            .or_default()
            .push(sandbox_id);
        Ok(())
    }

    async fn create_warm_sandbox(
        &self,
        template_id: &sandbox::TemplateId,
    ) -> sandbox::Result<sandbox::SandboxId> {
        let sandbox_id = self.next_sandbox_id()?;
        let runtime_template = self.runtime_template_id(template_id)?;
        let request = SingleNodeCreateRequest::new(
            sandbox_id.to_string(),
            runtime_template,
            default_resources(),
        );
        self.inner
            .create(request)
            .await
            .map_err(|error| Self::map_error("prewarm sandbox", error))?;
        Ok(sandbox_id)
    }

    fn create_request_from_spec(
        &self,
        sandbox_id: &sandbox::SandboxId,
        spec: sandbox::SandboxSpec,
    ) -> sandbox::Result<SingleNodeCreateRequest> {
        match spec {
            sandbox::SandboxSpec::Template {
                template_id,
                resources,
                env,
                deadline,
                ..
            } => {
                let runtime_template = self.runtime_template_id(&template_id)?;
                let mut request = SingleNodeCreateRequest::new(
                    sandbox_id.to_string(),
                    runtime_template,
                    resources.map_or_else(default_resources, single_node_resources),
                );
                if let Some(sandbox::SandboxDeadline::Timeout(timeout)) = deadline {
                    request = request.with_timeout(timeout);
                }
                let envs = env
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                Ok(request.with_envs(envs))
            }
            sandbox::SandboxSpec::Snapshot { snapshot, .. } => Err(sandbox::InvalidSpec::new(
                "create sandbox",
                sandbox::InvalidSpecReason::InvalidCommand(format!(
                    "use restore for snapshot {}",
                    snapshot.id()
                )),
            )
            .into()),
        }
    }

    fn runtime_template_id(&self, template_id: &sandbox::TemplateId) -> sandbox::Result<String> {
        let templates = self
            .templates
            .lock()
            .map_err(|_| self.lock_failure("resolve runtime template", "template registry"))?;
        let template = templates.get(template_id).ok_or_else(|| {
            sandbox::NotFound::new(
                "resolve runtime template",
                sandbox::ResourceKind::Template,
                template_id.to_string(),
            )
        })?;
        match template.source() {
            sandbox::TemplateSource::Oci(source) => Ok(source.reference.clone()),
            sandbox::TemplateSource::Git(source) => {
                Err(sandbox::TemplatePrepareFailure::UnsupportedImageConfig {
                    reason: format!(
                        "single-node Apple/VZ adapter cannot create Git template source `{}`",
                        source.url
                    ),
                }
                .into())
            }
            sandbox::TemplateSource::Local(source) => {
                Err(sandbox::TemplatePrepareFailure::UnsupportedImageConfig {
                    reason: format!(
                        "single-node Apple/VZ adapter cannot create local template source `{}`",
                        source.path.display()
                    ),
                }
                .into())
            }
        }
    }

    fn public_template_id_for_runtime(
        &self,
        runtime_template: &str,
    ) -> sandbox::Result<sandbox::TemplateId> {
        let templates = self
            .templates
            .lock()
            .map_err(|_| self.lock_failure("map runtime template", "template registry"))?;
        if let Some((id, _)) = templates.iter().find(|(_, template)| {
            matches!(
                template.source(),
                sandbox::TemplateSource::Oci(source) if source.reference == runtime_template
            )
        }) {
            return Ok(id.clone());
        }
        sandbox::TemplateId::new(sanitize_id_value(runtime_template)).map_err(Into::into)
    }

    fn active_to_info(
        &self,
        record: super::ActiveSessionRecord,
    ) -> sandbox::Result<sandbox::SandboxInfo> {
        Ok(sandbox::SandboxInfo {
            id: sandbox::SandboxId::new(record.sandbox_id)?,
            state: sandbox::SandboxState::Running,
            template_id: Some(self.public_template_id_for_runtime(&record.template_id)?),
            snapshot_id: None,
            resources: Some(sandbox::SandboxResources::new(
                record.resources.cpu_count(),
                record.resources.memory().as_bytes(),
            )),
            deadline: None,
            created_at: OffsetDateTime::from_unix_timestamp(record.started_at_unix_seconds)
                .unwrap_or_else(|_| OffsetDateTime::now_utc()),
            metadata: sandbox::SandboxMetadata::default(),
        })
    }
}

fn single_node_resources(resources: sandbox::SandboxResources) -> super::SandboxResources {
    super::SandboxResources::new(resources.vcpus, Size::bytes(resources.memory_bytes))
}

fn default_resources() -> super::SandboxResources {
    super::SandboxResources::new(2, Size::gib(8))
}

fn command_request(command: sandbox::Command) -> sandbox::Result<CommandRequest> {
    let request = match command.mode() {
        sandbox::CommandMode::Shell(command) => CommandRequest::new(command.clone()),
        sandbox::CommandMode::Argv { program, args } => {
            let joined = std::iter::once(program.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            CommandRequest::new(joined)
        }
    };
    Ok(request)
}

fn process_start_request(command: sandbox::Command) -> sandbox::Result<EnvdProcessStartRequest> {
    let (cmd, args) = match command.mode() {
        sandbox::CommandMode::Shell(command) => (
            "/bin/sh".to_owned(),
            vec!["-lc".to_owned(), command.clone()],
        ),
        sandbox::CommandMode::Argv { program, args } => (program.clone(), args.clone()),
    };
    Ok(EnvdProcessStartRequest {
        cmd,
        args,
        envs: command
            .env_ref()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        cwd: command.cwd_ref().map(ToOwned::to_owned),
        tag: command.tag_ref().map(ToString::to_string),
        stdin: Some(true),
        pty: command.pty_ref().map(|size| EnvdPtySize {
            rows: u32::from(size.rows),
            cols: u32::from(size.cols),
        }),
    })
}

fn process_selector(selector: sandbox::ProcessSelector) -> EnvdProcessSelector {
    match selector {
        sandbox::ProcessSelector::Id(id) => id.as_str().parse::<u32>().map_or_else(
            |_| EnvdProcessSelector::Tag(id.to_string()),
            EnvdProcessSelector::Pid,
        ),
        sandbox::ProcessSelector::Tag(tag) => EnvdProcessSelector::Tag(tag.to_string()),
    }
}

fn process_signal(signal: sandbox::Signal) -> EnvdProcessSignal {
    match signal {
        sandbox::Signal::Term => EnvdProcessSignal::Sigterm,
        sandbox::Signal::Kill => EnvdProcessSignal::Sigkill,
        sandbox::Signal::Interrupt => EnvdProcessSignal::Unknown(2),
        sandbox::Signal::Number(value) => EnvdProcessSignal::Unknown(value),
    }
}

fn process_input(input: sandbox::ProcessInput) -> EnvdProcessInput {
    match input {
        sandbox::ProcessInput::Bytes(bytes) => EnvdProcessInput::Stdin(bytes.to_vec()),
        sandbox::ProcessInput::Eof => EnvdProcessInput::Stdin(Vec::new()),
    }
}

fn process_info(info: firkin_envd::EnvdProcessInfo) -> sandbox::Result<sandbox::ProcessInfo> {
    Ok(sandbox::ProcessInfo {
        id: sandbox::ProcessId::new(info.pid.to_string())?,
        tag: info.tag.map(sandbox::ProcessTag::new).transpose()?,
        status: sandbox::ProcessStatus::Running,
    })
}

fn process_info_from_pid(pid: u32) -> sandbox::Result<sandbox::ProcessInfo> {
    Ok(sandbox::ProcessInfo {
        id: sandbox::ProcessId::new(pid.to_string())?,
        tag: None,
        status: sandbox::ProcessStatus::Running,
    })
}

fn process_event(event: EnvdProcessStreamEvent) -> sandbox::Result<sandbox::process::ProcessEvent> {
    match event {
        EnvdProcessStreamEvent::Start { pid } => {
            process_info_from_pid(pid).map(sandbox::process::ProcessEvent::Started)
        }
        EnvdProcessStreamEvent::Stdout(bytes) => {
            Ok(sandbox::process::ProcessEvent::Stdout(Bytes::from(bytes)))
        }
        EnvdProcessStreamEvent::Stderr(bytes) => {
            Ok(sandbox::process::ProcessEvent::Stderr(Bytes::from(bytes)))
        }
        EnvdProcessStreamEvent::Pty(bytes) => {
            Ok(sandbox::process::ProcessEvent::Stdout(Bytes::from(bytes)))
        }
        EnvdProcessStreamEvent::End {
            exit_code, exited, ..
        } => {
            if exited {
                Ok(sandbox::process::ProcessEvent::Exited(
                    sandbox::CommandExit { code: exit_code },
                ))
            } else {
                Ok(sandbox::process::ProcessEvent::Signaled(
                    sandbox::Signal::Number(exit_code),
                ))
            }
        }
    }
}

fn process_backend_failure(
    operation: &'static str,
    error: firkin_e2b_contract::BackendError,
) -> sandbox::Error {
    sandbox::ProcessFailure {
        operation,
        sandbox_id: None,
        process_id: None,
        reason: error.to_string(),
        retry: sandbox::RetryClass::Unknown,
    }
    .into()
}

fn command_output(output: super::CommandOutput) -> sandbox::CommandOutput {
    sandbox::CommandOutput {
        status: sandbox::CommandStatus::Exited(sandbox::CommandExit {
            code: output.exit_code(),
        }),
        stdout: Bytes::from(output.stdout),
        stderr: Bytes::from(output.stderr),
    }
}

fn command_output_from_envd(output: firkin_envd::EnvdProcessOutput) -> sandbox::CommandOutput {
    sandbox::CommandOutput {
        status: sandbox::CommandStatus::Exited(sandbox::CommandExit {
            code: output.exit_code,
        }),
        stdout: Bytes::from(output.stdout),
        stderr: Bytes::from(output.stderr),
    }
}

fn snapshot_info(record: SnapshotRecord) -> sandbox::Result<sandbox::SnapshotInfo> {
    Ok(sandbox::SnapshotInfo {
        id: sandbox::SnapshotId::new(record.snapshot_id)?,
        kind: sandbox::SnapshotKind::Continuation,
        source_sandbox_id: Some(sandbox::SandboxId::new(record.source_sandbox_id)?),
        source_template_id: None,
        created_at: OffsetDateTime::now_utc(),
        integrity: None,
    })
}

fn find_snapshot(
    records: super::Result<Vec<SnapshotRecord>>,
    id: &sandbox::SnapshotId,
) -> sandbox::Result<SnapshotRecord> {
    records
        .map_err(|error| AppleVzBackend::map_error("find snapshot", error))?
        .into_iter()
        .find(|record| record.snapshot_id == id.as_str())
        .ok_or_else(|| {
            sandbox::NotFound::new(
                "find snapshot",
                sandbox::ResourceKind::Snapshot,
                id.to_string(),
            )
            .into()
        })
}

fn warm_pool_key(template_id: &sandbox::TemplateId) -> sandbox::Result<sandbox::WarmPoolKey> {
    sandbox::WarmPoolKey::new(format!("template-{}", template_id.as_str())).map_err(Into::into)
}

fn filesystem_entry_from_stat_line(
    operation: &'static str,
    requested_path: Option<&sandbox::SandboxPath>,
    line: &str,
) -> sandbox::Result<sandbox::FileEntry> {
    let fields = filesystem_stat_fields(operation, requested_path, line)?;
    let path = sandbox::SandboxPath::new(fields[0].to_owned())?;
    let file_type = filesystem_file_type(fields[1]);
    let size_bytes = fields[2].parse::<u64>().map_err(|error| {
        filesystem_failure(
            operation,
            requested_path,
            format!("invalid filesystem size `{}`: {error}", fields[2]),
        )
    })?;
    Ok(sandbox::FileEntry {
        path,
        file_type,
        size_bytes,
    })
}

fn filesystem_permissions_from_stat_line(
    operation: &'static str,
    requested_path: Option<&sandbox::SandboxPath>,
    line: &str,
) -> sandbox::Result<sandbox::FilePermissions> {
    let fields = filesystem_stat_fields(operation, requested_path, line)?;
    let mode = u32::from_str_radix(fields[3].trim_start_matches("0x"), 16).map_err(|error| {
        filesystem_failure(
            operation,
            requested_path,
            format!("invalid filesystem mode `{}`: {error}", fields[3]),
        )
    })?;
    Ok(sandbox::FilePermissions { mode })
}

fn filesystem_stat_fields<'a>(
    operation: &'static str,
    requested_path: Option<&sandbox::SandboxPath>,
    line: &'a str,
) -> sandbox::Result<Vec<&'a str>> {
    let fields = line.trim_end().split('\t').collect::<Vec<_>>();
    if fields.len() != 4 {
        return Err(filesystem_failure(
            operation,
            requested_path,
            format!("invalid filesystem stat output `{line}`"),
        ));
    }
    Ok(fields)
}

fn filesystem_file_type(value: &str) -> sandbox::FileType {
    match value {
        "directory" => sandbox::FileType::Directory,
        "file" | "regular file" => sandbox::FileType::File,
        "symbolic link" | "symlink" => sandbox::FileType::Symlink,
        _ => sandbox::FileType::Other,
    }
}

fn filesystem_failure(
    operation: &'static str,
    path: Option<&sandbox::SandboxPath>,
    reason: String,
) -> sandbox::Error {
    sandbox::FilesystemFailure {
        operation,
        sandbox_id: None,
        path: path.cloned(),
        reason,
        retry: sandbox::RetryClass::Unknown,
    }
    .into()
}

fn source_reference(source: &sandbox::TemplateSource) -> Option<String> {
    match source {
        sandbox::TemplateSource::Oci(source) => Some(source.reference.clone()),
        sandbox::TemplateSource::Git(source) => Some(source.url.clone()),
        sandbox::TemplateSource::Local(source) => Some(source.path.display().to_string()),
    }
}

fn template_id_from_source(source: &sandbox::TemplateSource) -> String {
    let raw = source_reference(source).unwrap_or_else(|| "template".to_owned());
    sanitize_id_value(&raw)
}

fn sanitize_id_value(raw: &str) -> String {
    let cleaned = raw
        .bytes()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-' => char::from(byte),
            _ => '_',
        })
        .collect::<String>();
    cleaned.trim_matches('_').chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use bytes::Bytes;
    use firkin_envd::{
        EnvdProcessEventStream, EnvdProcessInput, EnvdProcessSelector, EnvdProcessStartRequest,
        EnvdProcessStreamEvent,
    };
    use firkin_sandbox::process::ProcessEvent as SandboxProcessEvent;
    use firkin_sandbox::{
        Command, DataPlaneSpec, Error as SandboxError, ProcessInput, ProcessSelector, ProcessTag,
        Runtime, SandboxPath, SandboxSpec, TemplateControl, TemplatePrepareFailure, TemplateSpec,
        WarmLeasePolicy, WarmPoolSpec,
    };
    use firkin_types::Size;
    use futures_util::StreamExt;
    use tokio::sync::mpsc;

    use super::AppleVzBackend;
    use crate::{
        CommandOutput, CommandRequest, Result, RuntimeCreatedSandbox, RuntimeDriver,
        SingleNodeBackend, SingleNodeConfig, SingleNodeCreateRequest, StateStore,
    };

    #[derive(Default)]
    struct RecordingDriver {
        creates: Mutex<Vec<SingleNodeCreateRequest>>,
        commands: Mutex<Vec<CommandRequest>>,
        outputs: Mutex<VecDeque<CommandOutput>>,
        process_requests: Mutex<Vec<EnvdProcessStartRequest>>,
        process_inputs: Mutex<Vec<(EnvdProcessSelector, EnvdProcessInput)>>,
    }

    impl RecordingDriver {
        fn push_output(&self, output: CommandOutput) {
            self.outputs.lock().expect("outputs lock").push_back(output);
        }
    }

    #[async_trait]
    impl RuntimeDriver for RecordingDriver {
        fn runtime_sandbox_exists(&self, _sandbox_id: &str) -> Result<bool> {
            Ok(true)
        }

        async fn create(&self, request: SingleNodeCreateRequest) -> Result<RuntimeCreatedSandbox> {
            self.creates
                .lock()
                .expect("creates lock")
                .push(request.clone());
            Ok(RuntimeCreatedSandbox {
                sandbox_id: request.sandbox_id().to_owned(),
                client_id: "client-1".to_owned(),
                envd_access_token: None,
                traffic_access_token: None,
            })
        }

        async fn delete(&self, _sandbox_id: &str) -> Result<()> {
            Ok(())
        }

        async fn run_command(
            &self,
            _sandbox_id: &str,
            request: CommandRequest,
        ) -> Result<CommandOutput> {
            self.commands.lock().expect("commands lock").push(request);
            Ok(self
                .outputs
                .lock()
                .expect("outputs lock")
                .pop_front()
                .unwrap_or_else(|| CommandOutput::new(b"ok\n".to_vec(), Vec::new(), 0)))
        }

        async fn start_process_stream(
            &self,
            _sandbox_id: &str,
            request: EnvdProcessStartRequest,
        ) -> Result<EnvdProcessEventStream<firkin_e2b_contract::BackendError>> {
            self.process_requests
                .lock()
                .expect("process requests lock")
                .push(request);
            let (sender, receiver) = mpsc::channel(8);
            for event in [
                EnvdProcessStreamEvent::Start { pid: 42 },
                EnvdProcessStreamEvent::Stdout(b"streamed\n".to_vec()),
                EnvdProcessStreamEvent::Stderr(b"warn\n".to_vec()),
                EnvdProcessStreamEvent::End {
                    exit_code: 0,
                    exited: true,
                    status: "exited".to_owned(),
                    error: None,
                },
            ] {
                sender.send(Ok(event)).await.expect("send process event");
            }
            Ok(EnvdProcessEventStream::from_receiver(receiver))
        }

        async fn send_process_input(
            &self,
            _sandbox_id: &str,
            selector: EnvdProcessSelector,
            input: EnvdProcessInput,
        ) -> Result<()> {
            self.process_inputs
                .lock()
                .expect("process inputs lock")
                .push((selector, input));
            Ok(())
        }
    }

    #[tokio::test]
    async fn sandbox_runtime_execs_through_single_node_backend() {
        let driver = Arc::new(RecordingDriver::default());
        let config = SingleNodeConfig::new("/tmp/firkin-sandbox-api-test", "cube.localhost");
        let backend =
            SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
        let runtime = Runtime::build(AppleVzBackend::from_single_node(backend))
            .await
            .expect("runtime");

        let template = runtime
            .templates()
            .prepare(TemplateSpec::oci("example").data_plane(DataPlaneSpec::none()))
            .await
            .expect("prepare");
        let sandbox = runtime
            .sandboxes()
            .create(SandboxSpec::from_template(&template).resources(
                firkin_sandbox::SandboxResources::new(2, Size::gib(1).as_bytes()),
            ))
            .await
            .expect("sandbox");
        let output = sandbox.exec(Command::shell("echo ok")).await.expect("exec");

        assert_eq!(output.stdout, Bytes::from_static(b"ok\n"));
        assert_eq!(
            driver.commands.lock().expect("commands lock")[0].command(),
            "echo ok"
        );
    }

    #[tokio::test]
    async fn sandbox_process_stream_routes_through_single_node_driver() {
        let driver = Arc::new(RecordingDriver::default());
        let config = SingleNodeConfig::new("/tmp/firkin-sandbox-api-test", "cube.localhost");
        let backend =
            SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
        let runtime = Runtime::build(AppleVzBackend::from_single_node(backend))
            .await
            .expect("runtime");
        let template = runtime
            .templates()
            .prepare(TemplateSpec::oci("example").data_plane(DataPlaneSpec::none()))
            .await
            .expect("prepare");
        let sandbox = runtime
            .sandboxes()
            .create(SandboxSpec::from_template(&template))
            .await
            .expect("sandbox");

        let mut stream = sandbox
            .process()
            .start_stream(
                Command::argv("printf", ["ok"])
                    .tag(ProcessTag::new("agent-a").expect("tag"))
                    .cwd("/work")
                    .env("A", "B"),
            )
            .await
            .expect("stream");

        assert_eq!(
            stream.next().await.expect("start").expect("start event"),
            SandboxProcessEvent::Started(firkin_sandbox::ProcessInfo {
                id: firkin_sandbox::ProcessId::new("42").expect("pid"),
                tag: None,
                status: firkin_sandbox::ProcessStatus::Running,
            })
        );
        assert_eq!(
            stream.next().await.expect("stdout").expect("stdout event"),
            SandboxProcessEvent::Stdout(Bytes::from_static(b"streamed\n"))
        );
        assert_eq!(
            stream.next().await.expect("stderr").expect("stderr event"),
            SandboxProcessEvent::Stderr(Bytes::from_static(b"warn\n"))
        );
        assert_eq!(
            stream.next().await.expect("exit").expect("exit event"),
            SandboxProcessEvent::Exited(firkin_sandbox::CommandExit::success())
        );
        assert!(stream.next().await.is_none());

        let requests = driver
            .process_requests
            .lock()
            .expect("process requests lock");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].cmd, "printf");
        assert_eq!(requests[0].args, ["ok"]);
        assert_eq!(requests[0].tag.as_deref(), Some("agent-a"));
        assert_eq!(requests[0].cwd.as_deref(), Some("/work"));
        assert_eq!(requests[0].envs.get("A").map(String::as_str), Some("B"));
        assert_eq!(requests[0].stdin, Some(true));
    }

    #[tokio::test]
    async fn sandbox_process_input_routes_through_single_node_driver() {
        let driver = Arc::new(RecordingDriver::default());
        let config = SingleNodeConfig::new("/tmp/firkin-sandbox-api-test", "cube.localhost");
        let backend =
            SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
        let runtime = Runtime::build(AppleVzBackend::from_single_node(backend))
            .await
            .expect("runtime");
        let template = runtime
            .templates()
            .prepare(TemplateSpec::oci("example").data_plane(DataPlaneSpec::none()))
            .await
            .expect("prepare");
        let sandbox = runtime
            .sandboxes()
            .create(SandboxSpec::from_template(&template))
            .await
            .expect("sandbox");

        sandbox
            .process()
            .send_input(
                ProcessSelector::Tag(ProcessTag::new("agent-a").expect("tag")),
                ProcessInput::Bytes(Bytes::from_static(b"echo ok\n")),
            )
            .await
            .expect("send input");

        let inputs = driver.process_inputs.lock().expect("process inputs lock");
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].0, EnvdProcessSelector::Tag("agent-a".to_owned()));
        assert_eq!(inputs[0].1, EnvdProcessInput::Stdin(b"echo ok\n".to_vec()));
    }

    #[tokio::test]
    async fn sandbox_filesystem_write_and_read_route_through_single_node_commands() {
        let driver = Arc::new(RecordingDriver::default());
        driver.push_output(CommandOutput::new(Vec::new(), Vec::new(), 0));
        driver.push_output(CommandOutput::new(
            b"/work/README.md\tfile\t12\t644\n".to_vec(),
            Vec::new(),
            0,
        ));
        driver.push_output(CommandOutput::new(b"hello firkin".to_vec(), Vec::new(), 0));
        let config = SingleNodeConfig::new("/tmp/firkin-sandbox-api-test", "cube.localhost");
        let backend =
            SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
        let runtime = Runtime::build(AppleVzBackend::from_single_node(backend))
            .await
            .expect("runtime");
        let template = runtime
            .templates()
            .prepare(TemplateSpec::oci("example").data_plane(DataPlaneSpec::none()))
            .await
            .expect("prepare");
        let sandbox = runtime
            .sandboxes()
            .create(SandboxSpec::from_template(&template))
            .await
            .expect("sandbox");
        let path = SandboxPath::new("/work/README.md").expect("path");

        let entry = sandbox
            .fs()
            .write(path.clone(), Bytes::from_static(b"hello firkin"))
            .await
            .expect("write");
        let bytes = sandbox.fs().read(path).await.expect("read");

        assert_eq!(entry.size_bytes, 12);
        assert_eq!(bytes, Bytes::from_static(b"hello firkin"));
        let commands = driver.commands.lock().expect("commands lock");
        assert_eq!(commands.len(), 3);
        assert_eq!(
            commands[0].env().get("FIRKIN_FS_PATH").map(String::as_str),
            Some("/work/README.md")
        );
        assert_eq!(commands[0].stdin(), Some(b"hello firkin".as_slice()));
        assert!(commands[0].command().contains("cat >"));
        assert!(commands[1].command().contains("stat -c"));
        assert!(commands[2].command().contains("cat --"));
    }

    #[tokio::test]
    async fn sandbox_filesystem_missing_stat_maps_to_typed_not_found() {
        let driver = Arc::new(RecordingDriver::default());
        driver.push_output(CommandOutput::new(Vec::new(), b"missing\n".to_vec(), 44));
        let config = SingleNodeConfig::new("/tmp/firkin-sandbox-api-test", "cube.localhost");
        let backend =
            SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
        let runtime = Runtime::build(AppleVzBackend::from_single_node(backend))
            .await
            .expect("runtime");
        let template = runtime
            .templates()
            .prepare(TemplateSpec::oci("example").data_plane(DataPlaneSpec::none()))
            .await
            .expect("prepare");
        let sandbox = runtime
            .sandboxes()
            .create(SandboxSpec::from_template(&template))
            .await
            .expect("sandbox");

        let error = sandbox
            .fs()
            .stat(SandboxPath::new("/work/missing.txt").expect("path"))
            .await
            .expect_err("missing path rejects");

        assert!(matches!(
            error,
            SandboxError::NotFound(firkin_sandbox::NotFound {
                resource: firkin_sandbox::ResourceKind::File,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn sandbox_warm_pool_prewarm_status_checkout_and_evict_are_typed() {
        let driver = Arc::new(RecordingDriver::default());
        let config = SingleNodeConfig::new("/tmp/firkin-sandbox-api-test", "cube.localhost");
        let backend =
            SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
        let runtime = Runtime::build(AppleVzBackend::from_single_node(backend))
            .await
            .expect("runtime");
        let template = runtime
            .templates()
            .prepare(TemplateSpec::oci("example").data_plane(DataPlaneSpec::none()))
            .await
            .expect("prepare");

        let report = runtime
            .warm_pool()
            .prewarm(&template, WarmPoolSpec::depth(1))
            .await
            .expect("prewarm");
        let status = runtime.warm_pool().status().await.expect("status");
        let lease = runtime
            .warm_pool()
            .checkout(&template, WarmLeasePolicy::RequireReady)
            .await
            .expect("checkout");
        let evict = runtime
            .warm_pool()
            .evict(lease.key.clone(), 1)
            .await
            .expect("evict empty pool");

        assert_eq!(report.created, 1);
        assert_eq!(report.ready, 1);
        assert_eq!(status.entries[0].ready, 1);
        assert!(lease.sandbox_id.as_str().starts_with("sbx_firkin_"));
        assert_eq!(evict.evicted, 0);
    }

    #[tokio::test]
    async fn sandbox_create_resolves_prepared_template_to_original_oci_reference() {
        let driver = Arc::new(RecordingDriver::default());
        let config = SingleNodeConfig::new("/tmp/firkin-sandbox-api-test", "cube.localhost");
        let backend =
            SingleNodeBackend::with_driver_and_state(config, driver.clone(), StateStore::new());
        let runtime = Runtime::build(AppleVzBackend::from_single_node(backend))
            .await
            .expect("runtime");
        let template = runtime
            .templates()
            .prepare(
                TemplateSpec::oci("docker.io/library/busybox:latest")
                    .data_plane(DataPlaneSpec::none()),
            )
            .await
            .expect("prepare");

        runtime
            .sandboxes()
            .create(SandboxSpec::from_template(&template))
            .await
            .expect("sandbox");

        assert_eq!(
            driver.creates.lock().expect("creates lock")[0].template_id(),
            "docker.io/library/busybox:latest"
        );
    }

    #[tokio::test]
    async fn envd_injection_is_prepare_time_typed_refusal() {
        let backend = AppleVzBackend::from_config(SingleNodeConfig::new(
            "/tmp/firkin-sandbox-api-test",
            "cube.localhost",
        ));
        let error = backend
            .prepare_template(TemplateSpec::oci("example"))
            .await
            .expect_err("envd injection not faked");
        assert!(matches!(
            error,
            firkin_sandbox::Error::TemplatePrepareFailure(
                TemplatePrepareFailure::EnvdMissing { .. }
            )
        ));
    }
}
