use std::collections::HashMap;

use async_trait::async_trait;
use firkin_e2b_contract::BackendError as E2bBackendError;
use firkin_envd::{
    EnvdProcessEventStream, EnvdProcessInfo, EnvdProcessInput, EnvdProcessOutput,
    EnvdProcessSelector, EnvdProcessSignal, EnvdProcessStartRequest, EnvdPtySize,
};

use super::{
    CommandOutput, CommandRequest, Error, Result, RuntimeCreatedSandbox, RuntimeSnapshotRef,
    SingleNodeCreateRequest, SnapshotRecord,
};

/// Runtime driver for local single-node sandbox mechanics.
///
/// Product adapters own HTTP DTOs and product policy. Implementations of this
/// trait own the concrete VM/container lifecycle.
#[async_trait]
pub trait RuntimeDriver: Send + Sync {
    /// Return whether a runtime sandbox is attached for the caller-visible ID.
    ///
    /// # Errors
    ///
    /// Returns a runtime error when the concrete driver cannot inspect runtime
    /// state.
    fn runtime_sandbox_exists(&self, sandbox_id: &str) -> Result<bool>;

    /// Create a new sandbox from a template.
    async fn create(&self, request: SingleNodeCreateRequest) -> Result<RuntimeCreatedSandbox>;

    /// Delete an attached sandbox.
    async fn delete(&self, sandbox_id: &str) -> Result<()>;

    /// Restore a new sandbox from a runtime snapshot.
    async fn restore(
        &self,
        request: SingleNodeCreateRequest,
        _snapshot: SnapshotRecord,
    ) -> Result<RuntimeCreatedSandbox> {
        self.create(request).await
    }

    /// Capture a runtime snapshot for a sandbox.
    async fn snapshot(
        &self,
        _sandbox_id: &str,
        _name: Option<String>,
    ) -> Result<RuntimeSnapshotRef> {
        Err(Error::UnsupportedCapability(
            "single-node runtime snapshot is not implemented by this driver".to_string(),
        ))
    }

    /// Delete a runtime snapshot artifact.
    async fn delete_snapshot(&self, _snapshot_id: &str) -> Result<()> {
        Ok(())
    }

    /// Execute a foreground command in a sandbox.
    async fn run_command(
        &self,
        _sandbox_id: &str,
        _request: CommandRequest,
    ) -> Result<CommandOutput> {
        Err(Error::UnsupportedCapability(
            "single-node runtime command execution is not implemented by this driver".to_string(),
        ))
    }

    /// Start a retained process stream in a sandbox.
    async fn start_process_stream(
        &self,
        _sandbox_id: &str,
        _request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<E2bBackendError>> {
        Err(Error::UnsupportedCapability(
            "single-node runtime retained process streaming is not implemented by this driver"
                .to_string(),
        ))
    }

    /// List retained process records for a sandbox.
    async fn list_processes(&self, _sandbox_id: &str) -> Result<Vec<EnvdProcessInfo>> {
        Ok(Vec::new())
    }

    /// Connect to a retained process record in a sandbox.
    async fn connect_process(
        &self,
        _sandbox_id: &str,
        _selector: EnvdProcessSelector,
    ) -> Result<EnvdProcessOutput> {
        Err(Error::UnsupportedCapability(
            "single-node runtime retained process connect is not implemented by this driver"
                .to_string(),
        ))
    }

    /// Send input to a retained process in a sandbox.
    async fn send_process_input(
        &self,
        _sandbox_id: &str,
        _selector: EnvdProcessSelector,
        _input: EnvdProcessInput,
    ) -> Result<()> {
        Err(Error::UnsupportedCapability(
            "single-node runtime retained process input is not implemented by this driver"
                .to_string(),
        ))
    }

    /// Close stdin for a retained process in a sandbox.
    async fn close_process_stdin(
        &self,
        _sandbox_id: &str,
        _selector: EnvdProcessSelector,
    ) -> Result<()> {
        Err(Error::UnsupportedCapability(
            "single-node runtime retained process stdin close is not implemented by this driver"
                .to_string(),
        ))
    }

    /// Send a signal to a retained process in a sandbox.
    async fn signal_process(
        &self,
        _sandbox_id: &str,
        _selector: EnvdProcessSelector,
        _signal: EnvdProcessSignal,
    ) -> Result<()> {
        Err(Error::UnsupportedCapability(
            "single-node runtime retained process signal is not implemented by this driver"
                .to_string(),
        ))
    }

    /// Resize a retained PTY process in a sandbox.
    async fn resize_process_pty(
        &self,
        _sandbox_id: &str,
        _selector: EnvdProcessSelector,
        _pty: Option<EnvdPtySize>,
    ) -> Result<()> {
        Err(Error::UnsupportedCapability(
            "single-node runtime retained process PTY resize is not implemented by this driver"
                .to_string(),
        ))
    }

    /// Start a detached template initialization command in a sandbox.
    async fn start_template_command(
        &self,
        _sandbox_id: &str,
        _command: String,
        _envs: HashMap<String, String>,
    ) -> Result<()> {
        Err(Error::UnsupportedCapability(
            "single-node runtime template start command is not implemented by this driver"
                .to_string(),
        ))
    }
}
