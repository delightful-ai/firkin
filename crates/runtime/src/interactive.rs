//! interactive — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use firkin_core::{ExecConfig, Stdio};
#[allow(unused_imports)]
use firkin_e2b_contract::BackendError;
#[allow(unused_imports)]
use firkin_envd::EnvdProcessStartRequest;
#[allow(unused_imports)]
use firkin_envd::{EnvdProcessEventStream, EnvdProcessOutput, EnvdProcessStreamEvent};
#[allow(unused_imports)]
use firkin_envd::{EnvdProcessInput, EnvdProcessSignal, EnvdPtySize};
#[allow(unused_imports)]
use firkin_trace::BenchmarkSample;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use std::sync::{Arc, Mutex as StdMutex};
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use std::time::{SystemTime, UNIX_EPOCH};
#[allow(unused_imports)]
use tokio::io::AsyncRead;
#[allow(unused_imports)]
use tokio::io::AsyncReadExt;
#[allow(unused_imports)]
use tokio::io::AsyncWriteExt;
#[allow(unused_imports)]
use tokio::sync::mpsc;
/// Report from starting one retained interactive process in a runtime session.
pub struct RuntimeInteractiveProcessStartReport {
    pub(crate) output: EnvdProcessOutput,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
    pub(crate) process: Box<dyn RuntimeInteractiveProcess>,
    pub(crate) event_stream: Option<EnvdProcessEventStream<BackendError>>,
}
impl RuntimeInteractiveProcessStartReport {
    /// Construct an interactive process start report.
    #[must_use]
    pub fn new(
        output: EnvdProcessOutput,
        benchmark_samples: Vec<BenchmarkSample>,
        process: Box<dyn RuntimeInteractiveProcess>,
    ) -> Self {
        Self {
            output,
            benchmark_samples,
            process,
            event_stream: None,
        }
    }
    /// Attach a live event stream for process stdout, stderr, or PTY output.
    #[must_use]
    pub fn with_event_stream(mut self, event_stream: EnvdProcessEventStream<BackendError>) -> Self {
        self.event_stream = Some(event_stream);
        self
    }
    /// Return benchmark samples recorded for process start.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
    /// Consume the report into its parts.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        EnvdProcessOutput,
        Vec<BenchmarkSample>,
        Box<dyn RuntimeInteractiveProcess>,
    ) {
        (self.output, self.benchmark_samples, self.process)
    }
    /// Consume the report into its parts, preserving any live output stream.
    #[must_use]
    pub fn into_parts_with_stream(
        self,
    ) -> (
        EnvdProcessOutput,
        Vec<BenchmarkSample>,
        Box<dyn RuntimeInteractiveProcess>,
        Option<EnvdProcessEventStream<BackendError>>,
    ) {
        (
            self.output,
            self.benchmark_samples,
            self.process,
            self.event_stream,
        )
    }
}
/// Retained interactive process handle for envd-compatible process operations.
#[async_trait]
pub trait RuntimeInteractiveProcess: Send {
    /// Send process input.
    async fn send_input(&mut self, input: EnvdProcessInput) -> Result<(), BackendError>;
    /// Close process stdin.
    async fn close_stdin(&mut self) -> Result<(), BackendError>;
    /// Send a process signal.
    async fn signal(&mut self, signal: EnvdProcessSignal) -> Result<(), BackendError>;
    /// Update PTY settings.
    async fn update_pty(&mut self, pty: Option<EnvdPtySize>) -> Result<(), BackendError>;
    /// Return currently captured process output.
    async fn connect(&mut self) -> Result<EnvdProcessOutput, BackendError>;
}
/// Session-owned interactive process starter.
#[async_trait]
pub trait RuntimeInteractiveProcessRunner {
    /// Error returned by the process starter implementation.
    type Error;
    /// Start a retained interactive process in the runtime session.
    async fn start_interactive_process(
        &mut self,
        request: &EnvdProcessStartRequest,
    ) -> Result<RuntimeInteractiveProcessStartReport, Self::Error>;
}
#[async_trait]
impl RuntimeInteractiveProcessRunner for firkin_core::Container<firkin_core::Streams> {
    type Error = firkin_core::Error;
    async fn start_interactive_process(
        &mut self,
        request: &EnvdProcessStartRequest,
    ) -> Result<RuntimeInteractiveProcessStartReport, Self::Error> {
        let started = Instant::now();
        if request.pty.is_some() {
            return start_core_interactive_pty(self, request, started).await;
        }
        start_core_interactive_streams(self, request, started).await
    }
}
struct CoreInteractiveProcess {
    pub(crate) pid: u32,
    pub(crate) process: firkin_core::Process<firkin_core::Streams>,
    pub(crate) stdin: Option<firkin_core::ChildStdin>,
    pub(crate) stdout: Arc<StdMutex<Vec<u8>>>,
    pub(crate) stderr: Arc<StdMutex<Vec<u8>>>,
}
#[async_trait]
impl RuntimeInteractiveProcess for CoreInteractiveProcess {
    async fn send_input(&mut self, input: EnvdProcessInput) -> Result<(), BackendError> {
        let bytes = match input {
            EnvdProcessInput::Stdin(bytes) | EnvdProcessInput::Pty(bytes) => bytes,
        };
        let stdin = self.stdin.as_mut().ok_or_else(|| {
            BackendError::Runtime(format!("process {} stdin is closed", self.pid))
        })?;
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| BackendError::Runtime(format!("write process stdin: {error}")))?;
        stdin
            .flush()
            .await
            .map_err(|error| BackendError::Runtime(format!("flush process stdin: {error}")))
    }
    async fn close_stdin(&mut self) -> Result<(), BackendError> {
        self.stdin.take();
        Ok(())
    }
    async fn signal(&mut self, signal: EnvdProcessSignal) -> Result<(), BackendError> {
        let signal = match signal {
            EnvdProcessSignal::Unspecified | EnvdProcessSignal::Sigterm => 15,
            EnvdProcessSignal::Sigkill => 9,
            EnvdProcessSignal::Unknown(value) => value,
        };
        self.process
            .kill(firkin_core::Signal::new(signal))
            .await
            .map_err(|error| BackendError::Runtime(format!("signal process: {error}")))
    }
    async fn update_pty(&mut self, _pty: Option<EnvdPtySize>) -> Result<(), BackendError> {
        Err(BackendError::Runtime(
            "Firkin RuntimeAdapter retained process was not started with a PTY".to_owned(),
        ))
    }
    async fn connect(&mut self) -> Result<EnvdProcessOutput, BackendError> {
        Ok(EnvdProcessOutput {
            pid: self.pid,
            stdout: self
                .stdout
                .lock()
                .expect("interactive stdout buffer mutex poisoned")
                .clone(),
            stderr: self
                .stderr
                .lock()
                .expect("interactive stderr buffer mutex poisoned")
                .clone(),
            pty: Vec::new(),
            exit_code: 0,
            exited: false,
            status: "running".to_owned(),
            error: None,
        })
    }
}
struct CoreInteractivePtyProcess {
    pub(crate) pid: u32,
    pub(crate) process: firkin_core::Process<firkin_core::Pty>,
    input: Option<firkin_core::PtyInput>,
    control: firkin_core::PtyControl,
    pub(crate) output: Arc<StdMutex<Vec<u8>>>,
}
#[async_trait]
impl RuntimeInteractiveProcess for CoreInteractivePtyProcess {
    async fn send_input(&mut self, input: EnvdProcessInput) -> Result<(), BackendError> {
        let bytes = match input {
            EnvdProcessInput::Stdin(bytes) | EnvdProcessInput::Pty(bytes) => bytes,
        };
        let pty = self
            .input
            .as_mut()
            .ok_or_else(|| BackendError::Runtime(format!("process {} PTY is closed", self.pid)))?;
        pty.write_all(&bytes)
            .await
            .map_err(|error| BackendError::Runtime(format!("write process pty: {error}")))?;
        pty.flush()
            .await
            .map_err(|error| BackendError::Runtime(format!("flush process pty: {error}")))
    }
    async fn close_stdin(&mut self) -> Result<(), BackendError> {
        self.input.take();
        Ok(())
    }
    async fn signal(&mut self, signal: EnvdProcessSignal) -> Result<(), BackendError> {
        let signal = match signal {
            EnvdProcessSignal::Unspecified | EnvdProcessSignal::Sigterm => 15,
            EnvdProcessSignal::Sigkill => 9,
            EnvdProcessSignal::Unknown(value) => value,
        };
        self.process
            .kill(firkin_core::Signal::new(signal))
            .await
            .map_err(|error| BackendError::Runtime(format!("signal process: {error}")))
    }
    async fn update_pty(&mut self, pty: Option<EnvdPtySize>) -> Result<(), BackendError> {
        let Some(size) = pty else {
            return Ok(());
        };
        let cols = u16::try_from(size.cols)
            .map_err(|_| BackendError::Runtime(format!("PTY cols {} exceed u16", size.cols)))?;
        let rows = u16::try_from(size.rows)
            .map_err(|_| BackendError::Runtime(format!("PTY rows {} exceed u16", size.rows)))?;
        self.control
            .resize(firkin_core::PtyConfig::new(cols, rows))
            .await
            .map_err(|error| BackendError::Runtime(format!("resize process pty: {error}")))
    }
    async fn connect(&mut self) -> Result<EnvdProcessOutput, BackendError> {
        Ok(EnvdProcessOutput {
            pid: self.pid,
            stdout: Vec::new(),
            stderr: Vec::new(),
            pty: self
                .output
                .lock()
                .expect("interactive pty output buffer mutex poisoned")
                .clone(),
            exit_code: 0,
            exited: false,
            status: "running".to_owned(),
            error: None,
        })
    }
}
async fn start_core_interactive_streams(
    container: &mut firkin_core::Container<firkin_core::Streams>,
    request: &EnvdProcessStartRequest,
    started: Instant,
) -> Result<RuntimeInteractiveProcessStartReport, firkin_core::Error> {
    let mut process = container
        .exec(
            runtime_command_process_id(),
            interactive_exec_builder(request).build(),
        )
        .await?;
    let pid = core_process_pid(process.pid());
    let stdin = process.take_stdin().await?;
    let stdout = Arc::new(StdMutex::new(Vec::new()));
    let (event_sender, event_receiver) = mpsc::channel(32);
    if let Some(stdout_stream) = process.take_stdout().await? {
        spawn_output_capture(
            stdout_stream,
            Arc::clone(&stdout),
            Some((event_sender.clone(), InteractiveOutputKind::Stdout)),
        );
    }
    let stderr = Arc::new(StdMutex::new(Vec::new()));
    if let Some(stderr_stream) = process.take_stderr().await? {
        spawn_output_capture(
            stderr_stream,
            Arc::clone(&stderr),
            Some((event_sender, InteractiveOutputKind::Stderr)),
        );
    }
    Ok(interactive_report(
        pid,
        started,
        Box::new(CoreInteractiveProcess {
            pid,
            process,
            stdin,
            stdout,
            stderr,
        }),
    )
    .with_event_stream(EnvdProcessEventStream::from_receiver(event_receiver)))
}
async fn start_core_interactive_pty(
    container: &mut firkin_core::Container<firkin_core::Streams>,
    request: &EnvdProcessStartRequest,
    started: Instant,
) -> Result<RuntimeInteractiveProcessStartReport, firkin_core::Error> {
    let pty_size = request.pty.as_ref().expect("caller checked pty presence");
    let mut process = container
        .exec(
            runtime_command_process_id(),
            interactive_exec_builder(request)
                .pty(envd_pty_to_core(*pty_size)?)
                .build(),
        )
        .await?;
    let pid = core_process_pid(process.pid());
    let Some(pty) = process.take_pty().await? else {
        return Err(firkin_core::Error::RuntimeOperation {
            operation: "start interactive pty process",
            reason: "vminitd did not return a pty handle".to_owned(),
        });
    };
    let (event_sender, event_receiver) = mpsc::channel(32);
    Ok(interactive_report(pid, started, {
        let (input, output_stream, control) = pty.split();
        let output = Arc::new(StdMutex::new(Vec::new()));
        spawn_output_capture(
            output_stream,
            Arc::clone(&output),
            Some((event_sender, InteractiveOutputKind::Pty)),
        );
        Box::new(CoreInteractivePtyProcess {
            pid,
            process,
            input: Some(input),
            control,
            output,
        })
    })
    .with_event_stream(EnvdProcessEventStream::from_receiver(event_receiver)))
}
fn interactive_exec_builder(
    request: &EnvdProcessStartRequest,
) -> firkin_core::ExecConfigBuilder<firkin_core::CommandSet> {
    let mut argv = Vec::with_capacity(request.args.len().saturating_add(1));
    argv.push(request.cmd.clone());
    argv.extend(request.args.iter().cloned());
    let mut builder = ExecConfig::builder()
        .command(argv)
        .stdin(Stdio::Piped)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Piped);
    if let Some(cwd) = request.cwd.as_ref() {
        builder = builder.working_dir(cwd.as_str());
    }
    builder.envs(request.envs.iter())
}
fn envd_pty_to_core(size: EnvdPtySize) -> Result<firkin_core::PtyConfig, firkin_core::Error> {
    let cols = u16::try_from(size.cols).map_err(|error| firkin_core::Error::RuntimeOperation {
        operation: "convert envd pty cols",
        reason: error.to_string(),
    })?;
    let rows = u16::try_from(size.rows).map_err(|error| firkin_core::Error::RuntimeOperation {
        operation: "convert envd pty rows",
        reason: error.to_string(),
    })?;
    Ok(firkin_core::PtyConfig::new(cols, rows))
}
fn core_process_pid(pid: Option<i32>) -> u32 {
    pid.and_then(|pid| u32::try_from(pid).ok()).unwrap_or(0)
}
fn interactive_report(
    pid: u32,
    started: Instant,
    process: Box<dyn RuntimeInteractiveProcess>,
) -> RuntimeInteractiveProcessStartReport {
    RuntimeInteractiveProcessStartReport::new(
        EnvdProcessOutput {
            pid,
            stdout: Vec::new(),
            stderr: Vec::new(),
            pty: Vec::new(),
            exit_code: 0,
            exited: false,
            status: "running".to_owned(),
            error: None,
        },
        vec![BenchmarkSample::new(
            "command_start",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            started.elapsed().as_secs_f64() * 1000.0,
        )],
        process,
    )
}
#[derive(Clone, Copy)]
enum InteractiveOutputKind {
    Stdout,
    Stderr,
    Pty,
}
fn spawn_output_capture<R>(
    mut stream: R,
    output: Arc<StdMutex<Vec<u8>>>,
    event_sender: Option<(
        mpsc::Sender<Result<EnvdProcessStreamEvent, BackendError>>,
        InteractiveOutputKind,
    )>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut bytes = [0_u8; 8192];
        loop {
            match stream.read(&mut bytes).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let chunk = bytes[..read].to_vec();
                    output
                        .lock()
                        .expect("interactive output buffer mutex poisoned")
                        .extend_from_slice(&chunk);
                    if let Some((sender, kind)) = &event_sender {
                        let event = match kind {
                            InteractiveOutputKind::Stdout => EnvdProcessStreamEvent::Stdout(chunk),
                            InteractiveOutputKind::Stderr => EnvdProcessStreamEvent::Stderr(chunk),
                            InteractiveOutputKind::Pty => EnvdProcessStreamEvent::Pty(chunk),
                        };
                        send_interactive_event(sender, event).await;
                    }
                }
            }
        }
    });
}

async fn send_interactive_event(
    sender: &mpsc::Sender<Result<EnvdProcessStreamEvent, BackendError>>,
    event: EnvdProcessStreamEvent,
) {
    let item = Ok(event);
    match sender.try_send(item) {
        Err(mpsc::error::TrySendError::Full(item)) => {
            let _ = sender.send(item).await;
        }
        Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}
pub(crate) fn runtime_command_process_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("envd-{nanos}")
}
