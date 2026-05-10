//! E2B `RuntimeAdapter` integration tests for the Firkin runtime crate.
//!
//! These tests were written against a local, unpublished E2B Rust SDK checkout
//! during Firkin development. They stay in the tree as compatibility evidence,
//! but are disabled in the standalone release candidate until they can target a
//! published SDK crate or a small in-repo compatibility harness.
#![cfg(any())]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
#[cfg(feature = "snapshot")]
use base64::Engine as _;
#[cfg(feature = "snapshot")]
use firkin_artifacts::{ContinuationSnapshotPlan, ContinuationSnapshotReason};
#[cfg(feature = "snapshot")]
use firkin_benchmark::{RuntimeProductSoakConfig, RuntimeProductSoakRunner};
#[cfg(feature = "snapshot")]
use firkin_e2b_contract::{SandboxRuntimeConfig, SnapshotRef};
#[cfg(feature = "snapshot")]
use firkin_e2b_wire::{
    ConnectedSandbox, ControlPlaneMethod, ControlPlaneRequest, CreateSnapshotRequest,
    FollowupSandboxCreateRequest, SnapshotInfo,
};
#[cfg(feature = "snapshot")]
use firkin_evidence::SoakStep;
use firkin_runtime::{
    DiskPressureProbe, FirkinRuntimeAdapter, FirkinWarmTemplateMaintainer, RuntimeCommandRunner,
    RuntimeCommandStartReport, RuntimeCommandStreamCompletion, RuntimeCommandStreamRunner,
    RuntimeCommandStreamStartReport, RuntimeInteractiveProcess, RuntimeInteractiveProcessRunner,
    RuntimeInteractiveProcessStartReport, RuntimePortRouter, RuntimeReadinessProbe,
    RuntimeReadinessReport, RuntimeSessionStop, SnapshotRestoreRequest,
};
use firkin_template::SnapshotSinkError;
use firkin_trace::{
    BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit, EventTraceRecorder, SandboxEventName,
    SandboxEventTrace, SandboxTraceEvent,
};
use firkin_types::{NetworkPolicyRule, SandboxNetworkPolicy, Size, hostname};
use prost::Message;
use reqwest::header::CONTENT_TYPE;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use {
    firkin_admission::{CapacityLedger, ResourceBudget},
    firkin_artifacts::{SnapshotArtifactIntegrity, SnapshotArtifactManifest},
};
use {
    firkin_e2b_contract::{
        BackendError, DEFAULT_CODE_INTERPRETER_PORT, DEFAULT_MCP_PORT, PortProxyStream, PortTarget,
        PreparedTemplate, PreparedTemplateArtifactIntegrity, RuntimeAdapter, RuntimeTemplateBuild,
        StartSandboxRequest,
    },
    firkin_e2b_server::{
        ControlPlaneHttpServer, DomainProxyHttpServer, EnvdProcessHttpServer, HostEnvdAdapter,
        LocalPodRegistry, LocalRuntimeBackend, LocalRuntimeState, LocalSandboxRegistry,
        LocalTemplateRegistry, LocalVolumeRegistry,
    },
    firkin_e2b_wire::{
        SandboxCreateRequest, TemplateBuildRequest, TemplateBuildStart, TemplateBuildStatus,
    },
    firkin_envd::{
        DEFAULT_ENVD_PORT, EnvdFilesystemAdapter, EnvdFilesystemEventType, EnvdFilesystemFileType,
        EnvdProcessAdapter, EnvdProcessEventStream, EnvdProcessInput, EnvdProcessOutput,
        EnvdProcessSelector, EnvdProcessSignal, EnvdProcessStartRequest, EnvdProcessStreamEvent,
        EnvdPtySize,
    },
};

fn trace_event_names(trace: &SandboxEventTrace) -> Vec<SandboxEventName> {
    trace.events().iter().map(SandboxTraceEvent::name).collect()
}

fn trace_with_event<'a>(
    traces: &'a [SandboxEventTrace],
    event: SandboxEventName,
    description: &'static str,
) -> &'a SandboxEventTrace {
    traces
        .iter()
        .find(|trace| trace.headline_event(event).is_some())
        .unwrap_or_else(|| panic!("{description}"))
}

fn command_only_trace(traces: &[SandboxEventTrace]) -> &SandboxEventTrace {
    traces
        .iter()
        .find(|trace| {
            trace.events().len() == 4
                && trace
                    .headline_event(SandboxEventName::SnapshotRestoreStart)
                    .is_none()
                && trace
                    .headline_event(SandboxEventName::GuestAgentPingPassed)
                    .is_none()
        })
        .expect("retain command-only exec trace")
}

#[derive(Clone, Default)]
struct RecordingLauncher {
    restored_paths: Vec<PathBuf>,
    stop_log: Arc<Mutex<Vec<String>>>,
    ready_log: Arc<Mutex<Vec<String>>>,
    cleanup_log: Arc<Mutex<Vec<String>>>,
    command_log: Arc<Mutex<Vec<Vec<String>>>>,
    command_outputs: Arc<Mutex<VecDeque<EnvdProcessOutput>>>,
    interactive_outputs: Arc<Mutex<VecDeque<Vec<u8>>>>,
    input_log: Arc<Mutex<Vec<Vec<u8>>>>,
    close_log: Arc<Mutex<Vec<u32>>>,
    signal_log: Arc<Mutex<Vec<EnvdProcessSignal>>>,
    pty_log: Arc<Mutex<Vec<Option<EnvdPtySize>>>>,
    fail_ready: bool,
    restore_delay: Duration,
}

struct RecordingDiskProbe {
    available: Size,
    probed_paths: Vec<PathBuf>,
}

impl DiskPressureProbe for RecordingDiskProbe {
    type Error = std::convert::Infallible;

    fn available_disk(&mut self, path: &Path) -> Result<Size, Self::Error> {
        self.probed_paths.push(path.to_path_buf());
        Ok(self.available)
    }
}

fn ample_disk_probe() -> RecordingDiskProbe {
    RecordingDiskProbe {
        available: Size::gib(128),
        probed_paths: Vec::new(),
    }
}

#[derive(Clone, Debug)]
struct RoutedSession {
    logical_id: String,
    stop_log: Arc<Mutex<Vec<String>>>,
    ready_log: Arc<Mutex<Vec<String>>>,
    cleanup_log: Arc<Mutex<Vec<String>>>,
    command_log: Arc<Mutex<Vec<Vec<String>>>>,
    command_outputs: Arc<Mutex<VecDeque<EnvdProcessOutput>>>,
    interactive_outputs: Arc<Mutex<VecDeque<Vec<u8>>>>,
    input_log: Arc<Mutex<Vec<Vec<u8>>>>,
    close_log: Arc<Mutex<Vec<u32>>>,
    signal_log: Arc<Mutex<Vec<EnvdProcessSignal>>>,
    pty_log: Arc<Mutex<Vec<Option<EnvdPtySize>>>>,
    fail_ready: bool,
}

#[derive(Clone)]
struct HostCommandLauncher {
    root: PathBuf,
}

#[derive(Clone)]
struct HostCommandSession {
    logical_id: String,
    adapter: HostEnvdAdapter,
}

struct FakeInteractiveProcess {
    pid: u32,
    input_log: Arc<Mutex<Vec<Vec<u8>>>>,
    close_log: Arc<Mutex<Vec<u32>>>,
    signal_log: Arc<Mutex<Vec<EnvdProcessSignal>>>,
    pty_log: Arc<Mutex<Vec<Option<EnvdPtySize>>>>,
    connect_stdout: Vec<u8>,
}

fn exited_stdout(stdout: impl Into<Vec<u8>>) -> EnvdProcessOutput {
    EnvdProcessOutput {
        pid: 41,
        stdout: stdout.into(),
        stderr: Vec::new(),
        pty: Vec::new(),
        exit_code: 0,
        exited: true,
        status: "exited".to_owned(),
        error: None,
    }
}

fn exited_error(exit_code: i32, stderr: impl Into<Vec<u8>>) -> EnvdProcessOutput {
    EnvdProcessOutput {
        pid: 41,
        stdout: Vec::new(),
        stderr: stderr.into(),
        pty: Vec::new(),
        exit_code,
        exited: true,
        status: "exited".to_owned(),
        error: None,
    }
}

fn filesystem_entry(path: &str, size: i64) -> Vec<u8> {
    format!("regular file\t{size}\t81a4\t-rw-r--r--\troot\troot\t{path}\t\n").into_bytes()
}

#[derive(Clone, PartialEq, Message)]
struct TestEnvdListRequest {}

#[derive(Clone, PartialEq, Message)]
struct TestEnvdListResponse {
    #[prost(message, repeated, tag = "1")]
    processes: Vec<TestEnvdProcessInfo>,
}

#[derive(Clone, PartialEq, Message)]
struct TestEnvdProcessInfo {
    #[prost(message, optional, tag = "1")]
    config: Option<TestEnvdProcessConfig>,
    #[prost(uint32, tag = "2")]
    pid: u32,
    #[prost(string, optional, tag = "3")]
    tag: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct TestEnvdProcessConfig {
    #[prost(string, tag = "1")]
    cmd: String,
    #[prost(string, repeated, tag = "2")]
    args: Vec<String>,
    #[prost(map = "string, string", tag = "3")]
    envs: HashMap<String, String>,
    #[prost(string, optional, tag = "4")]
    cwd: Option<String>,
}

fn grpc_web_frame(flags: u8, data: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(5 + data.len());
    encoded.push(flags);
    encoded.extend_from_slice(
        &u32::try_from(data.len())
            .expect("test envelope fits in u32")
            .to_be_bytes(),
    );
    encoded.extend_from_slice(data);
    encoded
}

fn decode_grpc_web_frame(body: &[u8]) -> (u8, Vec<u8>, &[u8]) {
    assert!(body.len() >= 5);
    let flags = body[0];
    let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
    assert!(body.len() >= 5 + len);
    (flags, body[5..5 + len].to_vec(), &body[5 + len..])
}

const TEST_SNAPSHOT_ARTIFACT: &str = "/tmp/firkin-runtime-test-repo-main.vz";
static NEXT_TEST_SNAPSHOT_WRITE: AtomicU64 = AtomicU64::new(1);
#[cfg(feature = "snapshot")]
const TEST_FOLLOWUP_ARTIFACT: &str = "/tmp/firkin-runtime-test-session-1-followup.vz";

#[cfg(feature = "snapshot")]
fn runtime_continuation_test_path(snapshot_id: &str) -> PathBuf {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(snapshot_id.as_bytes());
    std::env::var_os("FIRKIN_RUNTIME_CONTINUATION_ROOT")
        .map_or_else(
            firkin_runtime::default_runtime_continuation_root,
            PathBuf::from,
        )
        .join(format!("{encoded}.vz"))
}

fn test_snapshot_integrity() -> PreparedTemplateArtifactIntegrity {
    let write_id = NEXT_TEST_SNAPSHOT_WRITE.fetch_add(1, Ordering::Relaxed);
    let temp_path = format!(
        "{TEST_SNAPSHOT_ARTIFACT}.{}.{write_id}.tmp",
        std::process::id()
    );
    std::fs::write(&temp_path, b"firkin-runtime-test-snapshot")
        .expect("write temporary test snapshot artifact");
    std::fs::rename(&temp_path, TEST_SNAPSHOT_ARTIFACT).expect("publish test snapshot artifact");
    let manifest = SnapshotArtifactManifest::base("repo-main", TEST_SNAPSHOT_ARTIFACT);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("snapshot integrity");
    PreparedTemplateArtifactIntegrity {
        size_bytes: integrity.size_bytes(),
        sha256_hex: integrity.sha256_hex().to_owned(),
    }
}

#[cfg(feature = "snapshot")]
fn test_followup_integrity() -> PreparedTemplateArtifactIntegrity {
    std::fs::write(TEST_FOLLOWUP_ARTIFACT, b"firkin-runtime-test-followup")
        .expect("write test follow-up artifact");
    let manifest = SnapshotArtifactManifest::base("session-1", TEST_FOLLOWUP_ARTIFACT);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("follow-up integrity");
    PreparedTemplateArtifactIntegrity {
        size_bytes: integrity.size_bytes(),
        sha256_hex: integrity.sha256_hex().to_owned(),
    }
}

fn local_state_with_ready_template() -> LocalRuntimeState {
    let mut templates = LocalTemplateRegistry::new("2026-05-04T00:00:00Z");
    let requested = templates.request_build(TemplateBuildRequest {
        name: Some("repo-main".to_owned()),
        ..TemplateBuildRequest::default()
    });
    templates
        .set_prepared_template(
            &requested.template_id,
            PreparedTemplate {
                template_id: requested.template_id.clone(),
                build_id: requested.build_id.clone(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            },
        )
        .expect("template exists");
    templates
        .set_build_status(
            &requested.template_id,
            &requested.build_id,
            TemplateBuildStatus::Ready,
        )
        .expect("build exists");
    LocalRuntimeState {
        sandboxes: LocalSandboxRegistry::new(),
        pods: LocalPodRegistry::new(),
        templates,
        volumes: LocalVolumeRegistry::new(),
    }
}

fn sdk_config_for_sandbox(control_url: &str, proxy_url: &str, sandbox_id: &str) -> e2b_sdk::Config {
    e2b_sdk::Config::builder()
        .api_key("local-test")
        .api_url(format!("{control_url}/"))
        .sandbox_url(format!("{proxy_url}/"))
        .sandbox_header(
            "host",
            format!("{DEFAULT_ENVD_PORT}-{sandbox_id}.cube.localhost"),
        )
        .request_timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap()
}

#[async_trait]
impl firkin_runtime::SnapshotSessionLauncher for RecordingLauncher {
    type Error = &'static str;
    type Session = RoutedSession;

    async fn restore_from_snapshot(
        &mut self,
        request: &SnapshotRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        if !self.restore_delay.is_zero() {
            tokio::time::sleep(self.restore_delay).await;
        }
        self.restored_paths
            .push(request.manifest().path().to_path_buf());
        Ok(RoutedSession {
            logical_id: request.manifest().logical_id().to_owned(),
            stop_log: Arc::clone(&self.stop_log),
            ready_log: Arc::clone(&self.ready_log),
            cleanup_log: Arc::clone(&self.cleanup_log),
            command_log: Arc::clone(&self.command_log),
            command_outputs: Arc::clone(&self.command_outputs),
            interactive_outputs: Arc::clone(&self.interactive_outputs),
            input_log: Arc::clone(&self.input_log),
            close_log: Arc::clone(&self.close_log),
            signal_log: Arc::clone(&self.signal_log),
            pty_log: Arc::clone(&self.pty_log),
            fail_ready: self.fail_ready,
        })
    }
}

#[async_trait]
impl RuntimePortRouter for RoutedSession {
    type Error = &'static str;

    async fn connect_port(&self, port: u16) -> Result<PortProxyStream, Self::Error> {
        let (mut client, server) = tokio::io::duplex(64);
        let payload = format!("{}:{port}", self.logical_id);
        tokio::spawn(async move {
            let _ = client.write_all(payload.as_bytes()).await;
        });
        Ok(Box::new(server))
    }
}

#[async_trait]
impl RuntimeSessionStop for RoutedSession {
    type Error = &'static str;

    async fn stop_session(&mut self) -> Result<(), Self::Error> {
        self.stop_log
            .lock()
            .expect("lock stop log")
            .push(self.logical_id.clone());
        Ok(())
    }
}

#[async_trait]
impl RuntimeReadinessProbe for RoutedSession {
    type Error = &'static str;

    async fn probe_ready(
        &mut self,
        mut event_trace: EventTraceRecorder,
    ) -> Result<RuntimeReadinessReport, Self::Error> {
        if self.fail_ready {
            return Err("not ready");
        }
        self.ready_log
            .lock()
            .expect("lock ready log")
            .push(self.logical_id.clone());
        event_trace.record(SandboxEventName::GuestAgentPingPassed);
        event_trace.record(SandboxEventName::WorkspaceReady);
        event_trace.record(SandboxEventName::ReadyProbePassed);
        Ok(RuntimeReadinessReport::new(vec![event_trace.finish()]))
    }
}

#[async_trait]
impl RuntimeCommandRunner for RoutedSession {
    type Error = &'static str;

    async fn run_command(
        &mut self,
        request: &EnvdProcessStartRequest,
        mut event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStartReport, Self::Error> {
        let mut argv = Vec::with_capacity(request.args.len().saturating_add(1));
        argv.push(request.cmd.clone());
        argv.extend(request.args.iter().cloned());
        self.command_log
            .lock()
            .expect("lock command log")
            .push(argv);
        let output = self
            .command_outputs
            .lock()
            .expect("lock command outputs")
            .pop_front()
            .unwrap_or(EnvdProcessOutput {
                pid: 41,
                stdout: b"command output\n".to_vec(),
                stderr: Vec::new(),
                pty: Vec::new(),
                exit_code: 0,
                exited: true,
                status: "exited".to_owned(),
                error: None,
            });
        record_command_events(&mut event_trace);
        Ok(RuntimeCommandStartReport::new(
            output,
            diagnostic_command_samples(request),
            vec![event_trace.finish()],
        ))
    }
}

#[async_trait]
impl RuntimeCommandStreamRunner for RoutedSession {
    type Error = &'static str;

    async fn run_command_stream(
        &mut self,
        request: &EnvdProcessStartRequest,
        mut event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStreamStartReport, Self::Error> {
        let mut argv = Vec::with_capacity(request.args.len().saturating_add(1));
        argv.push(request.cmd.clone());
        argv.extend(request.args.iter().cloned());
        self.command_log
            .lock()
            .expect("lock command log")
            .push(argv);
        let output = self
            .command_outputs
            .lock()
            .expect("lock command outputs")
            .pop_front()
            .unwrap_or(EnvdProcessOutput {
                pid: 41,
                stdout: b"command output\n".to_vec(),
                stderr: Vec::new(),
                pty: Vec::new(),
                exit_code: 0,
                exited: true,
                status: "exited".to_owned(),
                error: None,
            });
        record_command_events(&mut event_trace);
        let samples = diagnostic_command_samples(request);
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        sender
            .try_send(Ok(EnvdProcessStreamEvent::Start { pid: output.pid }))
            .expect("fresh stream channel has capacity");
        if !output.stdout.is_empty() {
            sender
                .try_send(Ok(EnvdProcessStreamEvent::Stdout(output.stdout.clone())))
                .expect("fresh stream channel has capacity");
        }
        sender
            .try_send(Ok(EnvdProcessStreamEvent::End {
                exit_code: output.exit_code,
                exited: output.exited,
                status: output.status.clone(),
                error: output.error.clone(),
            }))
            .expect("fresh stream channel has capacity");
        let (completion_sender, completion) = tokio::sync::oneshot::channel();
        completion_sender
            .send(RuntimeCommandStreamCompletion::new(
                output.clone(),
                samples,
                vec![event_trace.finish()],
            ))
            .expect("completion receiver is alive");
        Ok(RuntimeCommandStreamStartReport::new(
            output.pid,
            EnvdProcessEventStream::from_receiver(receiver),
            completion,
        ))
    }
}

fn record_command_events(event_trace: &mut EventTraceRecorder) {
    event_trace.record(SandboxEventName::ExecRequestSent);
    event_trace.record(SandboxEventName::ProcessStarted);
    event_trace.record(SandboxEventName::FirstStdoutByte);
    event_trace.record(SandboxEventName::ProcessExited);
}

fn diagnostic_command_samples(request: &EnvdProcessStartRequest) -> Vec<BenchmarkSample> {
    let args = request.args.join("|||");
    let mut samples = vec![
        tagged_sample("command_start", 3.0, request, &args),
        tagged_sample("first_stdout_byte", 5.0, request, &args),
    ];
    if let Some(shell_kind) = diagnostic_shell_kind(request) {
        samples.push(
            tagged_sample("debug.exec.shell_command_start_ms", 3.0, request, &args)
                .with_static_tag("shell_kind", shell_kind),
        );
        samples.push(
            tagged_sample("debug.exec.shell_first_stdout_byte_ms", 5.0, request, &args)
                .with_static_tag("shell_kind", shell_kind),
        );
    } else {
        samples.push(tagged_sample(
            "debug.exec.direct_command_start_ms",
            3.0,
            request,
            &args,
        ));
        samples.push(tagged_sample(
            "debug.exec.direct_first_stdout_byte_ms",
            5.0,
            request,
            &args,
        ));
    }
    samples
}

fn tagged_sample(
    metric: &'static str,
    value: f64,
    request: &EnvdProcessStartRequest,
    args: &str,
) -> BenchmarkSample {
    BenchmarkSample::new(
        metric,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        value,
    )
    .with_dynamic_tag("cmd", request.cmd.clone())
    .with_dynamic_tag("args", args.to_owned())
}

fn diagnostic_shell_kind(request: &EnvdProcessStartRequest) -> Option<&'static str> {
    match request.cmd.as_str() {
        "/bin/sh" | "sh" | "/usr/bin/sh" => Some("sh"),
        "/bin/bash" | "bash" | "/usr/bin/bash" => {
            if request.args.iter().any(|arg| shell_arg_enables_login(arg)) {
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

async fn wait_for_process_output(
    adapter: &FirkinRuntimeAdapter<RecordingLauncher>,
    tag: &str,
) -> EnvdProcessOutput {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match adapter
            .connect_process(EnvdProcessSelector::Tag(tag.to_owned()))
            .await
        {
            Ok(output) if output.exited => return output,
            Ok(_) | Err(BackendError::NotFound(_)) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(output) => return output,
            Err(error) => panic!("connect streamed process `{tag}` failed: {error}"),
        }
    }
}

#[async_trait]
impl RuntimeInteractiveProcess for FakeInteractiveProcess {
    async fn send_input(&mut self, input: EnvdProcessInput) -> Result<(), BackendError> {
        let bytes = match input {
            EnvdProcessInput::Stdin(bytes) | EnvdProcessInput::Pty(bytes) => bytes,
        };
        self.input_log.lock().expect("lock input log").push(bytes);
        Ok(())
    }

    async fn close_stdin(&mut self) -> Result<(), BackendError> {
        self.close_log
            .lock()
            .expect("lock close log")
            .push(self.pid);
        Ok(())
    }

    async fn signal(&mut self, signal: EnvdProcessSignal) -> Result<(), BackendError> {
        self.signal_log
            .lock()
            .expect("lock signal log")
            .push(signal);
        Ok(())
    }

    async fn update_pty(&mut self, pty: Option<EnvdPtySize>) -> Result<(), BackendError> {
        self.pty_log.lock().expect("lock pty log").push(pty);
        Ok(())
    }

    async fn connect(&mut self) -> Result<EnvdProcessOutput, BackendError> {
        Ok(EnvdProcessOutput {
            pid: self.pid,
            stdout: self.connect_stdout.clone(),
            stderr: Vec::new(),
            pty: Vec::new(),
            exit_code: 0,
            exited: false,
            status: "running".to_owned(),
            error: None,
        })
    }
}

#[async_trait]
impl RuntimeInteractiveProcessRunner for RoutedSession {
    type Error = &'static str;

    async fn start_interactive_process(
        &mut self,
        request: &EnvdProcessStartRequest,
    ) -> Result<RuntimeInteractiveProcessStartReport, Self::Error> {
        let mut argv = Vec::with_capacity(request.args.len().saturating_add(1));
        argv.push(request.cmd.clone());
        argv.extend(request.args.iter().cloned());
        self.command_log
            .lock()
            .expect("lock command log")
            .push(argv);
        Ok(RuntimeInteractiveProcessStartReport::new(
            EnvdProcessOutput {
                pid: 42,
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
                2.0,
            )],
            Box::new(FakeInteractiveProcess {
                pid: 42,
                input_log: Arc::clone(&self.input_log),
                close_log: Arc::clone(&self.close_log),
                signal_log: Arc::clone(&self.signal_log),
                pty_log: Arc::clone(&self.pty_log),
                connect_stdout: self
                    .interactive_outputs
                    .lock()
                    .expect("lock interactive outputs")
                    .pop_front()
                    .unwrap_or_else(|| b"live output\n".to_vec()),
            }),
        ))
    }
}

#[async_trait]
impl firkin_runtime::RuntimeContinuationSnapshotSource for RoutedSession {
    async fn save_continuation_snapshot(
        &self,
        path: &std::path::Path,
    ) -> Result<(), SnapshotSinkError> {
        std::fs::write(path, self.logical_id.as_bytes())
            .map_err(|source| Box::new(source) as SnapshotSinkError)
    }

    async fn cleanup_unsnapshotted_staging(&self) -> Result<(), SnapshotSinkError> {
        self.cleanup_log
            .lock()
            .expect("lock cleanup log")
            .push(self.logical_id.clone());
        Ok(())
    }
}

#[async_trait]
impl firkin_runtime::SnapshotSessionLauncher for HostCommandLauncher {
    type Error = BackendError;
    type Session = HostCommandSession;

    async fn restore_from_snapshot(
        &mut self,
        request: &SnapshotRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        Ok(HostCommandSession {
            logical_id: request.manifest().logical_id().to_owned(),
            adapter: HostEnvdAdapter::new(self.root.clone()).await?,
        })
    }
}

#[async_trait]
impl RuntimePortRouter for HostCommandSession {
    type Error = BackendError;

    async fn connect_port(&self, port: u16) -> Result<PortProxyStream, Self::Error> {
        let (mut client, server) = tokio::io::duplex(64);
        let payload = format!("{}:{port}", self.logical_id);
        tokio::spawn(async move {
            let _ = client.write_all(payload.as_bytes()).await;
        });
        Ok(Box::new(server))
    }
}

#[async_trait]
impl RuntimeSessionStop for HostCommandSession {
    type Error = BackendError;

    async fn stop_session(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[async_trait]
impl RuntimeReadinessProbe for HostCommandSession {
    type Error = BackendError;

    async fn probe_ready(
        &mut self,
        mut event_trace: EventTraceRecorder,
    ) -> Result<RuntimeReadinessReport, Self::Error> {
        event_trace.record(SandboxEventName::GuestAgentPingPassed);
        event_trace.record(SandboxEventName::WorkspaceReady);
        event_trace.record(SandboxEventName::ReadyProbePassed);
        Ok(RuntimeReadinessReport::new(vec![event_trace.finish()]))
    }
}

#[async_trait]
impl RuntimeCommandRunner for HostCommandSession {
    type Error = BackendError;

    async fn run_command(
        &mut self,
        request: &EnvdProcessStartRequest,
        mut event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStartReport, Self::Error> {
        let output = self.adapter.start_process(request.clone()).await?;
        record_command_events(&mut event_trace);
        Ok(RuntimeCommandStartReport::new(
            output,
            vec![BenchmarkSample::new(
                "command_start",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                1.0,
            )],
            vec![event_trace.finish()],
        ))
    }
}

#[async_trait]
impl RuntimeCommandStreamRunner for HostCommandSession {
    type Error = BackendError;

    async fn run_command_stream(
        &mut self,
        request: &EnvdProcessStartRequest,
        mut event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStreamStartReport, Self::Error> {
        let output = self.adapter.start_process(request.clone()).await?;
        record_command_events(&mut event_trace);
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        sender
            .try_send(Ok(EnvdProcessStreamEvent::Start { pid: output.pid }))
            .expect("fresh stream channel has capacity");
        if !output.stdout.is_empty() {
            sender
                .try_send(Ok(EnvdProcessStreamEvent::Stdout(output.stdout.clone())))
                .expect("fresh stream channel has capacity");
        }
        sender
            .try_send(Ok(EnvdProcessStreamEvent::End {
                exit_code: output.exit_code,
                exited: output.exited,
                status: output.status.clone(),
                error: output.error.clone(),
            }))
            .expect("fresh stream channel has capacity");
        let (completion_sender, completion) = tokio::sync::oneshot::channel();
        completion_sender
            .send(RuntimeCommandStreamCompletion::new(
                output.clone(),
                vec![BenchmarkSample::new(
                    "command_start",
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    1.0,
                )],
                vec![event_trace.finish()],
            ))
            .expect("completion receiver is alive");
        Ok(RuntimeCommandStreamStartReport::new(
            output.pid,
            EnvdProcessEventStream::from_receiver(receiver),
            completion,
        ))
    }
}

#[async_trait]
impl RuntimeInteractiveProcessRunner for HostCommandSession {
    type Error = BackendError;

    async fn start_interactive_process(
        &mut self,
        _request: &EnvdProcessStartRequest,
    ) -> Result<RuntimeInteractiveProcessStartReport, Self::Error> {
        Err(BackendError::Runtime(
            "host command test session does not support retained processes".to_owned(),
        ))
    }
}

#[async_trait]
impl firkin_runtime::RuntimeContinuationSnapshotSource for HostCommandSession {
    async fn save_continuation_snapshot(
        &self,
        path: &std::path::Path,
    ) -> Result<(), SnapshotSinkError> {
        std::fs::write(path, self.logical_id.as_bytes())
            .map_err(|source| Box::new(source) as SnapshotSinkError)
    }
}

#[tokio::test]
async fn runtime_adapter_active_marker_tracks_started_session_until_stop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker_root = temp.path().join("active-vms");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    )
    .with_active_vm_marker_root(&marker_root);
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };

    let sandbox = adapter.start(request).await.expect("start succeeds");

    let marker_path = marker_root.join(&sandbox.config.sandbox_id);
    let heartbeat = std::fs::read_to_string(marker_path.join("heartbeat"))
        .expect("active marker exists")
        .trim()
        .parse::<u64>()
        .expect("active marker is epoch seconds");
    let runtime_pid = std::fs::read_to_string(marker_path.join("runtime.pid"))
        .expect("active marker pid exists")
        .trim()
        .parse::<u32>()
        .expect("active marker pid is a u32");
    let runtime_executable = std::fs::read_to_string(marker_path.join("runtime.executable"))
        .expect("active marker executable exists");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("now after epoch")
        .as_secs();
    assert!(heartbeat <= now);
    assert!(now.saturating_sub(heartbeat) <= 1);
    assert_eq!(runtime_pid, std::process::id());
    assert_eq!(
        runtime_executable.trim(),
        std::env::current_exe()
            .expect("current executable")
            .display()
            .to_string()
    );

    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop succeeds");

    assert!(
        !marker_path.exists(),
        "stop removes active VM marker from reconciliation root"
    );
}

#[tokio::test]
async fn runtime_adapter_managed_roots_wire_preflight_and_active_markers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_root = temp.path().join("snapshots");
    let log_root = temp.path().join("logs");
    let marker_root = temp.path().join("active-vms");
    std::fs::create_dir_all(&snapshot_root).expect("snapshot root");
    std::fs::create_dir_all(&log_root).expect("log root");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    )
    .with_managed_runtime_roots(&snapshot_root, &log_root, &marker_root, Size::bytes(1));
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };

    let sandbox = adapter.start(request).await.expect("start succeeds");

    let marker_path = marker_root.join(&sandbox.config.sandbox_id);
    assert!(marker_path.join("heartbeat").exists());
    assert!(marker_path.join("runtime.pid").exists());
    assert!(marker_path.join("runtime.executable").exists());

    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop succeeds");
    assert!(!marker_path.exists());
}

#[tokio::test]
async fn runtime_adapter_managed_roots_reject_missing_preflight_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_root = temp.path().join("missing-snapshots");
    let log_root = temp.path().join("logs");
    let marker_root = temp.path().join("active-vms");
    std::fs::create_dir_all(&log_root).expect("log root");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    )
    .with_managed_runtime_roots(&snapshot_root, &log_root, &marker_root, Size::bytes(1));
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };

    let error = adapter
        .start(request)
        .await
        .expect_err("missing snapshot root is rejected");

    assert!(matches!(
        error,
        BackendError::Runtime(message)
            if message.contains("required runtime root is missing")
                && message.contains("missing-snapshots")
    ));
    assert!(!marker_root.exists());
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[tokio::test]
async fn runtime_adapter_refreshes_active_marker_while_session_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let marker_root = temp.path().join("active-vms");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    )
    .with_active_vm_marker_root(&marker_root)
    .with_active_vm_heartbeat_interval(Duration::from_millis(25));
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };

    let sandbox = adapter.start(request).await.expect("start succeeds");
    let marker_path = marker_root.join(&sandbox.config.sandbox_id);
    let first = std::fs::read_to_string(marker_path.join("heartbeat"))
        .expect("active marker exists")
        .trim()
        .parse::<u64>()
        .expect("active marker is epoch seconds");
    let runtime_pid = std::fs::read_to_string(marker_path.join("runtime.pid"))
        .expect("active marker pid exists")
        .trim()
        .parse::<u32>()
        .expect("active marker pid is a u32");
    assert_eq!(runtime_pid, std::process::id());
    assert!(marker_path.join("runtime.executable").exists());

    tokio::time::sleep(Duration::from_millis(1_100)).await;

    let refreshed = std::fs::read_to_string(marker_path.join("heartbeat"))
        .expect("active marker still exists")
        .trim()
        .parse::<u64>()
        .expect("active marker is epoch seconds");
    assert!(refreshed > first);

    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop succeeds");
}

#[tokio::test]
async fn runtime_adapter_readiness_failure_releases_capacity_and_stops_session() {
    let stop_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            stop_log: Arc::clone(&stop_log),
            fail_ready: true,
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };

    let error = adapter
        .start(request)
        .await
        .expect_err("readiness failure rejects start");

    assert!(matches!(
        error,
        BackendError::Runtime(message) if message == "Firkin readiness probe failed: not ready"
    ));
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
    assert_eq!(*stop_log.lock().expect("lock stop log"), vec!["repo-main"]);
    assert!(matches!(
        adapter.port_target("sbx_firkin_1", DEFAULT_ENVD_PORT).await,
        Err(BackendError::NotFound(id)) if id == "sbx_firkin_1"
    ));
}

#[tokio::test]
async fn runtime_adapter_start_restores_prepared_template_snapshot() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };

    let sandbox = adapter.start(request).await.expect("start succeeds");

    assert_eq!(sandbox.config.sandbox_id, "sbx_firkin_1");
    assert_eq!(sandbox.config.domain, "cube.localhost");
    assert_eq!(sandbox.config.envd_version, "firkin-envd");
    assert_eq!(sandbox.config.cpu_count, 2);
    assert_eq!(sandbox.config.memory_mb, 8192);
    assert_eq!(
        sandbox.exposed_ports,
        vec![
            DEFAULT_ENVD_PORT,
            DEFAULT_CODE_INTERPRETER_PORT,
            DEFAULT_MCP_PORT
        ]
    );
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64))
    );
    assert_eq!(
        adapter.restored_paths().await,
        vec![PathBuf::from(TEST_SNAPSHOT_ARTIFACT)]
    );
}

#[tokio::test]
async fn runtime_adapter_queues_active_restore_until_capacity_releases() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(1, Size::gib(1), Size::gib(10))),
        RecordingLauncher::default(),
        ResourceBudget::new(1, Size::gib(1), Size::gib(1)),
        "cube.localhost",
        "firkin-envd",
        1,
        1024,
    );
    let first = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-main".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            }),
        })
        .await
        .expect("first start succeeds");

    let queued_adapter = adapter.clone();
    let queued = tokio::spawn(async move {
        queued_adapter
            .start(StartSandboxRequest {
                create_request: SandboxCreateRequest::default(),
                prepared_template: Some(PreparedTemplate {
                    template_id: "repo-next".to_owned(),
                    build_id: "build-2".to_owned(),
                    artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                    has_envd: true,
                    artifact_integrity: Some(test_snapshot_integrity()),
                }),
            })
            .await
    });

    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        !queued.is_finished(),
        "second restore should wait for active capacity instead of rejecting immediately"
    );

    adapter
        .stop(&first.config.sandbox_id)
        .await
        .expect("stop releases active capacity");
    let second = tokio::time::timeout(Duration::from_secs(1), queued)
        .await
        .expect("queued start wakes")
        .expect("queued task joins")
        .expect("queued start succeeds after capacity release");

    assert_eq!(second.config.sandbox_id, "sbx_firkin_2");
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(1, Size::gib(1), Size::gib(1))
    );
}

#[tokio::test]
async fn runtime_adapter_build_template_hard_fails_instead_of_empty_artifact() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );

    let error = adapter
        .build_template(RuntimeTemplateBuild {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            start: TemplateBuildStart::default(),
            uploaded_files: BTreeMap::new(),
        })
        .await
        .expect_err("adapter-level build hard-fails");

    assert!(matches!(
        error,
        BackendError::Runtime(message)
            if message == "Firkin RuntimeAdapter cannot build template `repo-main/build-1` directly; run the runtime template build pipeline and register its prepared snapshot artifact"
    ));
}

#[tokio::test]
async fn runtime_adapter_start_rejects_prepared_template_integrity_mismatch_before_restore() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    std::fs::write(&snapshot_path, b"snapshot-before").expect("snapshot");
    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("snapshot integrity");
    std::fs::write(&snapshot_path, b"snapshot-after").expect("mutate snapshot");
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: snapshot_path.display().to_string(),
            has_envd: true,
            artifact_integrity: Some(PreparedTemplateArtifactIntegrity {
                size_bytes: integrity.size_bytes(),
                sha256_hex: integrity.sha256_hex().to_owned(),
            }),
        }),
    };

    let error = adapter
        .start(request)
        .await
        .expect_err("integrity mismatch rejects start");

    assert!(matches!(
        error,
        BackendError::Runtime(message)
            if message.starts_with("Firkin snapshot integrity check failed:")
    ));
    assert!(adapter.restored_paths().await.is_empty());
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[tokio::test]
async fn local_runtime_backend_create_reaches_firkin_adapter_start() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let mut templates = LocalTemplateRegistry::new("2026-05-04T00:00:00Z");
    let requested = templates.request_build(TemplateBuildRequest {
        name: Some("repo-main".to_owned()),
        ..TemplateBuildRequest::default()
    });
    let template_id = requested.template_id.clone();
    templates
        .set_prepared_template(
            &requested.template_id,
            PreparedTemplate {
                template_id: requested.template_id.clone(),
                build_id: requested.build_id.clone(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            },
        )
        .expect("template exists");
    templates
        .set_build_status(
            &requested.template_id,
            &requested.build_id,
            TemplateBuildStatus::Ready,
        )
        .expect("build exists");
    let mut backend = LocalRuntimeBackend::from_state(
        adapter.clone(),
        LocalRuntimeState {
            sandboxes: LocalSandboxRegistry::new(),
            pods: LocalPodRegistry::new(),
            templates,
            volumes: LocalVolumeRegistry::new(),
        },
    );

    let connected = backend
        .create(SandboxCreateRequest {
            template_id,
            ..SandboxCreateRequest::default()
        })
        .await
        .expect("backend create succeeds");

    assert_eq!(connected.sandbox_id, "sbx_firkin_1");
    assert_eq!(connected.domain.as_deref(), Some("cube.localhost"));
    assert_eq!(
        adapter.restored_paths().await,
        vec![PathBuf::from(TEST_SNAPSHOT_ARTIFACT)]
    );
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64))
    );
}

#[tokio::test]
async fn runtime_adapter_starts_independent_snapshot_restores_concurrently() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            restore_delay: Duration::from_millis(200),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let create_request = || StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };

    let started = Instant::now();
    let (first, second) = tokio::join!(
        adapter.start(create_request()),
        adapter.start(create_request())
    );

    assert_eq!(
        first.expect("first starts").config.sandbox_id,
        "sbx_firkin_1"
    );
    assert_eq!(
        second.expect("second starts").config.sandbox_id,
        "sbx_firkin_2"
    );
    assert!(
        started.elapsed() < Duration::from_millis(350),
        "independent restores should overlap, elapsed={:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn runtime_adapter_start_consumes_prewarmed_template_without_restore_latency() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            restore_delay: Duration::from_millis(200),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let template = PreparedTemplate {
        template_id: "repo-main".to_owned(),
        build_id: "build-1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };
    let mut disk_probe = ample_disk_probe();
    adapter
        .prewarm_template_with_disk_probe(template.clone(), &mut disk_probe)
        .await
        .expect("prewarm succeeds");

    let started = Instant::now();
    let sandbox = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(template),
        })
        .await
        .expect("start consumes warm entry");

    assert_eq!(sandbox.config.sandbox_id, "sbx_firkin_1");
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "warm checkout should avoid restore latency, elapsed={:?}",
        started.elapsed()
    );
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64))
    );
    assert!(
        adapter
            .benchmark_samples()
            .await
            .iter()
            .any(|sample| sample.metric() == "warm_pool_checkout")
    );
}

#[tokio::test]
async fn runtime_adapter_warm_pool_checkout_retains_pool_lease_trace() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let template = PreparedTemplate {
        template_id: "repo-main".to_owned(),
        build_id: "build-1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };
    let mut disk_probe = ample_disk_probe();
    adapter
        .prewarm_template_with_disk_probe(template.clone(), &mut disk_probe)
        .await
        .expect("prewarm succeeds");

    adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(template),
        })
        .await
        .expect("start consumes warm entry");

    let traces = adapter.benchmark_event_traces().await;
    assert_eq!(traces.len(), 3);
    let prewarm_ready_trace = trace_with_event(
        &traces,
        SandboxEventName::SnapshotRestoreDone,
        "retain prewarm readiness trace",
    );
    assert_eq!(
        trace_event_names(prewarm_ready_trace),
        vec![
            SandboxEventName::SnapshotRestoreDone,
            SandboxEventName::GuestAgentPingPassed,
            SandboxEventName::WorkspaceReady,
            SandboxEventName::ReadyProbePassed,
        ]
    );
    let pool_trace = trace_with_event(
        &traces,
        SandboxEventName::PoolLeaseRequested,
        "retain lease-only pool trace",
    );
    assert_eq!(
        trace_event_names(pool_trace),
        vec![
            SandboxEventName::RequestStart,
            SandboxEventName::PoolLeaseRequested,
            SandboxEventName::PoolLeaseAcquired,
        ]
    );

    let derived = firkin_evidence::derive_available_contract_metric_samples(traces);
    let pool_lease = derived
        .iter()
        .find(|sample| sample.metric() == "pool.lease_ms")
        .expect("derive pool lease metric");
    assert_eq!(
        pool_lease
            .tags()
            .iter()
            .find(|tag| tag.key() == "trust")
            .map(firkin_trace::SampleTag::value),
        Some("exact_host_event_pair")
    );
    let hot_to_ready = derived
        .iter()
        .find(|sample| sample.metric() == "start.hot_to_ready_ms")
        .expect("derive hot readiness metric");
    assert_eq!(
        hot_to_ready
            .tags()
            .iter()
            .find(|tag| tag.key() == "trust")
            .map(firkin_trace::SampleTag::value),
        Some("exact_host_event_pair")
    );
}

#[tokio::test]
async fn runtime_adapter_start_evicts_warm_template_for_active_restore_capacity() {
    let stop_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(2, Size::gib(8), Size::gib(64))),
        RecordingLauncher {
            stop_log: Arc::clone(&stop_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let warm_template = PreparedTemplate {
        template_id: "repo-warm".to_owned(),
        build_id: "build-1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };
    let mut disk_probe = ample_disk_probe();
    adapter
        .prewarm_template_with_disk_probe(warm_template, &mut disk_probe)
        .await
        .expect("prewarm consumes all capacity");

    let sandbox = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-cold".to_owned(),
                build_id: "build-2".to_owned(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            }),
        })
        .await
        .expect("active restore evicts warm entry and starts");

    assert_eq!(sandbox.config.sandbox_id, "sbx_firkin_1");
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64))
    );
    assert_eq!(*stop_log.lock().expect("lock stop log"), vec!["repo-warm"]);
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn runtime_adapter_followup_evicts_warm_template_for_active_restore_capacity() {
    let stop_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(2, Size::gib(8), Size::gib(64))),
        RecordingLauncher {
            stop_log: Arc::clone(&stop_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let mut disk_probe = ample_disk_probe();
    adapter
        .prewarm_template_with_disk_probe(
            PreparedTemplate {
                template_id: "repo-warm".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            },
            &mut disk_probe,
        )
        .await
        .expect("prewarm consumes all capacity");
    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Idle,
        TEST_FOLLOWUP_ARTIFACT,
    );

    let sandbox = adapter
        .start_followup(
            StartSandboxRequest {
                create_request: SandboxCreateRequest::default(),
                prepared_template: None,
            },
            &plan,
        )
        .await
        .expect("follow-up restore evicts warm entry and starts");

    assert_eq!(sandbox.config.sandbox_id, "sbx_firkin_1");
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64))
    );
    assert_eq!(*stop_log.lock().expect("lock stop log"), vec!["repo-warm"]);
}

#[tokio::test]
async fn runtime_adapter_maintains_warm_template_targets_and_refills_after_checkout() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let template = PreparedTemplate {
        template_id: "repo-main".to_owned(),
        build_id: "build-1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };

    let mut disk_probe = ample_disk_probe();
    let first = adapter
        .maintain_warm_templates_with_disk_probe([template.clone()], &mut disk_probe)
        .await
        .expect("first maintain succeeds");
    assert_eq!(first.maintained(), ["repo-main"]);
    assert!(first.skipped_already_warm().is_empty());
    let second = adapter
        .maintain_warm_templates_with_disk_probe([template.clone()], &mut disk_probe)
        .await
        .expect("second maintain succeeds");
    assert!(second.maintained().is_empty());
    assert_eq!(second.skipped_already_warm(), ["repo-main"]);
    assert_eq!(
        adapter.restored_paths().await,
        vec![PathBuf::from(TEST_SNAPSHOT_ARTIFACT)]
    );

    let _sandbox = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(template.clone()),
        })
        .await
        .expect("start consumes warm entry");
    let refill = adapter
        .maintain_warm_templates_with_disk_probe([template], &mut disk_probe)
        .await
        .expect("refill succeeds");

    assert_eq!(refill.maintained(), ["repo-main"]);
    assert_eq!(
        adapter.restored_paths().await,
        vec![
            PathBuf::from(TEST_SNAPSHOT_ARTIFACT),
            PathBuf::from(TEST_SNAPSHOT_ARTIFACT)
        ]
    );
}

#[tokio::test]
async fn runtime_adapter_maintains_multiple_warm_entries_for_same_template() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            restore_delay: Duration::from_millis(200),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let template = PreparedTemplate {
        template_id: "repo-main".to_owned(),
        build_id: "build-1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };

    let mut disk_probe = ample_disk_probe();
    let report = adapter
        .maintain_warm_templates_with_disk_probe(
            [template.clone(), template.clone()],
            &mut disk_probe,
        )
        .await
        .expect("maintain two warm entries");
    assert_eq!(report.maintained(), ["repo-main", "repo-main"]);
    assert_eq!(
        adapter.restored_paths().await,
        vec![
            PathBuf::from(TEST_SNAPSHOT_ARTIFACT),
            PathBuf::from(TEST_SNAPSHOT_ARTIFACT)
        ]
    );

    let started = Instant::now();
    let (first, second) = tokio::join!(
        adapter.start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(template.clone()),
        }),
        adapter.start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(template),
        })
    );

    assert!(first.is_ok());
    assert!(second.is_ok());
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "both creates should consume retained warm entries, elapsed={:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn runtime_adapter_warm_template_maintainer_refills_after_checkout() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let template = PreparedTemplate {
        template_id: "repo-main".to_owned(),
        build_id: "build-1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };
    let maintainer = FirkinWarmTemplateMaintainer::new(
        adapter.clone(),
        vec![template.clone()],
        Duration::from_millis(25),
    );

    let mut disk_probe = ample_disk_probe();
    let first = maintainer
        .run_cycle_with_disk_probe(&mut disk_probe)
        .await
        .expect("first cycle succeeds");
    assert_eq!(first.maintained(), ["repo-main"]);
    assert!(first.skipped_already_warm().is_empty());
    assert_eq!(maintainer.interval(), Duration::from_millis(25));
    assert_eq!(maintainer.targets(), std::slice::from_ref(&template));

    let _sandbox = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(template),
        })
        .await
        .expect("start consumes warm entry");
    let refill = maintainer
        .run_cycle_with_disk_probe(&mut disk_probe)
        .await
        .expect("refill cycle succeeds");

    assert_eq!(refill.maintained(), ["repo-main"]);
    assert_eq!(
        adapter.restored_paths().await,
        vec![
            PathBuf::from(TEST_SNAPSHOT_ARTIFACT),
            PathBuf::from(TEST_SNAPSHOT_ARTIFACT)
        ]
    );
}

#[tokio::test]
async fn runtime_adapter_warm_template_maintainer_background_refills_after_checkout() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let template = PreparedTemplate {
        template_id: "repo-main".to_owned(),
        build_id: "build-1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };
    let maintainer = FirkinWarmTemplateMaintainer::new(
        adapter.clone(),
        vec![template.clone()],
        Duration::from_millis(10),
    );
    let handle = maintainer.spawn_with_disk_probe(ample_disk_probe());
    tokio::time::timeout(Duration::from_secs(1), async {
        while adapter.restored_paths().await.is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("background maintainer prewarms target");

    let _sandbox = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(template),
        })
        .await
        .expect("start consumes warm entry");
    tokio::time::timeout(Duration::from_secs(1), async {
        while adapter.restored_paths().await.len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("background maintainer refills consumed target");

    handle.shutdown().await.expect("maintainer shuts down");
}

#[tokio::test]
async fn runtime_adapter_warm_template_maintainer_uses_backend_ready_templates() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let backend =
        LocalRuntimeBackend::from_state(adapter.clone(), local_state_with_ready_template());

    let maintainer =
        FirkinWarmTemplateMaintainer::from_backend(&backend, Duration::from_millis(50));

    assert_eq!(maintainer.interval(), Duration::from_millis(50));
    assert_eq!(
        maintainer.targets(),
        [PreparedTemplate {
            template_id: "tpl_1".to_owned(),
            build_id: "bld_1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }]
    );
    let mut disk_probe = ample_disk_probe();
    let report = maintainer
        .run_cycle_with_disk_probe(&mut disk_probe)
        .await
        .expect("backend-derived maintainer prewarms ready template");
    assert_eq!(report.maintained(), ["tpl_1"]);
}

#[tokio::test]
async fn runtime_adapter_prewarm_proves_template_session_ready_before_checkout() {
    let ready_log = Arc::new(Mutex::new(Vec::new()));
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let launcher = RecordingLauncher {
        ready_log: Arc::clone(&ready_log),
        command_log: Arc::clone(&command_log),
        ..RecordingLauncher::default()
    };
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        launcher,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let template = PreparedTemplate {
        template_id: "tpl_1".to_owned(),
        build_id: "bld_1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };

    let mut disk_probe = ample_disk_probe();
    adapter
        .prewarm_template_with_disk_probe(template.clone(), &mut disk_probe)
        .await
        .expect("prewarm retains ready session");

    assert_eq!(
        *ready_log.lock().expect("lock ready log"),
        vec!["tpl_1".to_owned()]
    );
    assert!(command_log.lock().expect("lock command log").is_empty());
    adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(template),
        })
        .await
        .expect("checkout consumes ready warm entry");
    assert_eq!(
        *ready_log.lock().expect("lock ready log"),
        vec!["tpl_1".to_owned()]
    );
    assert!(command_log.lock().expect("lock command log").is_empty());
}

#[tokio::test]
async fn runtime_adapter_start_checks_preflight_before_restore() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let missing_snapshot_root = tempdir.path().join("snapshots");
    let log_root = tempdir.path().join("logs");
    std::fs::create_dir_all(&log_root).expect("logs");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    )
    .with_runtime_preflight(&missing_snapshot_root, &log_root, Size::gib(10));

    let started = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(PreparedTemplate {
                template_id: "tpl_1".to_owned(),
                build_id: "bld_1".to_owned(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            }),
        })
        .await;

    assert!(matches!(
        started,
        Err(BackendError::Runtime(message)) if message.contains("required runtime root is missing")
    ));
    assert!(adapter.restored_paths().await.is_empty());
}

#[tokio::test]
async fn runtime_adapter_prewarm_checks_preflight_before_restore() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let missing_snapshot_root = tempdir.path().join("snapshots");
    let log_root = tempdir.path().join("logs");
    std::fs::create_dir_all(&log_root).expect("logs");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    )
    .with_runtime_preflight(&missing_snapshot_root, &log_root, Size::gib(10));

    let prewarmed = adapter
        .prewarm_template(PreparedTemplate {
            template_id: "tpl_1".to_owned(),
            build_id: "bld_1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        })
        .await;

    assert!(matches!(
        prewarmed,
        Err(BackendError::Runtime(message)) if message.contains("required runtime root is missing")
    ));
    assert!(adapter.restored_paths().await.is_empty());
}

#[tokio::test]
async fn runtime_adapter_prewarm_uses_warm_pool_disk_floor_before_restore() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let template = PreparedTemplate {
        template_id: "tpl_1".to_owned(),
        build_id: "bld_1".to_owned(),
        artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
        has_envd: true,
        artifact_integrity: Some(test_snapshot_integrity()),
    };
    let mut disk_probe = RecordingDiskProbe {
        available: Size::gib(15),
        probed_paths: Vec::new(),
    };

    let prewarmed = adapter
        .prewarm_template_with_disk_probe(template, &mut disk_probe)
        .await;

    assert!(matches!(
        prewarmed,
        Err(BackendError::Runtime(message))
            if message.contains("insufficient disk")
                && message.contains("20 GiB")
                && message.contains("15 GiB")
    ));
    assert_eq!(disk_probe.probed_paths, vec![PathBuf::from("/tmp")]);
    assert!(adapter.restored_paths().await.is_empty());
}

#[tokio::test]
async fn local_runtime_backend_network_policy_failure_stops_firkin_session() {
    let stop_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            stop_log: Arc::clone(&stop_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let mut templates = LocalTemplateRegistry::new("2026-05-04T00:00:00Z");
    let requested = templates.request_build(TemplateBuildRequest {
        name: Some("repo-main".to_owned()),
        ..TemplateBuildRequest::default()
    });
    templates
        .set_prepared_template(
            &requested.template_id,
            PreparedTemplate {
                template_id: requested.template_id.clone(),
                build_id: requested.build_id.clone(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            },
        )
        .expect("template exists");
    templates
        .set_build_status(
            &requested.template_id,
            &requested.build_id,
            TemplateBuildStatus::Ready,
        )
        .expect("build exists");
    let mut backend = LocalRuntimeBackend::from_state(
        adapter.clone(),
        LocalRuntimeState {
            sandboxes: LocalSandboxRegistry::new(),
            pods: LocalPodRegistry::new(),
            templates,
            volumes: LocalVolumeRegistry::new(),
        },
    );

    let created = backend
        .create(SandboxCreateRequest {
            template_id: requested.template_id,
            network: Some(SandboxNetworkPolicy::new(
                Some(true),
                [NetworkPolicyRule::new("api.example.com").expect("allow rule")],
                [NetworkPolicyRule::new("169.254.169.254").expect("deny rule")],
                Some(false),
                None,
            )),
            ..SandboxCreateRequest::default()
        })
        .await;

    assert!(matches!(
        created,
        Err(BackendError::Runtime(message))
            if message == "Firkin RuntimeAdapter does not enforce E2B network policy"
    ));
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
    assert_eq!(*stop_log.lock().expect("lock stop log"), vec!["tpl_1"]);
    assert!(matches!(
        backend.sandboxes().get("sbx_firkin_1"),
        Err(BackendError::NotFound(id)) if id == "sbx_firkin_1"
    ));
}

#[tokio::test]
async fn runtime_adapter_start_requires_prepared_template_snapshot() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: None,
    };

    let error = adapter
        .start(request)
        .await
        .expect_err("missing prepared template is rejected");

    assert!(matches!(
        error,
        BackendError::Runtime(message)
            if message == "Firkin RuntimeAdapter start requires a prepared template snapshot"
    ));
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
}

#[tokio::test]
async fn runtime_adapter_stop_releases_active_capacity() {
    let stop_log = Arc::new(Mutex::new(Vec::new()));
    let cleanup_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            stop_log: Arc::clone(&stop_log),
            cleanup_log: Arc::clone(&cleanup_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let sandbox = adapter.start(request).await.expect("start succeeds");

    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop succeeds");

    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
    assert_eq!(*stop_log.lock().expect("lock stop log"), vec!["repo-main"]);
    assert_eq!(
        *cleanup_log.lock().expect("lock cleanup log"),
        vec!["repo-main"]
    );
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn runtime_adapter_stop_retains_staging_after_continuation_snapshot() {
    let cleanup_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            cleanup_log: Arc::clone(&cleanup_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let sandbox = adapter.start(request).await.expect("start succeeds");
    let expected_path = runtime_continuation_test_path("session-cleanup");
    let _ = std::fs::remove_file(&expected_path);

    let snapshot = adapter
        .snapshot(
            &sandbox.config.sandbox_id,
            Some("session-cleanup".to_owned()),
        )
        .await
        .expect("capture continuation snapshot");
    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop succeeds");

    assert!(cleanup_log.lock().expect("lock cleanup log").is_empty());
    if let Some(location) = snapshot.location {
        let _ = std::fs::remove_file(location);
    }
}

#[tokio::test]
async fn runtime_adapter_retains_lifecycle_benchmark_samples() {
    let ready_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            ready_log: Arc::clone(&ready_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };

    let sandbox = adapter.start(request).await.expect("start succeeds");
    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop succeeds");
    let metrics = adapter
        .benchmark_samples()
        .await
        .into_iter()
        .map(|sample| sample.metric().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        metrics,
        vec!["warm_snapshot_restore", "ready_probe", "kill_delete"]
    );
    assert_eq!(
        *ready_log.lock().expect("lock ready log"),
        vec!["repo-main"]
    );
}

#[tokio::test]
async fn runtime_adapter_command_start_records_command_latency_samples() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("command starts");
    let metrics = adapter
        .benchmark_samples()
        .await
        .into_iter()
        .map(|sample| sample.metric().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(output.pid, 41);
    assert_eq!(output.stdout, b"command output\n");
    assert_eq!(
        *command_log.lock().expect("lock command log"),
        vec![vec![
            "/bin/sh".to_owned(),
            "-lc".to_owned(),
            "printf ok".to_owned()
        ]]
    );
    assert_eq!(
        metrics,
        vec![
            "warm_snapshot_restore",
            "ready_probe",
            "command_start",
            "first_stdout_byte",
            "debug.exec.shell_command_start_ms",
            "debug.exec.shell_first_stdout_byte_ms",
            "sandbox.start.resume_snapshot_to_first_stdout_ms",
            "debug.exec.sandbox_first_command_start_ms",
            "debug.exec.sandbox_first_stdout_byte_ms"
        ]
    );
}

#[tokio::test]
async fn runtime_adapter_start_process_stream_emits_events_and_persists_completion() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_outputs: Arc::new(Mutex::new(VecDeque::from([exited_stdout(
                b"streamed output\n".to_vec(),
            )]))),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let mut stream = adapter
        .start_process_stream(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "printf streamed".to_owned()],
            tag: Some("streamed".to_owned()),
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("stream starts");
    let first = stream.recv().await.expect("start event").expect("start ok");
    let second = stream
        .recv()
        .await
        .expect("stdout event")
        .expect("stdout ok");
    let third = stream.recv().await.expect("end event").expect("end ok");

    assert!(matches!(first, EnvdProcessStreamEvent::Start { pid: 41 }));
    assert!(matches!(
        second,
        EnvdProcessStreamEvent::Stdout(bytes) if bytes == b"streamed output\n"
    ));
    assert!(matches!(
        third,
        EnvdProcessStreamEvent::End {
            exit_code: 0,
            exited: true,
            ..
        }
    ));

    let connected = wait_for_process_output(&adapter, "streamed").await;
    let samples = adapter.benchmark_samples().await;

    assert_eq!(connected.stdout, b"streamed output\n");
    assert!(samples.iter().any(|sample| sample.metric()
        == "debug.exec.shell_first_stdout_byte_ms"
        && sample.tag_value("cmd") == Some("/bin/sh")));
}

#[tokio::test]
async fn runtime_adapter_direct_exec_records_tagged_diagnostic_samples() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/usr/bin/printf".to_owned(),
            args: vec!["ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("command starts");
    let samples = adapter.benchmark_samples().await;
    let direct_first_stdout = samples
        .iter()
        .find(|sample| sample.metric() == "debug.exec.direct_first_stdout_byte_ms")
        .expect("direct first stdout diagnostic sample");

    assert_eq!(output.pid, 41);
    assert_eq!(
        direct_first_stdout.tag_value("cmd"),
        Some("/usr/bin/printf")
    );
    assert_eq!(direct_first_stdout.tag_value("args"), Some("ok"));
    assert!(samples.iter().any(
        |sample| sample.metric() == "debug.exec.direct_command_start_ms"
            && sample.tag_value("cmd") == Some("/usr/bin/printf")
    ));
    assert!(samples.iter().any(|sample| sample.metric()
        == "debug.exec.sandbox_first_stdout_byte_ms"
        && sample.tag_value("cmd") == Some("/usr/bin/printf")
        && sample.tag_value("args") == Some("ok")));
}

#[tokio::test]
async fn runtime_adapter_shell_exec_records_tagged_diagnostic_samples() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/bash".to_owned(),
            args: vec!["-l".to_owned(), "-c".to_owned(), "printf ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("shell command starts");
    let samples = adapter.benchmark_samples().await;
    let shell_first_stdout = samples
        .iter()
        .find(|sample| sample.metric() == "debug.exec.shell_first_stdout_byte_ms")
        .expect("shell first stdout diagnostic sample");

    assert_eq!(output.pid, 41);
    assert_eq!(shell_first_stdout.tag_value("cmd"), Some("/bin/bash"));
    assert_eq!(
        shell_first_stdout.tag_value("shell_kind"),
        Some("bash_login")
    );
    assert_eq!(
        shell_first_stdout.tag_value("args"),
        Some("-l|||-c|||printf ok")
    );
    assert!(samples.iter().any(
        |sample| sample.metric() == "debug.exec.shell_command_start_ms"
            && sample.tag_value("shell_kind") == Some("bash_login")
    ));
    assert!(samples.iter().any(|sample| sample.metric()
        == "debug.exec.sandbox_first_stdout_byte_ms"
        && sample.tag_value("cmd") == Some("/bin/bash")
        && sample.tag_value("args") == Some("-l|||-c|||printf ok")));
}

#[tokio::test]
async fn runtime_adapter_first_command_diagnostic_records_only_once_per_sandbox() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/usr/bin/printf".to_owned(),
            args: vec!["first".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("first command starts");
    adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/usr/bin/printf".to_owned(),
            args: vec!["second".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("second command starts");

    let first_stdout_samples = adapter
        .benchmark_samples()
        .await
        .into_iter()
        .filter(|sample| sample.metric() == "debug.exec.sandbox_first_stdout_byte_ms")
        .collect::<Vec<_>>();

    assert_eq!(first_stdout_samples.len(), 1);
    assert_eq!(
        first_stdout_samples[0].tag_value("cmd"),
        Some("/usr/bin/printf")
    );
    assert_eq!(first_stdout_samples[0].tag_value("args"), Some("first"));
}

#[tokio::test]
async fn runtime_adapter_command_start_retains_raw_event_trace() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("command starts");
    adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok again".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("second command starts");
    let traces = adapter.benchmark_event_traces().await;

    assert_eq!(traces.len(), 3);
    let readiness_trace = trace_with_event(
        &traces,
        SandboxEventName::GuestAgentPingPassed,
        "retain readiness probe trace",
    );
    assert!(
        readiness_trace
            .headline_event(SandboxEventName::ReadyProbePassed)
            .is_some()
    );

    let startup_trace = trace_with_event(
        &traces,
        SandboxEventName::SnapshotRestoreStart,
        "retain startup command trace",
    );
    assert_eq!(
        trace_event_names(startup_trace),
        vec![
            SandboxEventName::RequestStart,
            SandboxEventName::SnapshotRestoreStart,
            SandboxEventName::SnapshotRestoreDone,
            SandboxEventName::ReadyProbePassed,
            SandboxEventName::ExecRequestSent,
            SandboxEventName::ProcessStarted,
            SandboxEventName::FirstStdoutByte,
            SandboxEventName::ProcessExited,
        ]
    );
    assert_eq!(
        trace_event_names(command_only_trace(&traces)),
        vec![
            SandboxEventName::ExecRequestSent,
            SandboxEventName::ProcessStarted,
            SandboxEventName::FirstStdoutByte,
            SandboxEventName::ProcessExited,
        ]
    );
    let derived = firkin_evidence::derive_available_contract_metric_samples(traces);
    let derived_metrics = derived
        .iter()
        .map(|sample| sample.metric().to_owned())
        .collect::<Vec<_>>();
    assert!(derived_metrics.contains(&"start.warm_to_first_stdout_ms".to_owned()));
    assert!(derived_metrics.contains(&"exec.command_start_ms".to_owned()));
    assert!(derived_metrics.contains(&"exec.first_stdout_byte_ms".to_owned()));
}

#[tokio::test]
async fn runtime_adapter_process_list_and_connect_return_started_process_output() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok".to_owned()],
            envs: [("ROLE".to_owned(), "worker".to_owned())].into(),
            cwd: Some("/work".to_owned()),
            tag: Some("setup".to_owned()),
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("command starts");
    let processes = adapter.list_processes().await.expect("list succeeds");
    let connected = adapter
        .connect_process(EnvdProcessSelector::Tag("setup".to_owned()))
        .await
        .expect("connect succeeds");

    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].pid, output.pid);
    assert_eq!(processes[0].tag.as_deref(), Some("setup"));
    assert_eq!(processes[0].cmd, "/bin/sh");
    assert_eq!(processes[0].args, vec!["-lc", "printf ok"]);
    assert_eq!(processes[0].envs["ROLE"], "worker");
    assert_eq!(processes[0].cwd.as_deref(), Some("/work"));
    assert_eq!(connected, output);
}

#[tokio::test]
async fn runtime_adapter_process_interactive_operations_distinguish_missing_from_completed() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");
    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok".to_owned()],
            tag: Some("setup".to_owned()),
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("command starts");

    let missing = adapter
        .close_process_stdin(EnvdProcessSelector::Pid(99))
        .await
        .expect_err("missing process rejects");
    let completed = adapter
        .close_process_stdin(EnvdProcessSelector::Pid(output.pid))
        .await
        .expect_err("completed finite process is not interactive");
    let input = adapter
        .send_process_input(
            EnvdProcessSelector::Tag("setup".to_owned()),
            EnvdProcessInput::Stdin(b"hello\n".to_vec()),
        )
        .await
        .expect_err("completed finite process has no stdin");
    let signal = adapter
        .signal_process(
            EnvdProcessSelector::Pid(output.pid),
            EnvdProcessSignal::Sigkill,
        )
        .await
        .expect_err("completed finite process cannot be signaled");
    let pty = adapter
        .update_process_pty(
            EnvdProcessSelector::Pid(output.pid),
            Some(EnvdPtySize { rows: 24, cols: 80 }),
        )
        .await
        .expect_err("completed finite process has no pty");

    assert!(matches!(missing, BackendError::NotFound(id) if id == "99"));
    assert!(matches!(
        completed,
        BackendError::Runtime(message)
            if message == "Firkin RuntimeAdapter process 41 is finite and does not keep interactive stdin"
    ));
    assert!(matches!(
        input,
        BackendError::Runtime(message)
            if message == "Firkin RuntimeAdapter process 41 is finite and does not keep interactive stdin"
    ));
    assert!(matches!(
        signal,
        BackendError::Runtime(message)
            if message == "Firkin RuntimeAdapter process 41 is finite and has already exited and cannot be signaled"
    ));
    assert!(matches!(
        pty,
        BackendError::Runtime(message)
            if message == "Firkin RuntimeAdapter process 41 is finite and does not keep an interactive PTY"
    ));
}

#[tokio::test]
async fn runtime_adapter_interactive_process_routes_input_signal_pty_and_connect() {
    let input_log = Arc::new(Mutex::new(Vec::new()));
    let close_log = Arc::new(Mutex::new(Vec::new()));
    let signal_log = Arc::new(Mutex::new(Vec::new()));
    let pty_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            input_log: Arc::clone(&input_log),
            close_log: Arc::clone(&close_log),
            signal_log: Arc::clone(&signal_log),
            pty_log: Arc::clone(&pty_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "cat".to_owned()],
            tag: Some("interactive".to_owned()),
            stdin: Some(true),
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("interactive process starts");
    adapter
        .send_process_input(
            EnvdProcessSelector::Tag("interactive".to_owned()),
            EnvdProcessInput::Stdin(b"hello\n".to_vec()),
        )
        .await
        .expect("stdin routes");
    adapter
        .update_process_pty(
            EnvdProcessSelector::Pid(output.pid),
            Some(EnvdPtySize {
                rows: 30,
                cols: 100,
            }),
        )
        .await
        .expect("pty update routes");
    let connected = adapter
        .connect_process(EnvdProcessSelector::Pid(output.pid))
        .await
        .expect("connect routes");
    adapter
        .signal_process(
            EnvdProcessSelector::Pid(output.pid),
            EnvdProcessSignal::Sigterm,
        )
        .await
        .expect("signal routes");
    adapter
        .close_process_stdin(EnvdProcessSelector::Pid(output.pid))
        .await
        .expect("close stdin routes");

    assert_eq!(output.pid, 42);
    assert!(!output.exited);
    assert_eq!(connected.stdout, b"live output\n");
    assert_eq!(
        *input_log.lock().expect("lock input log"),
        vec![b"hello\n".to_vec()]
    );
    assert_eq!(*close_log.lock().expect("lock close log"), vec![42]);
    assert_eq!(
        *signal_log.lock().expect("lock signal log"),
        vec![EnvdProcessSignal::Sigterm]
    );
    assert_eq!(
        *pty_log.lock().expect("lock pty log"),
        vec![Some(EnvdPtySize {
            rows: 30,
            cols: 100
        })]
    );
}

#[tokio::test]
async fn runtime_adapter_can_back_envd_process_http_server() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );

    let server = EnvdProcessHttpServer::new(adapter.clone()).with_access_token("envd-token");

    assert!(server.adapter().list_processes().await.unwrap().is_empty());
}

#[tokio::test]
async fn runtime_adapter_envd_http_server_serves_process_list_request() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");
    let _output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "printf ok".to_owned()],
            tag: Some("setup".to_owned()),
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("command starts");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = EnvdProcessHttpServer::new(adapter);
    let task = tokio::spawn(server.serve(listener));
    let client = reqwest::Client::new();

    let mut request = Vec::new();
    TestEnvdListRequest {}
        .encode(&mut request)
        .expect("test list request encodes");
    let response = client
        .post(format!("{base_url}/process.Process/List"))
        .header(CONTENT_TYPE, "application/grpc-web+proto")
        .body(grpc_web_frame(0, &request))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body = response.bytes().await.unwrap();
    let (message_flags, message, trailers) = decode_grpc_web_frame(&body);
    assert_eq!(message_flags, 0);
    let list = TestEnvdListResponse::decode(message.as_slice()).unwrap();
    assert_eq!(list.processes.len(), 1);
    assert_eq!(list.processes[0].pid, 41);
    assert_eq!(list.processes[0].tag.as_deref(), Some("setup"));
    assert_eq!(
        list.processes[0].config.as_ref().unwrap().args,
        vec!["-lc", "printf ok"]
    );
    let (trailer_flags, trailer, rest) = decode_grpc_web_frame(trailers);
    assert_eq!(trailer_flags, 0x80);
    assert!(
        String::from_utf8(trailer)
            .unwrap()
            .contains("grpc-status: 0")
    );
    assert!(rest.is_empty());

    task.abort();
}

#[tokio::test]
async fn runtime_adapter_envd_http_server_serves_file_read_request() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = EnvdProcessHttpServer::new(adapter);
    let task = tokio::spawn(server.serve(listener));
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{base_url}/files?path=/work/README.md"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.bytes().await.unwrap().as_ref(),
        b"command output\n"
    );
    assert_eq!(
        *command_log.lock().expect("lock command log"),
        vec![vec!["/bin/cat".to_owned(), "/work/README.md".to_owned()]]
    );

    task.abort();
}

#[tokio::test]
async fn runtime_adapter_envd_http_server_serves_file_write_request() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = EnvdProcessHttpServer::new(adapter);
    let task = tokio::spawn(server.serve(listener));
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{base_url}/files?path=/work/README.md"))
        .body("hello firkin\n")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert!(response.text().await.unwrap().contains("/work/README.md"));
    let calls = command_log.lock().expect("lock command log");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][0], "/bin/sh");
    assert_eq!(calls[0][3], "firkin-write-file");
    assert_eq!(calls[0][5], "/work/README.md");

    task.abort();
}

#[tokio::test]
async fn runtime_adapter_filesystem_read_uses_active_session_command_runner() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let bytes = adapter
        .read_file("/work/README.md".to_owned())
        .await
        .expect("read succeeds");

    assert_eq!(bytes, b"command output\n");
    assert_eq!(
        *command_log.lock().expect("lock command log"),
        vec![vec!["/bin/cat".to_owned(), "/work/README.md".to_owned()]]
    );
}

#[tokio::test]
async fn runtime_adapter_filesystem_write_uses_active_session_command_runner() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let info = adapter
        .write_file("/work/README.md".to_owned(), b"hello firkin\n".to_vec())
        .await
        .expect("write succeeds");

    assert_eq!(info.name, "README.md");
    assert_eq!(info.file_type, "file");
    assert_eq!(info.path, "/work/README.md");
    let calls = command_log.lock().expect("lock command log");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0][0], "/bin/sh");
    assert_eq!(calls[0][1], "-lc");
    assert_eq!(calls[0][3], "firkin-write-file");
    assert_eq!(calls[0][5], "/work/README.md");
}

#[tokio::test]
async fn runtime_adapter_freshness_sync_allows_reads_and_blocks_writes_until_ready() {
    let outputs = Arc::new(Mutex::new(VecDeque::from([exited_stdout(
        b"template read\n".to_vec(),
    )])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_outputs: Arc::clone(&outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let create_request = SandboxCreateRequest {
        metadata: BTreeMap::from([
            (
                "firkin.sync.branch".to_owned(),
                "refs/heads/main".to_owned(),
            ),
            ("firkin.sync.target".to_owned(), "abc123".to_owned()),
        ]),
        ..SandboxCreateRequest::default()
    };
    let sandbox = adapter
        .start(StartSandboxRequest {
            create_request,
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-main".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            }),
        })
        .await
        .expect("start succeeds");

    let read = adapter
        .read_file("/work/README.md".to_owned())
        .await
        .expect("read remains allowed while sync runs");
    assert_eq!(read, b"template read\n");
    let blocked = adapter
        .write_file("/work/README.md".to_owned(), b"changed\n".to_vec())
        .await
        .expect_err("write is blocked until freshness sync completes");
    assert!(matches!(
        blocked,
        BackendError::Runtime(message)
            if message.contains("freshness sync") && message.contains("blocks write")
    ));

    adapter
        .complete_freshness_sync(&sandbox.config.sandbox_id, "def456")
        .await
        .expect("sync completion unlocks writes");
    adapter
        .write_file("/work/README.md".to_owned(), b"changed\n".to_vec())
        .await
        .expect("write succeeds after sync");
}

#[tokio::test]
async fn runtime_adapter_freshness_sync_runs_inside_restored_session_and_unlocks_writes() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let outputs = Arc::new(Mutex::new(VecDeque::from([
        exited_stdout(Vec::new()),
        exited_stdout(Vec::new()),
        exited_stdout(Vec::new()),
        exited_stdout(Vec::new()),
    ])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let create_request = SandboxCreateRequest {
        metadata: BTreeMap::from([
            (
                "firkin.sync.branch".to_owned(),
                "refs/heads/main".to_owned(),
            ),
            ("firkin.sync.target".to_owned(), "abc123".to_owned()),
            ("firkin.sync.checkout".to_owned(), "/work/repo".to_owned()),
        ]),
        ..SandboxCreateRequest::default()
    };
    let sandbox = adapter
        .start(StartSandboxRequest {
            create_request,
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-main".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            }),
        })
        .await
        .expect("start succeeds");

    adapter
        .run_freshness_sync(&sandbox.config.sandbox_id)
        .await
        .expect("runtime freshness sync succeeds");
    adapter
        .write_file("/work/repo/README.md".to_owned(), b"changed\n".to_vec())
        .await
        .expect("write succeeds after runtime sync");

    {
        let calls = command_log.lock().expect("lock command log");
        assert_eq!(
            calls[..3],
            [
                vec![
                    "git".to_owned(),
                    "fetch".to_owned(),
                    "--quiet".to_owned(),
                    "origin".to_owned(),
                    "main".to_owned(),
                ],
                vec![
                    "git".to_owned(),
                    "checkout".to_owned(),
                    "--quiet".to_owned(),
                    "main".to_owned(),
                ],
                vec![
                    "git".to_owned(),
                    "reset".to_owned(),
                    "--hard".to_owned(),
                    "--quiet".to_owned(),
                    "abc123".to_owned(),
                ],
            ]
        );
    }
    assert!(
        adapter
            .benchmark_samples()
            .await
            .iter()
            .any(|sample| sample.metric() == "freshness_sync")
    );
}

#[tokio::test]
async fn runtime_adapter_freshness_sync_starts_automatically_after_restore() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let outputs = Arc::new(Mutex::new(VecDeque::from([
        exited_stdout(Vec::new()),
        exited_stdout(Vec::new()),
        exited_stdout(Vec::new()),
        exited_stdout(Vec::new()),
    ])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let create_request = SandboxCreateRequest {
        metadata: BTreeMap::from([
            (
                "firkin.sync.branch".to_owned(),
                "refs/heads/main".to_owned(),
            ),
            ("firkin.sync.target".to_owned(), "abc123".to_owned()),
            ("firkin.sync.checkout".to_owned(), "/work/repo".to_owned()),
        ]),
        ..SandboxCreateRequest::default()
    };
    let sandbox = adapter
        .start(StartSandboxRequest {
            create_request,
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-main".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            }),
        })
        .await
        .expect("start succeeds");

    tokio::time::timeout(Duration::from_secs(1), async {
        while command_log.lock().expect("lock command log").len() < 3 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("automatic freshness sync runs");
    adapter
        .write_file("/work/repo/README.md".to_owned(), b"changed\n".to_vec())
        .await
        .expect("write succeeds after automatic sync");

    assert_eq!(sandbox.config.sandbox_id, "sbx_firkin_1");
}

#[tokio::test]
async fn runtime_adapter_filesystem_stat_uses_active_session_command_runner() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let outputs = Arc::new(Mutex::new(VecDeque::from([exited_stdout(
        "file\t17\t33188\t-rw-r--r--\troot\troot\t/work/README.md\t\n",
    )])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let entry = adapter
        .stat_entry("/work/README.md".to_owned())
        .await
        .expect("stat succeeds");

    assert_eq!(entry.name, "README.md");
    assert_eq!(entry.path, "/work/README.md");
    assert_eq!(entry.file_type, EnvdFilesystemFileType::File);
    assert_eq!(entry.size, 17);
    assert_eq!(entry.mode, 0o100_644);
    assert_eq!(entry.permissions, "-rw-r--r--");
    assert_eq!(entry.owner, "root");
    assert_eq!(entry.group, "root");
    let calls = command_log.lock().expect("lock command log");
    assert_eq!(calls[0][0], "/bin/sh");
    assert_eq!(calls[0][3], "firkin-stat-entry");
    assert_eq!(calls[0][4], "/work/README.md");
}

#[tokio::test]
async fn runtime_adapter_filesystem_stat_missing_maps_to_not_found() {
    let outputs = Arc::new(Mutex::new(VecDeque::from([exited_error(
        1,
        "stat: cannot statx '/work/missing.txt': No such file or directory\n",
    )])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_outputs: Arc::clone(&outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let error = adapter
        .stat_entry("/work/missing.txt".to_owned())
        .await
        .expect_err("missing stat rejects");

    assert!(matches!(error, BackendError::NotFound(path) if path == "/work/missing.txt"));
}

#[tokio::test]
async fn runtime_adapter_filesystem_list_mkdir_move_and_remove_use_guest_commands() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let outputs = Arc::new(Mutex::new(VecDeque::from([
        exited_stdout("directory\t0\t16877\tdrwxr-xr-x\troot\troot\t/work/cache\t\n"),
        exited_stdout(
            "file\t11\t33188\t-rw-r--r--\troot\troot\t/work/cache/a.txt\t\n\
             directory\t0\t16877\tdrwxr-xr-x\troot\troot\t/work/cache/nested\t\n",
        ),
        exited_stdout("file\t11\t33188\t-rw-r--r--\troot\troot\t/work/cache/b.txt\t\n"),
        exited_stdout(""),
    ])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let made = adapter
        .make_dir("/work/cache".to_owned())
        .await
        .expect("mkdir succeeds");
    let entries = adapter
        .list_dir("/work/cache".to_owned(), 1)
        .await
        .expect("list succeeds");
    let moved = adapter
        .move_entry(
            "/work/cache/a.txt".to_owned(),
            "/work/cache/b.txt".to_owned(),
        )
        .await
        .expect("move succeeds");
    adapter
        .remove_entry("/work/cache/b.txt".to_owned())
        .await
        .expect("remove succeeds");

    assert_eq!(made.file_type, EnvdFilesystemFileType::Directory);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].path, "/work/cache/a.txt");
    assert_eq!(entries[1].path, "/work/cache/nested");
    assert_eq!(moved.path, "/work/cache/b.txt");
    let calls = command_log.lock().expect("lock command log");
    assert_eq!(calls[0][3], "firkin-make-dir");
    assert_eq!(calls[1][3], "firkin-list-dir");
    assert_eq!(calls[2][3], "firkin-move-entry");
    assert_eq!(calls[3][3], "firkin-remove-entry");
}

#[tokio::test]
async fn runtime_adapter_filesystem_watch_probes_guest_path() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let outputs = Arc::new(Mutex::new(VecDeque::from([exited_stdout("")])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let _sandbox = adapter.start(request).await.expect("start succeeds");

    let events = adapter
        .watch_dir("/work/cache".to_owned(), true)
        .await
        .expect("watch succeeds");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].name, "/work/cache");
    assert_eq!(events[0].event_type, EnvdFilesystemEventType::Write);
    let calls = command_log.lock().expect("lock command log");
    assert_eq!(calls[0][3], "firkin-watch-dir");
    assert_eq!(calls[0][4], "/work/cache");
    assert_eq!(calls[0][5], "true");
}

#[tokio::test]
async fn runtime_adapter_routes_envd_port_to_adapter_backed_server() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let sandbox = adapter.start(request).await.expect("start succeeds");

    let target = adapter
        .port_target(&sandbox.config.sandbox_id, DEFAULT_ENVD_PORT)
        .await
        .expect("port target exists");
    assert!(matches!(
        target,
        PortTarget::Tcp {
            ref host,
            port
        } if host == "127.0.0.1" && port > 0
    ));
}

async fn start_two_sdk_sandboxes_through_domain_proxy(
    command_log: Arc<Mutex<Vec<Vec<String>>>>,
    interactive_outputs: Arc<Mutex<VecDeque<Vec<u8>>>>,
) -> (
    e2b_sdk::Sandbox,
    e2b_sdk::Sandbox,
    tokio::task::JoinHandle<std::io::Result<()>>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let command_outputs = Arc::new(Mutex::new(VecDeque::from([
        exited_stdout("first process\n"),
        exited_stdout("second process\n"),
        exited_stdout(Vec::new()),
        exited_stdout(Vec::new()),
        exited_stdout("first"),
        exited_stdout("second"),
        exited_stdout(filesystem_entry("/tmp/first.txt", 5)),
        exited_stdout(filesystem_entry("/tmp/second.txt", 6)),
        exited_stdout(filesystem_entry("/tmp/first.txt", 5)),
        exited_stdout(filesystem_entry("/tmp/second.txt", 6)),
        exited_stdout(Vec::new()),
        exited_stdout(Vec::new()),
    ])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&command_outputs),
            interactive_outputs,
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let backend = LocalRuntimeBackend::from_state(adapter, local_state_with_ready_template());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_task = tokio::spawn(control_plane.serve(control_listener));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let first = e2b_sdk::Sandbox::create_with_config(
        sdk_config_for_sandbox(&control_url, &proxy_url, "sbx_firkin_1"),
        e2b_sdk::SandboxCreateOpts::builder()
            .template("repo-main")
            .build(),
    )
    .await
    .unwrap();
    let second = e2b_sdk::Sandbox::create_with_config(
        sdk_config_for_sandbox(&control_url, &proxy_url, "sbx_firkin_2"),
        e2b_sdk::SandboxCreateOpts::builder()
            .template("repo-main")
            .build(),
    )
    .await
    .unwrap();

    (first, second, proxy_task, control_task)
}

async fn assert_two_active_sandbox_process_routes(
    first: &e2b_sdk::Sandbox,
    second: &e2b_sdk::Sandbox,
) {
    assert_eq!(first.sandbox_id(), "sbx_firkin_1");
    assert_eq!(second.sandbox_id(), "sbx_firkin_2");
    assert_eq!(
        first
            .commands()
            .run("printf first", e2b_sdk::CommandRunOpts::default())
            .await
            .unwrap()
            .stdout,
        "first process\n"
    );
    assert_eq!(
        second
            .commands()
            .run("printf second", e2b_sdk::CommandRunOpts::default())
            .await
            .unwrap()
            .stdout,
        "second process\n"
    );
}

async fn assert_two_active_sandbox_retained_processes_do_not_collide(
    first: &e2b_sdk::Sandbox,
    second: &e2b_sdk::Sandbox,
) {
    let first_handle = first
        .commands()
        .run_background(
            "cat",
            e2b_sdk::CommandRunOpts::builder().stdin(true).build(),
        )
        .await
        .unwrap();
    let second_handle = second
        .commands()
        .run_background(
            "cat",
            e2b_sdk::CommandRunOpts::builder().stdin(true).build(),
        )
        .await
        .unwrap();
    assert_eq!(first_handle.pid(), second_handle.pid());

    let first_connected = first
        .commands()
        .connect(first_handle.pid(), e2b_sdk::CommandRequestOpts::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    let second_connected = second
        .commands()
        .connect(second_handle.pid(), e2b_sdk::CommandRequestOpts::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();

    assert_eq!(first_connected.stdout, "first retained\n");
    assert_eq!(second_connected.stdout, "second retained\n");
}

async fn assert_two_active_sandbox_filesystem_routes(
    first: &e2b_sdk::Sandbox,
    second: &e2b_sdk::Sandbox,
) {
    assert_eq!(
        first
            .files()
            .write(
                "/tmp/first.txt",
                b"first".to_vec(),
                e2b_sdk::FilesystemWriteOpts::default()
            )
            .await
            .unwrap()
            .path,
        "/tmp/first.txt"
    );
    assert_eq!(
        second
            .files()
            .write(
                "/tmp/second.txt",
                b"second".to_vec(),
                e2b_sdk::FilesystemWriteOpts::default()
            )
            .await
            .unwrap()
            .path,
        "/tmp/second.txt"
    );
    assert_eq!(
        first
            .files()
            .read_bytes("/tmp/first.txt", e2b_sdk::FilesystemReadOpts::default())
            .await
            .unwrap(),
        b"first"
    );
    assert_eq!(
        second
            .files()
            .read_bytes("/tmp/second.txt", e2b_sdk::FilesystemReadOpts::default())
            .await
            .unwrap(),
        b"second"
    );
    assert_eq!(
        first
            .files()
            .get_info("/tmp/first.txt", e2b_sdk::FilesystemRequestOpts::default())
            .await
            .unwrap()
            .size,
        5
    );
    assert_eq!(
        second
            .files()
            .get_info("/tmp/second.txt", e2b_sdk::FilesystemRequestOpts::default())
            .await
            .unwrap()
            .size,
        6
    );
    assert_eq!(
        first
            .files()
            .list("/tmp", e2b_sdk::FilesystemListOpts::default())
            .await
            .unwrap()[0]
            .path,
        "/tmp/first.txt"
    );
    assert_eq!(
        second
            .files()
            .list("/tmp", e2b_sdk::FilesystemListOpts::default())
            .await
            .unwrap()[0]
            .path,
        "/tmp/second.txt"
    );
    first
        .files()
        .remove("/tmp/first.txt", e2b_sdk::FilesystemRequestOpts::default())
        .await
        .unwrap();
    second
        .files()
        .remove("/tmp/second.txt", e2b_sdk::FilesystemRequestOpts::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn runtime_adapter_domain_proxy_routes_envd_to_each_active_sandbox() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let interactive_outputs = Arc::new(Mutex::new(VecDeque::from([
        b"first retained\n".to_vec(),
        b"second retained\n".to_vec(),
    ])));
    let (first, second, proxy_task, control_task) = start_two_sdk_sandboxes_through_domain_proxy(
        Arc::clone(&command_log),
        Arc::clone(&interactive_outputs),
    )
    .await;
    assert_two_active_sandbox_process_routes(&first, &second).await;
    assert_two_active_sandbox_retained_processes_do_not_collide(&first, &second).await;
    assert_two_active_sandbox_filesystem_routes(&first, &second).await;
    assert_eq!(command_log.lock().expect("lock command log").len(), 14);

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
async fn runtime_adapter_stop_preserves_other_sandbox_process_records() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let interactive_outputs = Arc::new(Mutex::new(VecDeque::from([
        b"first retained\n".to_vec(),
        b"second retained\n".to_vec(),
    ])));
    let (first, second, proxy_task, control_task) = start_two_sdk_sandboxes_through_domain_proxy(
        Arc::clone(&command_log),
        Arc::clone(&interactive_outputs),
    )
    .await;
    let first_handle = first
        .commands()
        .run_background(
            "cat",
            e2b_sdk::CommandRunOpts::builder().stdin(true).build(),
        )
        .await
        .unwrap();
    let second_handle = second
        .commands()
        .run_background(
            "cat",
            e2b_sdk::CommandRunOpts::builder().stdin(true).build(),
        )
        .await
        .unwrap();
    assert_eq!(first_handle.pid(), second_handle.pid());

    assert!(first.kill().await.unwrap());

    let second_connected = second
        .commands()
        .connect(second_handle.pid(), e2b_sdk::CommandRequestOpts::default())
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(second_connected.stdout, "second retained\n");

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
async fn runtime_adapter_routes_code_interpreter_port_to_runtime_probe() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let sandbox = adapter.start(request).await.expect("start succeeds");

    let target = adapter
        .port_target(&sandbox.config.sandbox_id, DEFAULT_CODE_INTERPRETER_PORT)
        .await
        .expect("port target exists");
    assert!(matches!(
        target,
        PortTarget::Tcp {
            ref host,
            port: _
        } if host == "127.0.0.1"
    ));

    let mut stream = adapter
        .connect_port_target(&sandbox.config.sandbox_id, target)
        .await
        .expect("code-interpreter probe connects");
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 49999-sbx_firkin_1.cube.localhost\r\n\r\n")
        .await
        .expect("write probe request");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.expect("read probe");

    let response = String::from_utf8(bytes).expect("utf8 response");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains(r#""service":"code-interpreter""#),
        "{response}"
    );
    assert!(
        response.contains(r#""sandboxID":"sbx_firkin_1""#),
        "{response}"
    );
}

#[tokio::test]
async fn runtime_adapter_executes_code_interpreter_execute_request() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let command_outputs = Arc::new(Mutex::new(VecDeque::from([exited_stdout(
        b"code stdout\n".to_vec(),
    )])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&command_outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let sandbox = adapter.start(request).await.expect("start succeeds");
    let target = adapter
        .port_target(&sandbox.config.sandbox_id, DEFAULT_CODE_INTERPRETER_PORT)
        .await
        .expect("port target exists");
    let mut stream = adapter
        .connect_port_target(&sandbox.config.sandbox_id, target)
        .await
        .expect("code-interpreter connects");
    let body = serde_json::json!({
        "code": "echo code stdout",
        "language": "bash",
        "env_vars": {
            "ROLE": "worker"
        }
    })
    .to_string();
    stream
        .write_all(
            format!(
                "POST /execute HTTP/1.1\r\nHost: 49999-sbx_firkin_1.cube.localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("write execute request");
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("read execute response");

    let response = String::from_utf8(bytes).expect("utf8 response");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains(r#""type":"stdout""#), "{response}");
    assert!(response.contains(r#""text":"code stdout\n""#), "{response}");
    assert!(
        response.contains(r#""type":"number_of_executions""#),
        "{response}"
    );
    assert_eq!(
        *command_log.lock().expect("lock command log"),
        vec![vec![
            "/bin/sh".to_owned(),
            "-lc".to_owned(),
            "echo code stdout".to_owned()
        ]]
    );
    let processes = adapter.list_processes().await.expect("list processes");
    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].envs["ROLE"], "worker");
}

#[tokio::test]
async fn runtime_adapter_executes_code_interpreter_python_request_without_shell() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let command_outputs = Arc::new(Mutex::new(VecDeque::from([exited_stdout(
        b"python stdout\n".to_vec(),
    )])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&command_outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let sandbox = adapter.start(request).await.expect("start succeeds");
    let target = adapter
        .port_target(&sandbox.config.sandbox_id, DEFAULT_CODE_INTERPRETER_PORT)
        .await
        .expect("port target exists");
    let mut stream = adapter
        .connect_port_target(&sandbox.config.sandbox_id, target)
        .await
        .expect("code-interpreter connects");
    let body = serde_json::json!({
        "code": "print('python stdout')"
    })
    .to_string();
    stream
        .write_all(
            format!(
                "POST /execute HTTP/1.1\r\nHost: 49999-sbx_firkin_1.cube.localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("write execute request");
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("read execute response");

    let response = String::from_utf8(bytes).expect("utf8 response");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains(r#""text":"python stdout\n""#),
        "{response}"
    );
    assert_eq!(
        *command_log.lock().expect("lock command log"),
        vec![vec![
            "python3".to_owned(),
            "-c".to_owned(),
            "print('python stdout')".to_owned()
        ]]
    );
}

#[tokio::test]
async fn runtime_adapter_preserves_python_context_between_execute_requests() {
    let root = tempfile::tempdir().expect("tempdir");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        HostCommandLauncher {
            root: root.path().to_path_buf(),
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let sandbox = adapter.start(request).await.expect("start succeeds");
    let first = code_interpreter_execute_http(
        &adapter,
        &sandbox.config.sandbox_id,
        serde_json::json!({
            "code": "x = 41\nprint('stored')",
            "context_id": "ctx-main"
        }),
    )
    .await;
    assert!(first.contains(r#""text":"stored\n""#), "{first}");

    let second = code_interpreter_execute_http(
        &adapter,
        &sandbox.config.sandbox_id,
        serde_json::json!({
            "code": "x += 1\nprint(x)",
            "context_id": "ctx-main"
        }),
    )
    .await;
    assert!(second.starts_with("HTTP/1.1 200 OK"), "{second}");
    assert!(second.contains(r#""text":"42\n""#), "{second}");
}

async fn code_interpreter_execute_http<L>(
    adapter: &FirkinRuntimeAdapter<L>,
    sandbox_id: &str,
    body: serde_json::Value,
) -> String
where
    L: firkin_runtime::SnapshotSessionLauncher + Clone + Send + 'static,
    L::Error: std::fmt::Display + Send,
    L::Session: RuntimeCommandRunner
        + RuntimeCommandStreamRunner
        + RuntimeInteractiveProcessRunner
        + RuntimePortRouter
        + RuntimeReadinessProbe
        + RuntimeSessionStop
        + firkin_runtime::RuntimeContinuationSnapshotSource
        + Send
        + Sync
        + 'static,
    <L::Session as RuntimeCommandRunner>::Error: std::fmt::Display + Send,
    <L::Session as RuntimeCommandStreamRunner>::Error: std::fmt::Display + Send,
    <L::Session as RuntimeInteractiveProcessRunner>::Error: std::fmt::Display + Send,
    <L::Session as RuntimePortRouter>::Error: std::fmt::Display,
    <L::Session as RuntimeReadinessProbe>::Error: std::fmt::Display + Send,
    <L::Session as RuntimeSessionStop>::Error: std::fmt::Display,
{
    let target = adapter
        .port_target(sandbox_id, DEFAULT_CODE_INTERPRETER_PORT)
        .await
        .expect("port target exists");
    let mut stream = adapter
        .connect_port_target(sandbox_id, target)
        .await
        .expect("code-interpreter connects");
    let body = body.to_string();
    stream
        .write_all(
            format!(
                "POST /execute HTTP/1.1\r\nHost: {DEFAULT_CODE_INTERPRETER_PORT}-{sandbox_id}.cube.localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("write execute request");
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .await
        .expect("read execute response");
    String::from_utf8(bytes).expect("utf8 response")
}

async fn assert_domain_proxy_connect_tunnels_port(
    proxy_addr: std::net::SocketAddr,
    host: &str,
    expected: &[u8],
) {
    let mut stream = TcpStream::connect(proxy_addr).await.unwrap();
    stream
        .write_all(format!("CONNECT / HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = Vec::new();
    let mut byte = [0_u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        response.push(byte[0]);
    }
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(bytes, expected);
}

#[tokio::test]
async fn runtime_adapter_domain_proxy_tunnels_code_interpreter_and_mcp_ports() {
    let command_log = Arc::new(Mutex::new(Vec::new()));
    let command_outputs = Arc::new(Mutex::new(VecDeque::from([exited_stdout(
        b"proxy stdout\n".to_vec(),
    )])));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            command_log: Arc::clone(&command_log),
            command_outputs: Arc::clone(&command_outputs),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let backend = LocalRuntimeBackend::from_state(adapter, local_state_with_ready_template());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_task = tokio::spawn(control_plane.serve(control_listener));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let proxy_url = format!("http://{proxy_addr}");
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox = e2b_sdk::Sandbox::create_with_config(
        sdk_config_for_sandbox(&control_url, &proxy_url, "sbx_firkin_1"),
        e2b_sdk::SandboxCreateOpts::builder()
            .template("repo-main")
            .build(),
    )
    .await
    .unwrap();
    assert_eq!(sandbox.sandbox_id(), "sbx_firkin_1");

    let response = reqwest::Client::new()
        .get(format!("http://{proxy_addr}/health"))
        .header("host", "49999-sbx_firkin_1.cube.localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["service"], "code-interpreter");
    assert_eq!(body["sandboxID"], "sbx_firkin_1");

    let execute = reqwest::Client::new()
        .post(format!("http://{proxy_addr}/execute"))
        .header("host", "49999-sbx_firkin_1.cube.localhost")
        .json(&serde_json::json!({
            "code": "printf proxy stdout",
            "language": "bash"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(execute.status(), 200);
    let execute_body = execute.text().await.unwrap();
    assert!(
        execute_body.contains(r#""text":"proxy stdout\n""#),
        "{execute_body}"
    );
    assert_eq!(
        *command_log.lock().expect("lock command log"),
        vec![vec![
            "/bin/sh".to_owned(),
            "-lc".to_owned(),
            "printf proxy stdout".to_owned()
        ]]
    );

    assert_domain_proxy_connect_tunnels_port(
        proxy_addr,
        "50005-sbx_firkin_1.cube.localhost",
        b"tpl_1:50005",
    )
    .await;

    proxy_task.abort();
    control_task.abort();
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn runtime_adapter_followup_restores_continuation_snapshot() {
    let ready_log = Arc::new(Mutex::new(Vec::new()));
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher {
            ready_log: Arc::clone(&ready_log),
            ..RecordingLauncher::default()
        },
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: None,
    };
    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Idle,
        TEST_FOLLOWUP_ARTIFACT,
    );

    let sandbox = adapter
        .start_followup(request, &plan)
        .await
        .expect("follow-up create succeeds");

    assert_eq!(sandbox.config.sandbox_id, "sbx_firkin_1");
    assert_eq!(
        *ready_log.lock().expect("lock ready log"),
        vec!["sbx_firkin_1"]
    );
    assert_eq!(
        adapter.restored_paths().await,
        vec![PathBuf::from(TEST_FOLLOWUP_ARTIFACT)]
    );
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64))
    );
    let command = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "echo followup".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("follow-up command routes through active sandbox");
    assert_eq!(command.stdout, b"command output\n");
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn runtime_adapter_queues_followup_restore_until_capacity_releases() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(1, Size::gib(1), Size::gib(10))),
        RecordingLauncher::default(),
        ResourceBudget::new(1, Size::gib(1), Size::gib(1)),
        "cube.localhost",
        "firkin-envd",
        1,
        1024,
    );
    let first = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-main".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
                has_envd: true,
                artifact_integrity: Some(test_snapshot_integrity()),
            }),
        })
        .await
        .expect("first start succeeds");
    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Idle,
        TEST_FOLLOWUP_ARTIFACT,
    );

    let queued_adapter = adapter.clone();
    let queued = tokio::spawn(async move {
        queued_adapter
            .start_followup(
                StartSandboxRequest {
                    create_request: SandboxCreateRequest::default(),
                    prepared_template: None,
                },
                &plan,
            )
            .await
    });

    tokio::time::sleep(Duration::from_millis(25)).await;
    assert!(
        !queued.is_finished(),
        "follow-up restore should wait for active capacity instead of rejecting immediately"
    );

    adapter
        .stop(&first.config.sandbox_id)
        .await
        .expect("stop releases active capacity");
    let followup = tokio::time::timeout(Duration::from_secs(1), queued)
        .await
        .expect("queued follow-up wakes")
        .expect("queued task joins")
        .expect("queued follow-up succeeds after capacity release");

    assert_eq!(followup.config.sandbox_id, "sbx_firkin_2");
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(1, Size::gib(1), Size::gib(1))
    );
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn runtime_adapter_snapshot_captures_active_continuation_artifact() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let request = StartSandboxRequest {
        create_request: SandboxCreateRequest::default(),
        prepared_template: Some(PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: TEST_SNAPSHOT_ARTIFACT.to_owned(),
            has_envd: true,
            artifact_integrity: Some(test_snapshot_integrity()),
        }),
    };
    let sandbox = adapter.start(request).await.expect("start succeeds");
    let expected_path = runtime_continuation_test_path("session-1");
    let _ = std::fs::remove_file(&expected_path);
    let expected_manifest_sidecar =
        SnapshotArtifactManifest::sidecar_path_for_artifact(&expected_path);
    let expected_integrity_sidecar =
        SnapshotArtifactIntegrity::sidecar_path_for_artifact(&expected_path);
    let _ = std::fs::remove_file(&expected_manifest_sidecar);
    let _ = std::fs::remove_file(&expected_integrity_sidecar);

    let snapshot = adapter
        .snapshot(&sandbox.config.sandbox_id, Some("session-1".to_owned()))
        .await
        .expect("capture continuation snapshot");

    assert_eq!(snapshot.snapshot_id, "session-1");
    let location = snapshot.location.expect("snapshot location");
    assert_eq!(
        std::fs::read(&location).expect("snapshot bytes"),
        b"repo-main"
    );
    let integrity = snapshot.artifact_integrity.expect("snapshot integrity");
    assert_eq!(integrity.size_bytes, b"repo-main".len() as u64);
    assert!(expected_manifest_sidecar.exists());
    assert!(expected_integrity_sidecar.exists());
    assert!(
        adapter
            .benchmark_samples()
            .await
            .iter()
            .any(|sample| sample.metric() == "snapshot_save")
    );
    let _ = std::fs::remove_file(location);
    let _ = std::fs::remove_file(expected_manifest_sidecar);
    let _ = std::fs::remove_file(expected_integrity_sidecar);
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn local_runtime_backend_snapshot_route_captures_firkin_continuation() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let expected_path = runtime_continuation_test_path("session-route");
    let _ = std::fs::remove_file(&expected_path);
    let expected_manifest_sidecar =
        SnapshotArtifactManifest::sidecar_path_for_artifact(&expected_path);
    let expected_integrity_sidecar =
        SnapshotArtifactIntegrity::sidecar_path_for_artifact(&expected_path);
    let _ = std::fs::remove_file(&expected_manifest_sidecar);
    let _ = std::fs::remove_file(&expected_integrity_sidecar);
    let mut backend = LocalRuntimeBackend::from_state(adapter, local_state_with_ready_template());
    let connected = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, "/sandboxes")
                .with_json(&SandboxCreateRequest {
                    template_id: "repo-main".to_owned(),
                    ..SandboxCreateRequest::default()
                })
                .expect("create json"),
        )
        .await
        .expect("create sandbox")
        .decode_json::<ConnectedSandbox>()
        .expect("connected sandbox");
    let snapshot = backend
        .handle_control_plane(
            ControlPlaneRequest::new(
                ControlPlaneMethod::Post,
                format!("/sandboxes/{}/snapshots", connected.sandbox_id),
            )
            .with_json(&CreateSnapshotRequest {
                name: Some("session-route".to_owned()),
            })
            .expect("snapshot json"),
        )
        .await
        .expect("snapshot route")
        .decode_json::<SnapshotInfo>()
        .expect("snapshot info");

    assert_eq!(snapshot.snapshot_id, "session-route");
    assert_eq!(
        std::fs::read(&expected_path).expect("snapshot bytes"),
        b"tpl_1"
    );
    assert!(expected_manifest_sidecar.exists());
    assert!(expected_integrity_sidecar.exists());

    backend
        .handle_control_plane(ControlPlaneRequest::new(
            ControlPlaneMethod::Delete,
            format!("/templates/{}", snapshot.snapshot_id),
        ))
        .await
        .expect("delete snapshot route succeeds");
    assert!(!expected_path.exists());
    assert!(!expected_manifest_sidecar.exists());
    assert!(!expected_integrity_sidecar.exists());
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn local_runtime_backend_followup_route_reaches_firkin_continuation_restore() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let mut sandboxes = LocalSandboxRegistry::new();
    sandboxes
        .create(
            SandboxCreateRequest::default(),
            SandboxRuntimeConfig {
                sandbox_id: "sbx_seed".to_owned(),
                domain: "cube.localhost".to_owned(),
                envd_version: "firkin-envd".to_owned(),
                envd_access_token: None,
                traffic_access_token: None,
                started_at: "2026-05-04T00:00:00Z".to_owned(),
                end_at: "2026-05-04T00:05:00Z".to_owned(),
                cpu_count: 2,
                memory_mb: 8192,
            },
        )
        .expect("seed sandbox");
    sandboxes
        .create_snapshot(
            "sbx_seed",
            CreateSnapshotRequest {
                name: Some("session-1".to_owned()),
            },
            SnapshotRef {
                snapshot_id: "session-1".to_owned(),
                location: Some(TEST_FOLLOWUP_ARTIFACT.to_owned()),
                artifact_integrity: Some(test_followup_integrity()),
            },
        )
        .expect("seed continuation snapshot");
    sandboxes.delete("sbx_seed");
    let mut backend = LocalRuntimeBackend::from_state(
        adapter.clone(),
        LocalRuntimeState {
            sandboxes,
            pods: LocalPodRegistry::new(),
            templates: LocalTemplateRegistry::new("2026-05-04T00:00:00Z"),
            volumes: LocalVolumeRegistry::new(),
        },
    );

    let connected = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, "/sandboxes/followups")
                .with_json(&FollowupSandboxCreateRequest {
                    snapshot_id: "session-1".to_owned(),
                    create_request: SandboxCreateRequest::default(),
                })
                .expect("follow-up json"),
        )
        .await
        .expect("follow-up route succeeds")
        .decode_json::<ConnectedSandbox>()
        .expect("connected sandbox");

    assert_eq!(connected.sandbox_id, "sbx_firkin_1");
    assert_eq!(
        backend
            .sandboxes()
            .get("sbx_firkin_1")
            .expect("registered follow-up")
            .template_id,
        "session-1"
    );
    assert_eq!(
        adapter.restored_paths().await,
        vec![PathBuf::from(TEST_FOLLOWUP_ARTIFACT)]
    );
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn local_runtime_backend_followup_route_reads_integrity_sidecar() {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("session-1.vz");
    std::fs::write(&snapshot_path, b"firkin-followup-sidecar").expect("snapshot");
    let manifest = SnapshotArtifactManifest::continuation("session-1", &snapshot_path);
    SnapshotArtifactIntegrity::from_file(&manifest)
        .expect("snapshot integrity")
        .write_json(SnapshotArtifactIntegrity::sidecar_path_for_artifact(
            &snapshot_path,
        ))
        .expect("write integrity sidecar");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let mut sandboxes = LocalSandboxRegistry::new();
    sandboxes
        .create(
            SandboxCreateRequest::default(),
            SandboxRuntimeConfig {
                sandbox_id: "sbx_seed".to_owned(),
                domain: "cube.localhost".to_owned(),
                envd_version: "firkin-envd".to_owned(),
                envd_access_token: None,
                traffic_access_token: None,
                started_at: "2026-05-04T00:00:00Z".to_owned(),
                end_at: "2026-05-04T00:05:00Z".to_owned(),
                cpu_count: 2,
                memory_mb: 8192,
            },
        )
        .expect("seed sandbox");
    sandboxes
        .create_snapshot(
            "sbx_seed",
            CreateSnapshotRequest {
                name: Some("session-1".to_owned()),
            },
            SnapshotRef {
                snapshot_id: "session-1".to_owned(),
                location: Some(snapshot_path.display().to_string()),
                artifact_integrity: None,
            },
        )
        .expect("seed continuation snapshot");
    sandboxes.delete("sbx_seed");
    let mut backend = LocalRuntimeBackend::from_state(
        adapter.clone(),
        LocalRuntimeState {
            sandboxes,
            pods: LocalPodRegistry::new(),
            templates: LocalTemplateRegistry::new("2026-05-04T00:00:00Z"),
            volumes: LocalVolumeRegistry::new(),
        },
    );

    let connected = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, "/sandboxes/followups")
                .with_json(&FollowupSandboxCreateRequest {
                    snapshot_id: "session-1".to_owned(),
                    create_request: SandboxCreateRequest::default(),
                })
                .expect("follow-up json"),
        )
        .await
        .expect("follow-up route succeeds")
        .decode_json::<ConnectedSandbox>()
        .expect("connected sandbox");

    assert_eq!(connected.sandbox_id, "sbx_firkin_1");
    assert_eq!(adapter.restored_paths().await, vec![snapshot_path]);
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn local_runtime_backend_followup_route_rejects_integrity_mismatch_before_restore() {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("session-1.vz");
    std::fs::write(&snapshot_path, b"snapshot-before").expect("snapshot");
    let manifest = SnapshotArtifactManifest::base("session-1", &snapshot_path);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("snapshot integrity");
    std::fs::write(&snapshot_path, b"snapshot-after").expect("mutate snapshot");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let mut sandboxes = LocalSandboxRegistry::new();
    sandboxes
        .create(
            SandboxCreateRequest::default(),
            SandboxRuntimeConfig {
                sandbox_id: "sbx_seed".to_owned(),
                domain: "cube.localhost".to_owned(),
                envd_version: "firkin-envd".to_owned(),
                envd_access_token: None,
                traffic_access_token: None,
                started_at: "2026-05-04T00:00:00Z".to_owned(),
                end_at: "2026-05-04T00:05:00Z".to_owned(),
                cpu_count: 2,
                memory_mb: 8192,
            },
        )
        .expect("seed sandbox");
    sandboxes
        .create_snapshot(
            "sbx_seed",
            CreateSnapshotRequest {
                name: Some("session-1".to_owned()),
            },
            SnapshotRef {
                snapshot_id: "session-1".to_owned(),
                location: Some(snapshot_path.display().to_string()),
                artifact_integrity: Some(PreparedTemplateArtifactIntegrity {
                    size_bytes: integrity.size_bytes(),
                    sha256_hex: integrity.sha256_hex().to_owned(),
                }),
            },
        )
        .expect("seed continuation snapshot");
    sandboxes.delete("sbx_seed");
    let mut backend = LocalRuntimeBackend::from_state(
        adapter.clone(),
        LocalRuntimeState {
            sandboxes,
            pods: LocalPodRegistry::new(),
            templates: LocalTemplateRegistry::new("2026-05-04T00:00:00Z"),
            volumes: LocalVolumeRegistry::new(),
        },
    );

    let error = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, "/sandboxes/followups")
                .with_json(&FollowupSandboxCreateRequest {
                    snapshot_id: "session-1".to_owned(),
                    create_request: SandboxCreateRequest::default(),
                })
                .expect("follow-up json"),
        )
        .await
        .expect_err("integrity mismatch rejects follow-up route");

    assert!(matches!(
        error,
        firkin_e2b_server::ControlPlaneError::Backend(BackendError::Runtime(message))
            if message.starts_with("Firkin snapshot integrity check failed:")
    ));
    assert!(adapter.restored_paths().await.is_empty());
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn runtime_adapter_delete_snapshot_removes_continuation_artifact_and_sidecars() {
    let snapshot_id = format!("delete-sidecars-{}", std::process::id());
    let snapshot_path = runtime_continuation_test_path(&snapshot_id);
    if let Some(parent) = snapshot_path.parent() {
        std::fs::create_dir_all(parent).expect("snapshot parent");
    }
    std::fs::write(&snapshot_path, b"continuation").expect("snapshot");
    let manifest = SnapshotArtifactManifest::continuation(&snapshot_id, &snapshot_path);
    manifest
        .write_json(SnapshotArtifactManifest::sidecar_path_for_artifact(
            &snapshot_path,
        ))
        .expect("manifest sidecar");
    SnapshotArtifactIntegrity::from_file(&manifest)
        .expect("integrity")
        .write_json(SnapshotArtifactIntegrity::sidecar_path_for_artifact(
            &snapshot_path,
        ))
        .expect("integrity sidecar");
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );

    adapter
        .delete_snapshot(&snapshot_id)
        .await
        .expect("delete snapshot succeeds");

    assert!(!snapshot_path.exists());
    assert!(!SnapshotArtifactManifest::sidecar_path_for_artifact(&snapshot_path).exists());
    assert!(!SnapshotArtifactIntegrity::sidecar_path_for_artifact(&snapshot_path).exists());
}

#[cfg(feature = "snapshot")]
#[tokio::test]
async fn runtime_product_soak_runner_records_inspect_like_steps() {
    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        RecordingLauncher::default(),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let backend = LocalRuntimeBackend::from_state(adapter, local_state_with_ready_template());
    let config = RuntimeProductSoakConfig::inspect_like(
        Duration::ZERO,
        SandboxCreateRequest {
            template_id: "repo-main".to_owned(),
            ..SandboxCreateRequest::default()
        },
    )
    .with_snapshot_prefix(format!("soak-test-{}", std::process::id()))
    .with_iteration_pause(Duration::ZERO);
    let mut runner = RuntimeProductSoakRunner::new(backend, config);

    let report = runner.run().await;

    for step in SoakStep::required_inspect_loop() {
        let evidence = report.step(step).expect("step evidence");
        assert_eq!(evidence.attempts(), 1, "{step:?}");
        assert_eq!(evidence.failures(), 0, "{step:?}");
    }
    assert!(runner.backend().sandboxes().list().is_empty());
    assert!(runner.backend().sandboxes().list_snapshots(None).is_empty());
    assert_eq!(report.duration(), Duration::ZERO);
}
