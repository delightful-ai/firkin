use std::collections::HashMap;
use std::time::Duration;

use firkin_types::Size;

/// Resource request for a single-node sandbox session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SandboxResources {
    /// Requested virtual CPU count.
    pub vcpus: u32,
    /// Requested memory in bytes.
    pub memory_bytes: u64,
}

impl SandboxResources {
    /// Construct a resource request.
    #[must_use]
    pub const fn new(vcpus: u32, memory: Size) -> Self {
        Self {
            vcpus,
            memory_bytes: memory.as_bytes(),
        }
    }

    /// Return requested virtual CPU count.
    #[must_use]
    pub const fn cpu_count(&self) -> u32 {
        self.vcpus
    }

    /// Return requested memory.
    #[must_use]
    pub const fn memory(&self) -> Size {
        Size::bytes(self.memory_bytes)
    }
}

/// Runtime mode for a single-node sandbox.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SingleNodeRuntimeMode {
    /// One product sandbox maps to one Firkin VM-backed container.
    #[default]
    SingleVmBackedContainer,
}

/// Request to create a single-node runtime session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SingleNodeCreateRequest {
    sandbox_id: String,
    template_id: String,
    resources: SandboxResources,
    runtime_mode: SingleNodeRuntimeMode,
    timeout: Option<Duration>,
    env: HashMap<String, String>,
}

impl SingleNodeCreateRequest {
    /// Construct a create request using the default one-VM-backed-container mode.
    #[must_use]
    pub fn new(
        sandbox_id: impl Into<String>,
        template_id: impl Into<String>,
        resources: SandboxResources,
    ) -> Self {
        Self {
            sandbox_id: sandbox_id.into(),
            template_id: template_id.into(),
            resources,
            runtime_mode: SingleNodeRuntimeMode::SingleVmBackedContainer,
            timeout: None,
            env: HashMap::new(),
        }
    }

    /// Return the caller-visible sandbox ID.
    #[must_use]
    pub fn sandbox_id(&self) -> &str {
        &self.sandbox_id
    }

    /// Return the template ID used to create the session.
    #[must_use]
    pub fn template_id(&self) -> &str {
        &self.template_id
    }

    /// Return requested resources.
    #[must_use]
    pub const fn resources(&self) -> &SandboxResources {
        &self.resources
    }

    /// Return the runtime mode.
    #[must_use]
    pub const fn runtime_mode(&self) -> SingleNodeRuntimeMode {
        self.runtime_mode
    }

    /// Return the optional operation timeout.
    #[must_use]
    pub const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Return environment variables passed to the runtime session.
    #[must_use]
    pub fn env(&self) -> &HashMap<String, String> {
        &self.env
    }

    /// Set the operation timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Add or replace a runtime environment variable.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Replace runtime environment variables.
    #[must_use]
    pub fn with_envs(mut self, env: HashMap<String, String>) -> Self {
        self.env = env;
        self
    }
}

/// Current lifecycle state of a runtime session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SandboxSessionState {
    /// The session is being created.
    Creating,
    /// The session is ready for runtime operations.
    #[default]
    Running,
    /// The session is being stopped.
    Stopping,
    /// The session has stopped.
    Stopped,
}

/// Runtime session returned by single-node creation and lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxSession {
    sandbox_id: String,
    state: SandboxSessionState,
}

impl SandboxSession {
    /// Construct a runtime session record.
    #[must_use]
    pub fn new(sandbox_id: impl Into<String>, state: SandboxSessionState) -> Self {
        Self {
            sandbox_id: sandbox_id.into(),
            state,
        }
    }

    /// Return the caller-visible sandbox ID.
    #[must_use]
    pub fn sandbox_id(&self) -> &str {
        &self.sandbox_id
    }

    /// Return the current session state.
    #[must_use]
    pub const fn state(&self) -> SandboxSessionState {
        self.state
    }
}

/// Runtime creation result returned before a product adapter maps to API DTOs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeCreatedSandbox {
    /// Sandbox ID that was created.
    pub sandbox_id: String,
    /// Runtime client ID.
    pub client_id: String,
    /// Optional envd access token.
    pub envd_access_token: Option<String>,
    /// Optional traffic access token.
    pub traffic_access_token: Option<String>,
}

/// Snapshot kind managed by the single-node runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotKind {
    /// Snapshot used as a reusable template root.
    Template,
    /// Snapshot used to resume a previously running session.
    Continuation,
}

/// Runtime snapshot record.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SnapshotRecord {
    /// Snapshot ID.
    pub snapshot_id: String,
    /// Source sandbox ID.
    pub source_sandbox_id: String,
    /// Human or template names that point to this snapshot.
    pub names: Vec<String>,
    /// Runtime snapshot artifact location.
    pub location: Option<String>,
    /// Runtime staging directory captured for restore.
    pub staging_dir: Option<String>,
    /// Virtualization.framework machine identifier bytes.
    pub machine_identifier: Option<Vec<u8>>,
    /// Captured guest network MAC addresses.
    pub network_macs: Option<Vec<String>>,
    /// Template metadata captured with the snapshot.
    #[serde(default)]
    pub template_metadata: TemplateMetadata,
}

impl SnapshotRecord {
    /// Construct a snapshot record.
    #[must_use]
    pub fn new(snapshot_id: impl Into<String>, source_sandbox_id: impl Into<String>) -> Self {
        Self {
            snapshot_id: snapshot_id.into(),
            source_sandbox_id: source_sandbox_id.into(),
            names: Vec::new(),
            location: None,
            staging_dir: None,
            machine_identifier: None,
            network_macs: None,
            template_metadata: TemplateMetadata::default(),
        }
    }

    /// Return the snapshot ID.
    #[must_use]
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    /// Return the source sandbox ID.
    #[must_use]
    pub fn source_sandbox_id(&self) -> &str {
        &self.source_sandbox_id
    }
}

/// Reference returned by runtime snapshot creation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeSnapshotRef {
    /// Snapshot ID.
    pub snapshot_id: String,
    /// Source sandbox ID captured in the snapshot.
    pub source_sandbox_id: Option<String>,
    /// Runtime snapshot artifact location.
    pub location: Option<String>,
    /// Runtime staging directory captured for restore.
    pub staging_dir: Option<String>,
    /// Virtualization.framework machine identifier bytes.
    pub machine_identifier: Option<Vec<u8>>,
    /// Captured guest network MAC addresses.
    pub network_macs: Option<Vec<String>>,
}

/// Runtime metadata captured while building a reusable template snapshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct TemplateMetadata {
    /// Environment variables captured by template build steps.
    #[serde(default)]
    pub envs: HashMap<String, String>,
    /// Optional command started after a sandbox is created from the template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_cmd: Option<String>,
    /// Optional readiness probe command for the template.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_cmd: Option<String>,
}

impl TemplateMetadata {
    /// Return environment variables captured by template build steps.
    #[must_use]
    pub fn envs(&self) -> &HashMap<String, String> {
        &self.envs
    }

    /// Return the optional start command.
    #[must_use]
    pub fn start_command(&self) -> Option<&str> {
        self.start_cmd.as_deref()
    }

    /// Return the optional readiness command.
    #[must_use]
    pub fn ready_command(&self) -> Option<&str> {
        self.ready_cmd.as_deref()
    }

    /// Return whether no template metadata has been captured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.envs.is_empty() && self.start_cmd.is_none() && self.ready_cmd.is_none()
    }

    /// Add or replace an environment variable.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.insert(key.into(), value.into());
        self
    }

    /// Set the template start command.
    #[must_use]
    pub fn with_start_command(mut self, command: impl Into<String>) -> Self {
        self.start_cmd = Some(command.into());
        self
    }

    /// Set the template readiness command.
    #[must_use]
    pub fn with_ready_command(mut self, command: impl Into<String>) -> Self {
        self.ready_cmd = Some(command.into());
        self
    }
}

/// Runtime command execution request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandRequest {
    /// Shell command to run.
    pub command: String,
    /// Optional working directory.
    pub cwd: Option<String>,
    /// Environment variables for the command.
    pub envs: HashMap<String, String>,
    /// Optional user string.
    pub user: Option<String>,
    /// Bytes to write to stdin.
    pub stdin: Vec<u8>,
}

impl CommandRequest {
    /// Construct a command request.
    #[must_use]
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: None,
            envs: HashMap::new(),
            user: None,
            stdin: Vec::new(),
        }
    }

    /// Return the command line.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Return the optional working directory.
    #[must_use]
    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// Return command environment overrides.
    #[must_use]
    pub fn env(&self) -> &HashMap<String, String> {
        &self.envs
    }

    /// Return optional standard input bytes.
    #[must_use]
    pub fn stdin(&self) -> Option<&[u8]> {
        (!self.stdin.is_empty()).then_some(self.stdin.as_slice())
    }

    /// Set the working directory.
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Add or replace an environment variable.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.insert(key.into(), value.into());
        self
    }

    /// Set the optional command user.
    #[must_use]
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Set standard input bytes.
    #[must_use]
    pub fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = stdin;
        self
    }
}

/// Runtime command execution output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    /// Captured stdout bytes.
    pub stdout: Vec<u8>,
    /// Captured stderr bytes.
    pub stderr: Vec<u8>,
    /// Process exit code.
    pub exit_code: i32,
}

impl CommandOutput {
    /// Construct command output.
    #[must_use]
    pub const fn new(stdout: Vec<u8>, stderr: Vec<u8>, exit_code: i32) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
        }
    }

    /// Return stdout bytes.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Return stderr bytes.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// Return the process exit code.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Return whether the command exited successfully.
    #[must_use]
    pub const fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Runtime log event.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct LogEvent {
    /// Unix timestamp in seconds.
    pub timestamp_unix_seconds: i64,
    /// Human-readable log message.
    pub message: String,
    /// Source label for the log event.
    pub source: String,
}

impl LogEvent {
    /// Construct a runtime log event.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            timestamp_unix_seconds: time::OffsetDateTime::now_utc().unix_timestamp(),
            message: message.into(),
            source: "single-node".to_owned(),
        }
    }

    /// Return the log message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Port route from a sandbox-visible port to a host-side runtime target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortRoute {
    sandbox_id: String,
    port: u16,
}

impl PortRoute {
    /// Construct a port route record.
    #[must_use]
    pub fn new(sandbox_id: impl Into<String>, port: u16) -> Self {
        Self {
            sandbox_id: sandbox_id.into(),
            port,
        }
    }

    /// Return the sandbox ID.
    #[must_use]
    pub fn sandbox_id(&self) -> &str {
        &self.sandbox_id
    }

    /// Return the sandbox-visible port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

/// Runtime identity assigned to a sandbox session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeIdentity {
    vm_id: String,
    container_id: String,
}

impl RuntimeIdentity {
    /// Construct a runtime identity.
    #[must_use]
    pub fn new(vm_id: impl Into<String>, container_id: impl Into<String>) -> Self {
        Self {
            vm_id: vm_id.into(),
            container_id: container_id.into(),
        }
    }

    /// Return the VM identity.
    #[must_use]
    pub fn vm_id(&self) -> &str {
        &self.vm_id
    }

    /// Return the container identity.
    #[must_use]
    pub fn container_id(&self) -> &str {
        &self.container_id
    }
}
