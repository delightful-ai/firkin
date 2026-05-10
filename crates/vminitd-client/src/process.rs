//! process — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Result, VminitdError};
#[allow(unused_imports)]
use crate::pb;
#[allow(unused_imports)]
use firkin_oci::Spec;
#[allow(unused_imports)]
use firkin_types::VsockPort;
#[allow(unused_imports)]
use firkin_types::{ContainerId, ProcessId};
/// Stdio ports for `CreateProcess`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessStdio {
    stdin: Option<VsockPort>,
    stdout: Option<VsockPort>,
    stderr: Option<VsockPort>,
}
impl ProcessStdio {
    /// Construct empty stdio configuration.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stdin: None,
            stdout: None,
            stderr: None,
        }
    }
    /// Set the guest-to-host stdin port.
    #[must_use]
    pub const fn stdin(mut self, port: VsockPort) -> Self {
        self.stdin = Some(port);
        self
    }
    /// Set the guest-to-host stdout port.
    #[must_use]
    pub const fn stdout(mut self, port: VsockPort) -> Self {
        self.stdout = Some(port);
        self
    }
    /// Set the guest-to-host stderr port.
    #[must_use]
    pub const fn stderr(mut self, port: VsockPort) -> Self {
        self.stderr = Some(port);
        self
    }
}
/// Typed builder for a process-centric `CreateProcess` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessCreate {
    pub(crate) id: ProcessId,
    pub(crate) container_id: ContainerId,
    stdio: ProcessStdio,
    oci_runtime_path: Option<String>,
    configuration: Spec,
    pub(crate) options: Option<Vec<u8>>,
}
impl ProcessCreate {
    /// Construct a process creation request builder.
    #[must_use]
    pub fn new(id: ProcessId, container_id: ContainerId, configuration: Spec) -> Self {
        Self {
            id,
            container_id,
            stdio: ProcessStdio::new(),
            oci_runtime_path: None,
            configuration,
            options: None,
        }
    }
    /// Set stdio ports.
    #[must_use]
    pub const fn stdio(mut self, stdio: ProcessStdio) -> Self {
        self.stdio = stdio;
        self
    }
    /// Set an OCI runtime path override.
    #[must_use]
    pub fn oci_runtime_path(mut self, path: impl Into<String>) -> Self {
        self.oci_runtime_path = Some(path.into());
        self
    }
    /// Set process creation options.
    #[must_use]
    pub fn options(mut self, options: impl Into<Vec<u8>>) -> Self {
        self.options = Some(options.into());
        self
    }
    /// Convert into the generated protobuf request.
    ///
    /// # Errors
    ///
    /// Returns [`VminitdError::EncodeSpec`] if the OCI runtime spec cannot be
    /// serialized to JSON.
    pub fn into_request(self) -> Result<pb::CreateProcessRequest> {
        let configuration = serde_json::to_vec(&self.configuration)
            .map_err(|source| VminitdError::EncodeSpec { source })?;
        Ok(pb::CreateProcessRequest {
            id: self.id.to_string(),
            container_id: Some(self.container_id.to_string()),
            stdin: self.stdio.stdin.map(VsockPort::get),
            stdout: self.stdio.stdout.map(VsockPort::get),
            stderr: self.stdio.stderr.map(VsockPort::get),
            oci_runtime_path: self.oci_runtime_path,
            configuration,
            options: self.options,
        })
    }
    /// Build a `StartProcess` request for an existing process.
    #[must_use]
    pub fn start_request(
        id: &ProcessId,
        container_id: Option<&ContainerId>,
    ) -> pb::StartProcessRequest {
        pb::StartProcessRequest {
            id: id.to_string(),
            container_id: container_id.map(ToString::to_string),
        }
    }
    /// Build a `WaitProcess` request for an existing process.
    #[must_use]
    pub fn wait_request(
        id: &ProcessId,
        container_id: Option<&ContainerId>,
    ) -> pb::WaitProcessRequest {
        pb::WaitProcessRequest {
            id: id.to_string(),
            container_id: container_id.map(ToString::to_string),
        }
    }
    /// Build a `KillProcess` request for an existing process.
    #[must_use]
    pub fn kill_request(
        id: &ProcessId,
        container_id: Option<&ContainerId>,
        signal: i32,
    ) -> pb::KillProcessRequest {
        pb::KillProcessRequest {
            id: id.to_string(),
            container_id: container_id.map(ToString::to_string),
            signal,
        }
    }
    /// Build a `DeleteProcess` request for an existing process.
    #[must_use]
    pub fn delete_request(
        id: &ProcessId,
        container_id: Option<&ContainerId>,
    ) -> pb::DeleteProcessRequest {
        pb::DeleteProcessRequest {
            id: id.to_string(),
            container_id: container_id.map(ToString::to_string),
        }
    }
    /// Build a `ResizeProcess` request for an existing process.
    #[must_use]
    pub fn resize_request(
        id: &ProcessId,
        container_id: Option<&ContainerId>,
        rows: u32,
        columns: u32,
    ) -> pb::ResizeProcessRequest {
        pb::ResizeProcessRequest {
            id: id.to_string(),
            container_id: container_id.map(ToString::to_string),
            rows,
            columns,
        }
    }
}
