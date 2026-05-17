//! session — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::interactive::runtime_command_process_id;
#[allow(unused_imports)]
use crate::restore::ActiveSessionReservation;
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use firkin_admission::CapacityLedger;
#[allow(unused_imports)]
use firkin_admission::ResourceBudget;
#[allow(unused_imports)]
use firkin_core::{ExecConfig, ExecStartupTiming, Stdio};
#[allow(unused_imports)]
use firkin_e2b_contract::PortProxyStream;
#[allow(unused_imports)]
use firkin_envd::DEFAULT_ENVD_PORT;
#[allow(unused_imports)]
use firkin_envd::EnvdProcessEventStream;
#[allow(unused_imports)]
use firkin_envd::EnvdProcessOutput;
#[allow(unused_imports)]
use firkin_envd::EnvdProcessStartRequest;
#[allow(unused_imports)]
use firkin_envd::EnvdProcessStreamEvent;
#[allow(unused_imports)]
use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};
#[allow(unused_imports)]
use firkin_trace::{
    BenchmarkSample, EventTraceRecorder, LifecycleClass, RuntimeProfile, SandboxEventName,
    SandboxEventTrace, WorkloadClass,
};
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use std::time::Instant;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
#[allow(unused_imports)]
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;
#[allow(unused_imports)]
use tokio::sync::mpsc;
#[allow(unused_imports)]
use tokio::sync::oneshot;
/// Request passed to the runtime session terminator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionTerminationRequest<'a> {
    session_id: &'a str,
}
impl<'a> SessionTerminationRequest<'a> {
    /// Construct a session termination request.
    #[must_use]
    pub const fn new(session_id: &'a str) -> Self {
        Self { session_id }
    }
    /// Return the session id being terminated.
    #[must_use]
    pub const fn session_id(self) -> &'a str {
        self.session_id
    }
}
/// Runtime-owned adapter for stopping and deleting an active session.
pub trait SessionTerminator {
    /// Error returned by the terminator implementation.
    type Error;
    /// Stop and delete the session described by `request`.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific termination errors.
    fn terminate_session(
        &mut self,
        request: &SessionTerminationRequest<'_>,
    ) -> Result<(), Self::Error>;
}
/// Session-owned port router for a restored Firkin sandbox.
#[async_trait]
pub trait RuntimePortRouter {
    /// Error returned by the router implementation.
    type Error;
    /// Connect to a guest service port owned by this runtime session.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific routing errors.
    async fn connect_port(&self, port: u16) -> Result<PortProxyStream, Self::Error>;
}
#[async_trait]
impl RuntimePortRouter for firkin_core::Container<firkin_core::Streams> {
    type Error = firkin_core::Error;
    async fn connect_port(&self, port: u16) -> Result<PortProxyStream, Self::Error> {
        self.dial_vsock(firkin_types::VsockPort::new(u32::from(port)))
            .await
            .map(|stream| Box::new(stream) as PortProxyStream)
    }
}
/// Session-owned stop hook for a restored Firkin sandbox.
#[async_trait]
pub trait RuntimeSessionStop {
    /// Error returned by the stop implementation.
    type Error;
    /// Stop the live runtime session.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific termination errors.
    async fn stop_session(&mut self) -> Result<(), Self::Error>;
}
#[async_trait]
impl RuntimeSessionStop for firkin_core::Container<firkin_core::Streams> {
    type Error = firkin_core::Error;
    async fn stop_session(&mut self) -> Result<(), Self::Error> {
        self.kill(firkin_core::Signal::new(15)).await
    }
}
/// Report from probing readiness for a restored Firkin sandbox.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeReadinessReport {
    event_traces: Vec<SandboxEventTrace>,
}

impl RuntimeReadinessReport {
    /// Construct a readiness report from raw event traces.
    #[must_use]
    pub fn new(event_traces: Vec<SandboxEventTrace>) -> Self {
        Self { event_traces }
    }

    /// Return raw readiness event traces.
    #[must_use]
    pub fn benchmark_event_traces(&self) -> &[SandboxEventTrace] {
        &self.event_traces
    }
}

/// Session-owned readiness probe for a restored Firkin sandbox.
#[async_trait]
pub trait RuntimeReadinessProbe {
    /// Error returned by the readiness probe implementation.
    type Error;
    /// Verify that the restored session's service surface is reachable.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific probe errors.
    async fn probe_ready(
        &mut self,
        event_trace: EventTraceRecorder,
    ) -> Result<RuntimeReadinessReport, Self::Error>;
}
#[async_trait]
impl RuntimeReadinessProbe for firkin_core::Container<firkin_core::Streams> {
    type Error = firkin_core::Error;
    async fn probe_ready(
        &mut self,
        mut event_trace: EventTraceRecorder,
    ) -> Result<RuntimeReadinessReport, Self::Error> {
        self.dial_vsock(firkin_types::VsockPort::new(u32::from(DEFAULT_ENVD_PORT)))
            .await
            .map(drop)?;
        event_trace.record(SandboxEventName::GuestAgentPingPassed);
        let report = RuntimeCommandRunner::run_command(
            self,
            &EnvdProcessStartRequest {
                cmd: "/bin/pwd".to_owned(),
                cwd: Some("/tmp".to_owned()),
                ..EnvdProcessStartRequest::default()
            },
            event_trace,
        )
        .await?;
        if report.output().exit_code != 0 || !report.output().stdout.starts_with(b"/tmp") {
            return Err(firkin_core::Error::RuntimeOperation {
                operation: "readiness exec probe",
                reason: report
                    .output()
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("process exited {}", report.output().exit_code)),
            });
        }
        Ok(RuntimeReadinessReport::new(
            report.benchmark_event_traces().to_vec(),
        ))
    }
}
/// Report from starting one command in a restored runtime session.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeCommandStartReport {
    pub(crate) output: EnvdProcessOutput,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
    pub(crate) event_traces: Vec<SandboxEventTrace>,
}
impl RuntimeCommandStartReport {
    /// Construct a command start report.
    #[must_use]
    pub fn new(
        output: EnvdProcessOutput,
        benchmark_samples: Vec<BenchmarkSample>,
        event_traces: Vec<SandboxEventTrace>,
    ) -> Self {
        Self {
            output,
            benchmark_samples,
            event_traces,
        }
    }
    /// Return SDK-visible command output.
    #[must_use]
    pub const fn output(&self) -> &EnvdProcessOutput {
        &self.output
    }
    /// Return benchmark samples recorded for command execution.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
    /// Return raw event traces recorded for command execution.
    #[must_use]
    pub fn benchmark_event_traces(&self) -> &[SandboxEventTrace] {
        &self.event_traces
    }
    /// Consume the report into its parts.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        EnvdProcessOutput,
        Vec<BenchmarkSample>,
        Vec<SandboxEventTrace>,
    ) {
        (self.output, self.benchmark_samples, self.event_traces)
    }
}
/// Completion report for one streamed command after process exit.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeCommandStreamCompletion {
    output: EnvdProcessOutput,
    benchmark_samples: Vec<BenchmarkSample>,
    event_traces: Vec<SandboxEventTrace>,
}
impl RuntimeCommandStreamCompletion {
    /// Construct a streamed command completion report.
    #[must_use]
    pub fn new(
        output: EnvdProcessOutput,
        benchmark_samples: Vec<BenchmarkSample>,
        event_traces: Vec<SandboxEventTrace>,
    ) -> Self {
        Self {
            output,
            benchmark_samples,
            event_traces,
        }
    }
    /// Consume the report into its parts.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        EnvdProcessOutput,
        Vec<BenchmarkSample>,
        Vec<SandboxEventTrace>,
    ) {
        (self.output, self.benchmark_samples, self.event_traces)
    }
}
/// Report returned immediately after starting one streamed command.
#[allow(missing_debug_implementations)]
pub struct RuntimeCommandStreamStartReport {
    pid: u32,
    stream: EnvdProcessEventStream<firkin_e2b_contract::BackendError>,
    completion: oneshot::Receiver<RuntimeCommandStreamCompletion>,
}
impl RuntimeCommandStreamStartReport {
    /// Construct a streamed command start report.
    #[must_use]
    pub fn new(
        pid: u32,
        stream: EnvdProcessEventStream<firkin_e2b_contract::BackendError>,
        completion: oneshot::Receiver<RuntimeCommandStreamCompletion>,
    ) -> Self {
        Self {
            pid,
            stream,
            completion,
        }
    }
    /// Consume the report into stream and completion handles.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        u32,
        EnvdProcessEventStream<firkin_e2b_contract::BackendError>,
        oneshot::Receiver<RuntimeCommandStreamCompletion>,
    ) {
        (self.pid, self.stream, self.completion)
    }
}
/// Session-owned command runner for envd-compatible process starts.
#[async_trait]
pub trait RuntimeCommandRunner {
    /// Error returned by the command runner implementation.
    type Error;
    /// Start a command in the restored runtime session.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific command execution errors.
    async fn run_command(
        &mut self,
        request: &EnvdProcessStartRequest,
        event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStartReport, Self::Error>;
}
/// Session-owned streamed command runner for envd-compatible process starts.
#[async_trait]
pub trait RuntimeCommandStreamRunner {
    /// Error returned by the streamed command runner implementation.
    type Error;
    /// Start a command and return an envd event stream before finite output is complete.
    ///
    /// # Errors
    ///
    /// Returns implementation-specific command execution errors before a PID is available.
    async fn run_command_stream(
        &mut self,
        request: &EnvdProcessStartRequest,
        event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStreamStartReport, Self::Error>;
}
#[async_trait]
impl RuntimeCommandRunner for firkin_core::Container<firkin_core::Streams> {
    type Error = firkin_core::Error;
    async fn run_command(
        &mut self,
        request: &EnvdProcessStartRequest,
        event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStartReport, Self::Error> {
        let mut event_trace = command_event_trace_for_request(request, event_trace);
        let mut argv = Vec::with_capacity(request.args.len().saturating_add(1));
        argv.push(request.cmd.clone());
        argv.extend(request.args.iter().cloned());
        let mut builder = ExecConfig::builder()
            .command(argv)
            .stdout(Stdio::Piped)
            .stderr(Stdio::Piped);
        if let Some(cwd) = request.cwd.as_ref() {
            builder = builder.working_dir(cwd.as_str());
        }
        builder = builder.envs(request.envs.iter());
        let started = Instant::now();
        let process_id = runtime_command_process_id();
        event_trace.record(SandboxEventName::ExecRequestSent);
        let mut process = self.exec(process_id, builder.build()).await?;
        event_trace.record(SandboxEventName::ProcessStarted);
        let command_start_ms = started.elapsed().as_secs_f64() * 1000.0;
        let startup_timing = process.startup_timing();
        let mut stdout = Vec::new();
        let mut first_stdout_byte_ms = None;
        if let Some(mut stdout_stream) = process.take_stdout().await? {
            let mut first = [0_u8; 1];
            let bytes_read = stdout_stream.read(&mut first).await.map_err(|error| {
                firkin_core::Error::RuntimeOperation {
                    operation: "read exec stdout",
                    reason: error.to_string(),
                }
            })?;
            if bytes_read > 0 {
                event_trace.record(SandboxEventName::FirstStdoutByte);
                if event_trace.workload() == WorkloadClass::ReadinessProbe {
                    event_trace.record(SandboxEventName::WorkspaceReady);
                    event_trace.record(SandboxEventName::ReadyProbePassed);
                }
                first_stdout_byte_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                stdout.extend_from_slice(&first[..bytes_read]);
                stdout_stream
                    .read_to_end(&mut stdout)
                    .await
                    .map_err(|error| firkin_core::Error::RuntimeOperation {
                        operation: "read exec stdout",
                        reason: error.to_string(),
                    })?;
            }
        }
        let mut stderr = Vec::new();
        if let Some(mut stderr_stream) = process.take_stderr().await? {
            stderr_stream
                .read_to_end(&mut stderr)
                .await
                .map_err(|error| firkin_core::Error::RuntimeOperation {
                    operation: "read exec stderr",
                    reason: error.to_string(),
                })?;
        }
        let status = process.wait().await?;
        event_trace.record(SandboxEventName::ProcessExited);
        if event_trace.workload() == WorkloadClass::ReadinessProbe
            && status.success()
            && first_stdout_byte_ms.is_none()
        {
            event_trace.record(SandboxEventName::WorkspaceReady);
            event_trace.record(SandboxEventName::ReadyProbePassed);
        }
        let exit_code = status.code().unwrap_or(128);
        let samples = command_benchmark_samples(
            request,
            command_start_ms,
            first_stdout_byte_ms,
            Some(startup_timing),
        );
        Ok(RuntimeCommandStartReport::new(
            EnvdProcessOutput {
                pid: process
                    .pid()
                    .and_then(|pid| u32::try_from(pid).ok())
                    .unwrap_or(0),
                stdout,
                stderr,
                pty: Vec::new(),
                exit_code,
                exited: true,
                status: if status.success() {
                    "exited".to_owned()
                } else {
                    "errored".to_owned()
                },
                error: (!status.success()).then(|| format!("process exited {exit_code}")),
            },
            samples,
            vec![event_trace.finish()],
        ))
    }
}
#[async_trait]
impl RuntimeCommandStreamRunner for firkin_core::Container<firkin_core::Streams> {
    type Error = firkin_core::Error;

    async fn run_command_stream(
        &mut self,
        request: &EnvdProcessStartRequest,
        event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStreamStartReport, Self::Error> {
        let mut event_trace = command_event_trace_for_request(request, event_trace);
        let mut argv = Vec::with_capacity(request.args.len().saturating_add(1));
        argv.push(request.cmd.clone());
        argv.extend(request.args.iter().cloned());
        let mut builder = ExecConfig::builder()
            .command(argv)
            .stdin(Stdio::Null)
            .stdout(Stdio::Piped)
            .stderr(Stdio::Piped);
        if let Some(cwd) = request.cwd.as_ref() {
            builder = builder.working_dir(cwd.as_str());
        }
        builder = builder.envs(request.envs.iter());
        let started = Instant::now();
        let process_id = runtime_command_process_id();
        event_trace.record(SandboxEventName::ExecRequestSent);
        let mut process = self.exec(process_id, builder.build()).await?;
        event_trace.record(SandboxEventName::ProcessStarted);
        let command_start_ms = started.elapsed().as_secs_f64() * 1000.0;
        let startup_timing = process.startup_timing();
        let pid = process
            .pid()
            .and_then(|pid| u32::try_from(pid).ok())
            .unwrap_or(0);
        let stdout = process.take_stdout().await?;
        let stderr = process.take_stderr().await?;

        let (sender, receiver) = mpsc::channel(32);
        sender
            .try_send(Ok(EnvdProcessStreamEvent::Start { pid }))
            .expect("fresh process event stream channel has capacity");
        let (completion_sender, completion) = oneshot::channel();
        spawn_runtime_stream_completion(RuntimeStreamCompletionInputs {
            process,
            stdout,
            stderr,
            sender,
            completion_sender,
            request: request.clone(),
            event_trace,
            started,
            command_start_ms,
            startup_timing,
            pid,
        });

        Ok(RuntimeCommandStreamStartReport::new(
            pid,
            EnvdProcessEventStream::from_receiver(receiver),
            completion,
        ))
    }
}

#[derive(Clone, Copy)]
enum RuntimeStreamOutputKind {
    Stdout,
    Stderr,
}

type RuntimeStreamReadResult = Result<(Vec<u8>, Option<f64>), firkin_e2b_contract::BackendError>;
type RuntimeStreamReadHandle = tokio::task::JoinHandle<RuntimeStreamReadResult>;

struct RuntimeStreamCompletionInputs<R, S> {
    process: firkin_core::Process<firkin_core::Streams>,
    stdout: Option<R>,
    stderr: Option<S>,
    sender: mpsc::Sender<Result<EnvdProcessStreamEvent, firkin_e2b_contract::BackendError>>,
    completion_sender: oneshot::Sender<RuntimeCommandStreamCompletion>,
    request: EnvdProcessStartRequest,
    event_trace: EventTraceRecorder,
    started: Instant,
    command_start_ms: f64,
    startup_timing: ExecStartupTiming,
    pid: u32,
}

fn spawn_runtime_stream_completion<R, S>(inputs: RuntimeStreamCompletionInputs<R, S>)
where
    R: AsyncRead + Unpin + Send + 'static,
    S: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let RuntimeStreamCompletionInputs {
            mut process,
            stdout,
            stderr,
            sender,
            completion_sender,
            request,
            mut event_trace,
            started,
            command_start_ms,
            startup_timing,
            pid,
        } = inputs;
        let stdout_task = spawn_runtime_stream_reader(
            stdout,
            RuntimeStreamOutputKind::Stdout,
            sender.clone(),
            started,
        );
        let stderr_task = spawn_runtime_stream_reader(
            stderr,
            RuntimeStreamOutputKind::Stderr,
            sender.clone(),
            started,
        );
        let status = process.wait().await;
        let stdout = join_runtime_stream_reader(stdout_task, "stdout").await;
        let stderr = join_runtime_stream_reader(stderr_task, "stderr").await;
        let (stdout, first_stdout_byte_ms) = collect_runtime_stream_read(stdout, &sender).await;
        let stderr = collect_runtime_stream_read(stderr, &sender).await.0;
        record_stream_completion_readiness(&mut event_trace, first_stdout_byte_ms);
        let (exit_code, status_text, error) = runtime_stream_exit_status(status, &mut event_trace);
        let output = EnvdProcessOutput {
            pid,
            stdout,
            stderr,
            pty: Vec::new(),
            exit_code,
            exited: true,
            status: status_text.clone(),
            error: error.clone(),
        };
        let samples = command_benchmark_samples(
            &request,
            command_start_ms,
            first_stdout_byte_ms,
            Some(startup_timing),
        );
        let completion_report =
            RuntimeCommandStreamCompletion::new(output, samples, vec![event_trace.finish()]);
        let _ = completion_sender.send(completion_report);
        let _ = sender
            .send(Ok(EnvdProcessStreamEvent::End {
                exit_code,
                exited: true,
                status: status_text,
                error,
            }))
            .await;
    });
}

async fn join_runtime_stream_reader(
    task: RuntimeStreamReadHandle,
    stream_name: &'static str,
) -> RuntimeStreamReadResult {
    task.await.unwrap_or_else(|error| {
        Err(firkin_e2b_contract::BackendError::Runtime(format!(
            "join {stream_name} reader: {error}"
        )))
    })
}

async fn collect_runtime_stream_read(
    result: RuntimeStreamReadResult,
    sender: &mpsc::Sender<Result<EnvdProcessStreamEvent, firkin_e2b_contract::BackendError>>,
) -> (Vec<u8>, Option<f64>) {
    match result {
        Ok((output, first_byte_ms)) => (output, first_byte_ms),
        Err(error) => {
            let _ = sender.send(Err(error)).await;
            (Vec::new(), None)
        }
    }
}

fn record_stream_completion_readiness(
    event_trace: &mut EventTraceRecorder,
    first_stdout_byte_ms: Option<f64>,
) {
    if let Some(value) = first_stdout_byte_ms {
        event_trace.record_at_elapsed(
            SandboxEventName::FirstStdoutByte,
            Duration::from_secs_f64(value / 1000.0),
        );
        if event_trace.workload() == WorkloadClass::ReadinessProbe {
            event_trace.record(SandboxEventName::WorkspaceReady);
            event_trace.record(SandboxEventName::ReadyProbePassed);
        }
    }
}

fn runtime_stream_exit_status(
    status: firkin_core::Result<firkin_core::ExitStatus>,
    event_trace: &mut EventTraceRecorder,
) -> (i32, String, Option<String>) {
    match status {
        Ok(status) => {
            event_trace.record(SandboxEventName::ProcessExited);
            let exit_code = status.code().unwrap_or(128);
            let status_text = if status.success() {
                "exited"
            } else {
                "errored"
            };
            (
                exit_code,
                status_text.to_owned(),
                (!status.success()).then(|| format!("process exited {exit_code}")),
            )
        }
        Err(error) => (
            128,
            "errored".to_owned(),
            Some(format!("wait failed: {error}")),
        ),
    }
}

fn spawn_runtime_stream_reader<R>(
    reader: Option<R>,
    kind: RuntimeStreamOutputKind,
    sender: mpsc::Sender<Result<EnvdProcessStreamEvent, firkin_e2b_contract::BackendError>>,
    started: Instant,
) -> RuntimeStreamReadHandle
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let Some(mut reader) = reader else {
            return Ok((Vec::new(), None));
        };
        let mut output = Vec::new();
        let mut first_byte_ms = None;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader.read(&mut buffer).await.map_err(|error| {
                firkin_e2b_contract::BackendError::Runtime(format!("read process output: {error}"))
            })?;
            if read == 0 {
                break;
            }
            if matches!(kind, RuntimeStreamOutputKind::Stdout) && first_byte_ms.is_none() {
                first_byte_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            }
            let bytes = buffer[..read].to_vec();
            output.extend_from_slice(&bytes);
            let event = match kind {
                RuntimeStreamOutputKind::Stdout => EnvdProcessStreamEvent::Stdout(bytes),
                RuntimeStreamOutputKind::Stderr => EnvdProcessStreamEvent::Stderr(bytes),
            };
            if sender.send(Ok(event)).await.is_err() {
                break;
            }
        }
        Ok((output, first_byte_ms))
    })
}

fn command_benchmark_samples(
    request: &EnvdProcessStartRequest,
    command_start_ms: f64,
    first_stdout_byte_ms: Option<f64>,
    startup_timing: Option<ExecStartupTiming>,
) -> Vec<BenchmarkSample> {
    let command_args = request.args.join("|||");
    let command_start_sample = BenchmarkSample::new(
        "command_start",
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        command_start_ms,
    )
    .with_dynamic_tag("cmd", request.cmd.clone())
    .with_dynamic_tag("args", command_args.clone());
    let mut samples = vec![command_start_sample];
    if direct_exec_request(request) {
        samples.push(
            BenchmarkSample::new(
                "exec.direct_command_start_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                command_start_ms,
            )
            .with_dynamic_tag("cmd", request.cmd.clone())
            .with_dynamic_tag("args", command_args.clone()),
        );
    } else if let Some(shell_kind) = shell_exec_kind(request) {
        samples.push(
            BenchmarkSample::new(
                "debug.exec.shell_command_start_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                command_start_ms,
            )
            .with_static_tag("shell_kind", shell_kind)
            .with_dynamic_tag("cmd", request.cmd.clone())
            .with_dynamic_tag("args", command_args.clone()),
        );
    }
    if let Some(value) = first_stdout_byte_ms {
        samples.extend(first_stdout_benchmark_samples(
            request,
            &command_args,
            value,
        ));
    }
    if let Some(timing) = startup_timing {
        samples.extend(exec_startup_timing_samples(request, &command_args, timing));
    }
    samples
}

fn exec_startup_timing_samples(
    request: &EnvdProcessStartRequest,
    command_args: &str,
    timing: ExecStartupTiming,
) -> Vec<BenchmarkSample> {
    let rows = [
        ("debug.exec.core_spec_build_ms", timing.spec_build()),
        ("debug.exec.core_stdio_prepare_ms", timing.stdio_prepare()),
        ("debug.exec.core_request_encode_ms", timing.request_encode()),
        (
            "debug.exec.core_create_process_rpc_ms",
            timing.create_process_rpc(),
        ),
        (
            "debug.exec.core_start_process_rpc_ms",
            timing.start_process_rpc(),
        ),
    ];
    let mut samples: Vec<_> = rows
        .into_iter()
        .map(|(metric, elapsed)| {
            BenchmarkSample::new(
                metric,
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                elapsed.as_secs_f64() * 1000.0,
            )
            .with_dynamic_tag("cmd", request.cmd.clone())
            .with_dynamic_tag("args", command_args.to_owned())
        })
        .collect();
    if direct_exec_request(request) {
        samples.extend(exec_startup_route_timing_samples(
            "debug.exec.direct_core",
            request,
            command_args,
            timing,
            None,
        ));
    } else if let Some(shell_kind) = shell_exec_kind(request) {
        samples.extend(exec_startup_route_timing_samples(
            "debug.exec.shell_core",
            request,
            command_args,
            timing,
            Some(shell_kind),
        ));
    }
    samples
}

fn exec_startup_route_timing_samples(
    metric_prefix: &'static str,
    request: &EnvdProcessStartRequest,
    command_args: &str,
    timing: ExecStartupTiming,
    shell_kind: Option<&'static str>,
) -> Vec<BenchmarkSample> {
    let rows = [
        ("spec_build_ms", timing.spec_build()),
        ("stdio_prepare_ms", timing.stdio_prepare()),
        ("request_encode_ms", timing.request_encode()),
        ("create_process_rpc_ms", timing.create_process_rpc()),
        ("start_process_rpc_ms", timing.start_process_rpc()),
    ];
    rows.into_iter()
        .map(|(suffix, elapsed)| {
            let metric = format!("{metric_prefix}_{suffix}");
            let mut sample = BenchmarkSample::new(
                metric,
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                elapsed.as_secs_f64() * 1000.0,
            )
            .with_dynamic_tag("cmd", request.cmd.clone())
            .with_dynamic_tag("args", command_args.to_owned());
            if let Some(shell_kind) = shell_kind {
                sample = sample.with_static_tag("shell_kind", shell_kind);
            }
            sample
        })
        .collect()
}

fn first_stdout_benchmark_samples(
    request: &EnvdProcessStartRequest,
    command_args: &str,
    value: f64,
) -> Vec<BenchmarkSample> {
    let first_stdout_sample = BenchmarkSample::new(
        "first_stdout_byte",
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        value,
    )
    .with_dynamic_tag("cmd", request.cmd.clone())
    .with_dynamic_tag("args", command_args.to_owned());
    if !direct_exec_request(request) {
        let mut samples = vec![first_stdout_sample];
        if let Some(shell_kind) = shell_exec_kind(request) {
            samples.push(
                BenchmarkSample::new(
                    "debug.exec.shell_first_stdout_byte_ms",
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    value,
                )
                .with_static_tag("shell_kind", shell_kind)
                .with_dynamic_tag("cmd", request.cmd.clone())
                .with_dynamic_tag("args", command_args.to_owned()),
            );
        }
        return samples;
    }
    vec![
        first_stdout_sample,
        BenchmarkSample::new(
            "exec.direct_first_stdout_byte_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            value,
        )
        .with_dynamic_tag("cmd", request.cmd.clone())
        .with_dynamic_tag("args", command_args.to_owned()),
    ]
}

fn command_event_trace_for_request(
    request: &EnvdProcessStartRequest,
    event_trace: EventTraceRecorder,
) -> EventTraceRecorder {
    if !event_trace.has_recorded_events() && event_trace.workload() == WorkloadClass::TinyExec {
        if direct_exec_request(request) {
            return event_trace.with_future_workload(WorkloadClass::DirectExec);
        }
        if shell_exec_kind(request).is_some() {
            return event_trace.with_future_workload(WorkloadClass::ShellExec);
        }
    }
    event_trace
}

fn direct_exec_request(request: &EnvdProcessStartRequest) -> bool {
    !matches!(
        request.cmd.as_str(),
        "/bin/sh" | "sh" | "/usr/bin/sh" | "/bin/bash" | "bash" | "/usr/bin/bash"
    )
}

fn shell_exec_kind(request: &EnvdProcessStartRequest) -> Option<&'static str> {
    match request.cmd.as_str() {
        "/bin/sh" | "sh" | "/usr/bin/sh" => Some("sh"),
        "/bin/bash" | "bash" | "/usr/bin/bash" => {
            let login = request.args.iter().any(|arg| shell_arg_enables_login(arg));
            if login {
                Some("bash_login")
            } else {
                Some("bash")
            }
        }
        _ => None,
    }
}

fn shell_arg_enables_login(arg: &str) -> bool {
    arg == "--login"
        || arg
            .strip_prefix('-')
            .is_some_and(|flags| !flags.starts_with('-') && flags.chars().any(|flag| flag == 'l'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_login_detection_accepts_shell_option_forms() {
        assert!(shell_arg_enables_login("-l"));
        assert!(shell_arg_enables_login("-lc"));
        assert!(shell_arg_enables_login("--login"));
    }

    #[test]
    fn shell_login_detection_rejects_incidental_text() {
        assert!(!shell_arg_enables_login("hello"));
        assert!(!shell_arg_enables_login("--label"));
        assert!(!shell_arg_enables_login("-c"));
    }

    #[test]
    fn exec_startup_timing_samples_preserve_phase_tags() {
        let request = EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        };
        let samples = exec_startup_timing_samples(
            &request,
            "-lc|||printf ok",
            ExecStartupTiming::new(
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(3),
                Duration::from_millis(4),
                Duration::from_millis(5),
            ),
        );

        let create_rpc = samples
            .iter()
            .find(|sample| sample.metric() == "debug.exec.core_create_process_rpc_ms")
            .expect("create-process timing sample");
        let shell_start_rpc = samples
            .iter()
            .find(|sample| sample.metric() == "debug.exec.shell_core_start_process_rpc_ms")
            .expect("shell start-process timing sample");
        assert_eq!(samples.len(), 10);
        assert!((create_rpc.value() - 4.0).abs() < f64::EPSILON);
        assert_eq!(create_rpc.tag_value("cmd"), Some("/bin/sh"));
        assert_eq!(create_rpc.tag_value("args"), Some("-lc|||printf ok"));
        assert!((shell_start_rpc.value() - 5.0).abs() < f64::EPSILON);
        assert_eq!(shell_start_rpc.tag_value("shell_kind"), Some("sh"));
    }

    #[test]
    fn exec_startup_timing_samples_preserve_direct_route_split() {
        let request = EnvdProcessStartRequest {
            cmd: "/usr/bin/printf".to_owned(),
            args: vec!["ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        };
        let samples = exec_startup_timing_samples(
            &request,
            "ok",
            ExecStartupTiming::new(
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(3),
                Duration::from_millis(4),
                Duration::from_millis(5),
            ),
        );

        let direct_start_rpc = samples
            .iter()
            .find(|sample| sample.metric() == "debug.exec.direct_core_start_process_rpc_ms")
            .expect("direct start-process timing sample");
        assert_eq!(samples.len(), 10);
        assert!((direct_start_rpc.value() - 5.0).abs() < f64::EPSILON);
        assert_eq!(direct_start_rpc.tag_value("cmd"), Some("/usr/bin/printf"));
        assert_eq!(direct_start_rpc.tag_value("args"), Some("ok"));
        assert_eq!(direct_start_rpc.tag_value("shell_kind"), None);
    }

    #[test]
    fn command_event_trace_classifies_direct_tiny_exec_as_direct_exec() {
        let request = EnvdProcessStartRequest {
            cmd: "/usr/bin/printf".to_owned(),
            args: vec!["ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        };
        let trace = EventTraceRecorder::new(
            LifecycleClass::Hot,
            WorkloadClass::TinyExec,
            RuntimeProfile::FastAgent,
        );

        let trace = command_event_trace_for_request(&request, trace);

        assert_eq!(trace.workload(), WorkloadClass::DirectExec);
    }

    #[test]
    fn command_event_trace_classifies_shell_tiny_exec_as_shell_exec() {
        let request = EnvdProcessStartRequest {
            cmd: "/bin/bash".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        };
        let trace = EventTraceRecorder::new(
            LifecycleClass::Hot,
            WorkloadClass::TinyExec,
            RuntimeProfile::FastAgent,
        );

        let trace = command_event_trace_for_request(&request, trace);

        assert_eq!(trace.workload(), WorkloadClass::ShellExec);
    }

    #[test]
    fn command_event_trace_preserves_readiness_probe_workload_for_shell_probe() {
        let request = EnvdProcessStartRequest {
            cmd: "sh".to_owned(),
            args: vec!["-c".to_owned(), "printf ready".to_owned()],
            ..EnvdProcessStartRequest::default()
        };
        let trace = EventTraceRecorder::new(
            LifecycleClass::Hot,
            WorkloadClass::ReadinessProbe,
            RuntimeProfile::FastAgent,
        );

        let trace = command_event_trace_for_request(&request, trace);

        assert_eq!(trace.workload(), WorkloadClass::ReadinessProbe);
    }

    #[test]
    fn command_event_trace_preserves_startup_trace_workload_for_first_shell_command() {
        let request = EnvdProcessStartRequest {
            cmd: "/bin/bash".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        };
        let mut trace = EventTraceRecorder::new(
            LifecycleClass::Hot,
            WorkloadClass::TinyExec,
            RuntimeProfile::FastAgent,
        );
        trace.record(SandboxEventName::PoolLeaseAcquired);

        let trace = command_event_trace_for_request(&request, trace);

        assert_eq!(trace.workload(), WorkloadClass::TinyExec);
    }
}

/// Runtime session termination error.
#[derive(Clone, Debug, PartialEq, Eq, ThisError)]
pub enum SessionTerminationError<E> {
    /// Runtime terminator failed before active capacity was released.
    #[error("session termination failed: {source}")]
    Terminate {
        /// Source terminator error.
        source: E,
    },
}
/// Report from terminating a runtime session.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionTerminationReport {
    released_budget: Option<ResourceBudget>,
    pub(crate) benchmark_samples: Vec<BenchmarkSample>,
}
impl SessionTerminationReport {
    /// Construct a session termination report.
    #[must_use]
    pub fn new(
        released_budget: Option<ResourceBudget>,
        benchmark_samples: Vec<BenchmarkSample>,
    ) -> Self {
        Self {
            released_budget,
            benchmark_samples,
        }
    }
    /// Return the released active budget.
    #[must_use]
    pub const fn released_budget(&self) -> Option<ResourceBudget> {
        self.released_budget
    }
    /// Return benchmark samples recorded during termination.
    #[must_use]
    pub fn benchmark_samples(&self) -> &[BenchmarkSample] {
        &self.benchmark_samples
    }
}
/// Runtime-level session termination orchestrator.
#[derive(Debug)]
pub struct RuntimeSessionTerminate<'a> {
    pub(crate) ledger: &'a mut CapacityLedger,
    pub(crate) reservation: &'a mut ActiveSessionReservation,
    session_id: &'a str,
}
impl<'a> RuntimeSessionTerminate<'a> {
    /// Construct a runtime-level session termination operation.
    pub fn new(
        ledger: &'a mut CapacityLedger,
        reservation: &'a mut ActiveSessionReservation,
        session_id: &'a str,
    ) -> Self {
        Self {
            ledger,
            reservation,
            session_id,
        }
    }
    /// Terminate the session, release active capacity, and record kill/delete latency.
    ///
    /// # Errors
    ///
    /// Returns [`SessionTerminationError`] when the runtime terminator fails.
    pub fn execute_with_elapsed<T>(
        self,
        terminator: &mut T,
        elapsed: Duration,
    ) -> Result<SessionTerminationReport, SessionTerminationError<T::Error>>
    where
        T: SessionTerminator,
    {
        let request = SessionTerminationRequest::new(self.session_id);
        terminator
            .terminate_session(&request)
            .map_err(|source| SessionTerminationError::Terminate { source })?;
        let released_budget = self.reservation.release_into(self.ledger);
        let sample = BenchmarkSample::new(
            "kill_delete",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            elapsed.as_secs_f64() * 1000.0,
        );
        Ok(SessionTerminationReport::new(released_budget, vec![sample]))
    }
}
