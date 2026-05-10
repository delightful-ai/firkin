//! envd process protocol contracts.
#![allow(missing_docs)]
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use std::collections::BTreeMap;
#[allow(unused_imports)]
use tokio::sync::mpsc;
/// Request to start an envd process.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct EnvdProcessStartRequest {
    /// Command executable.
    pub cmd: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variables.
    pub envs: BTreeMap<String, String>,
    /// Optional working directory.
    pub cwd: Option<String>,
    /// Optional process tag.
    pub tag: Option<String>,
    /// Whether stdin should remain open.
    pub stdin: Option<bool>,
    /// Optional PTY size.
    pub pty: Option<EnvdPtySize>,
}
/// SDK-visible envd process information.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct EnvdProcessInfo {
    /// Process id.
    pub pid: u32,
    /// Optional process tag.
    pub tag: Option<String>,
    /// Command executable.
    pub cmd: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Environment variables.
    pub envs: BTreeMap<String, String>,
    /// Optional working directory.
    pub cwd: Option<String>,
}
/// envd process selector.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum EnvdProcessSelector {
    /// Select by process id.
    Pid(u32),
    /// Select by process tag.
    Tag(String),
}
/// envd process input.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum EnvdProcessInput {
    /// Standard input bytes.
    Stdin(Vec<u8>),
    /// PTY input bytes.
    Pty(Vec<u8>),
}
/// envd process signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum EnvdProcessSignal {
    /// Unspecified signal.
    Unspecified,
    /// SIGTERM.
    Sigterm,
    /// SIGKILL.
    Sigkill,
    /// Unknown raw signal value.
    Unknown(i32),
}
/// envd PTY size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct EnvdPtySize {
    /// Terminal columns.
    pub cols: u32,
    /// Terminal rows.
    pub rows: u32,
}
/// Result of an envd process start request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct EnvdProcessOutput {
    /// Process id.
    pub pid: u32,
    /// Captured stdout bytes.
    pub stdout: Vec<u8>,
    /// Captured stderr bytes.
    pub stderr: Vec<u8>,
    /// Captured PTY bytes.
    pub pty: Vec<u8>,
    /// Exit code.
    pub exit_code: i32,
    /// Whether the process exited.
    pub exited: bool,
    /// Exit status string.
    pub status: String,
    /// Optional error string.
    pub error: Option<String>,
}
/// Streaming envd process event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum EnvdProcessStreamEvent {
    /// Process started with a pid.
    Start {
        /// Process id.
        pid: u32,
    },
    /// Stdout bytes were emitted.
    Stdout(Vec<u8>),
    /// Stderr bytes were emitted.
    Stderr(Vec<u8>),
    /// PTY bytes were emitted.
    Pty(Vec<u8>),
    /// Process exited or otherwise ended.
    End {
        /// Exit code.
        exit_code: i32,
        /// Whether the process exited.
        exited: bool,
        /// Status string.
        status: String,
        /// Optional error string.
        error: Option<String>,
    },
}
/// Streaming envd process output.
#[derive(Debug)]
#[allow(private_interfaces)]
pub struct EnvdProcessEventStream<E> {
    #[allow(missing_docs)]
    pub receiver: mpsc::Receiver<Result<EnvdProcessStreamEvent, E>>,
}
impl<E> EnvdProcessEventStream<E> {
    /// Construct a process event stream from an event receiver.
    ///
    /// Runtime adapters outside this crate can use this to provide live process
    /// streams without buffering a complete [`EnvdProcessOutput`] first.
    #[must_use]
    pub fn from_receiver(receiver: mpsc::Receiver<Result<EnvdProcessStreamEvent, E>>) -> Self {
        Self { receiver }
    }
    #[allow(missing_docs)]
    pub fn from_output(output: &EnvdProcessOutput) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let events = process_output_events(output);
        for event in events {
            sender
                .try_send(Ok(event))
                .expect("fresh process event stream channel has capacity");
        }
        Self { receiver }
    }
    #[allow(missing_docs)]
    pub async fn recv(&mut self) -> Option<Result<EnvdProcessStreamEvent, E>> {
        self.receiver.recv().await
    }
}
/// Runtime adapter for the envd process Connect API.
#[async_trait]
#[allow(private_interfaces)]
pub trait EnvdProcessAdapter: Clone + Send + Sync + 'static {
    /// Error returned by this envd adapter.
    type Error: Send + 'static;

    /// List envd processes.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn list_processes(&self) -> Result<Vec<EnvdProcessInfo>, Self::Error> {
        Ok(Vec::new())
    }
    /// Send input to an envd process.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn send_process_input(
        &self,
        selector: EnvdProcessSelector,
        input: EnvdProcessInput,
    ) -> Result<(), Self::Error>;
    /// Close stdin for an envd process.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn close_process_stdin(&self, selector: EnvdProcessSelector) -> Result<(), Self::Error>;
    /// Send a signal to an envd process.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn signal_process(
        &self,
        selector: EnvdProcessSelector,
        signal: EnvdProcessSignal,
    ) -> Result<(), Self::Error>;
    /// Connect to one envd process and return finite output.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn connect_process(
        &self,
        selector: EnvdProcessSelector,
    ) -> Result<EnvdProcessOutput, Self::Error>;
    /// Update process PTY settings.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn update_process_pty(
        &self,
        selector: EnvdProcessSelector,
        pty: Option<EnvdPtySize>,
    ) -> Result<(), Self::Error>;
    /// Start one envd process and return finite output.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn start_process(
        &self,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessOutput, Self::Error>;
    /// Start one envd process and return a stream of output events.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn start_process_stream(
        &self,
        request: EnvdProcessStartRequest,
    ) -> Result<EnvdProcessEventStream<Self::Error>, Self::Error> {
        Ok(EnvdProcessEventStream::from_output(
            &self.start_process(request).await?,
        ))
    }
}
#[allow(missing_docs)]
#[allow(private_interfaces)]
pub fn process_output_events(output: &EnvdProcessOutput) -> Vec<EnvdProcessStreamEvent> {
    let mut events = vec![EnvdProcessStreamEvent::Start { pid: output.pid }];
    if !output.stdout.is_empty() {
        events.push(EnvdProcessStreamEvent::Stdout(output.stdout.clone()));
    }
    if !output.stderr.is_empty() {
        events.push(EnvdProcessStreamEvent::Stderr(output.stderr.clone()));
    }
    if !output.pty.is_empty() {
        events.push(EnvdProcessStreamEvent::Pty(output.pty.clone()));
    }
    events.push(EnvdProcessStreamEvent::End {
        exit_code: output.exit_code,
        exited: output.exited,
        status: output.status.clone(),
        error: output.error.clone(),
    });
    events
}
