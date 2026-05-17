//! Ignored live VZ snapshot restore tests for the runtime crate.

#![cfg(any())]
// Scaffolding: this signed Apple/VZ integration suite mixes runtime, benchmark,
// single-node, and E2B SDK compatibility coverage. Keep it out of the default
// package graph until it moves under a dedicated live harness.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Child, Command as StdCommand, Stdio as ProcessStdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use firkin_benchmark::{
    AttributedHostMemorySnapshot, AutoscaleProtectionCounts, AutoscaleResourceBudget,
    CleanupScanEntry, DensityP95Point, ExclusiveVzTaskSetVmmapCollector,
    GuestDiskCoreBenchmarkOutput, GuestIoPressure, HOST_BYTES_RECLAIMED_AFTER_TRIM_METRIC,
    HostFootprintSnapshot, HostGuestDiskUsageOutput, HostMemoryAttributionCollector,
    HostMemoryAttributionScope, MAX_AGENT_COMPUTERS_BEFORE_READY_P95_DOUBLES_METRIC,
    MAX_PRESTARTED_AGENT_SLOTS_BEFORE_CHECKOUT_READY_P95_DOUBLES_METRIC,
    MAX_RETAINED_SHELLS_BEFORE_FIRST_STDOUT_P95_DOUBLES_METRIC, ReadyQueueOutcomes,
    RuntimeAgentComputerScorecardEvidenceWriter, RuntimeAutoscaleScorecardEvidenceWriter,
    RuntimeBenchmarkEvidenceWriter, RuntimeOverheadEvidenceWriter, RuntimeProductSoakConfig,
    RuntimeProductSoakRunner, SPARSE_BLOAT_AFTER_DELETE_METRIC, SafeSpareResourceSnapshot,
    SignedLiveReliabilityAttemptCounts, cleanup_leftover_bytes_sample,
    exact_host_memory_footprint_from_attributed_snapshots, guest_disk_core_benchmark_script,
    host_guest_disk_usage_json, max_active_before_p95_doubles,
    prestarted_agent_slot_fifo_acceptance_p95_sample, signed_live_guest_io_pressure_script,
    vz_virtual_machine_pid_set,
};
use firkin_envd::{EnvdProcessInput, EnvdProcessStartRequest, EnvdPtySize};
use firkin_oci::{Client, Reference};
use firkin_runtime::core::{Container, Rootfs, Stdio, Streams};
use firkin_runtime::template::TemplateSnapshotSink;
use firkin_runtime::types::{Platform, Size, hostname};
use firkin_runtime::{
    CommandHostProcessTerminator, CoreContainerSnapshotSink, CoreSnapshotSessionLauncher,
    CoreTemplateCommandRunner, DiskPressureError, DiskPressureProbe, FirkinRuntimeAdapter,
    FirkinWarmTemplateMaintainer, HostDiskPressureProbe, RuntimeCommandRunner,
    RuntimeCommandStartReport, RuntimeCommandStreamRunner, RuntimeCommandStreamStartReport,
    RuntimeContinuationSnapshotCapture, RuntimeContinuationSnapshotRestore,
    RuntimeDiskPressureGuard, RuntimeFilesystemReconciler, RuntimeHostProcessStuckVmCleaner,
    RuntimeHostScanner, RuntimeHygieneMaintenance, RuntimeInteractiveProcessRunner,
    RuntimeInteractiveProcessStartReport, RuntimePortRouter, RuntimeReadinessProbe,
    RuntimeReadinessReport, RuntimeSessionStop, RuntimeSnapshotRestore, RuntimeSnapshotWarmPool,
    RuntimeStuckVmCleanup, SnapshotRestoreRequest, SnapshotSessionLauncher,
    TemplateBuildRuntimeRequest, TemplateCommandRunner, default_runtime_continuation_root,
    firkin_cache_root,
};
use firkin_trace::{
    BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit, EventTraceRecorder, LifecycleClass,
    RuntimeProfile, SandboxEventName, SandboxEventTrace, WorkloadClass,
};
use firkin_vmm::Network;
use futures_util::{Stream, StreamExt, future::join_all};
use prost::Message;
use reqwest::header::CONTENT_TYPE;
use tokio::net::TcpListener;
use {
    firkin_admission::{CapacityLedger, ResourceBudget, WarmPoolKey},
    firkin_artifacts::{
        ContinuationSnapshotPlan, ContinuationSnapshotReason, SnapshotArtifactIntegrity,
        SnapshotArtifactKind, SnapshotArtifactManifest,
    },
    firkin_evidence::{
        AGENT_COMPUTER_SCORECARD_METRICS, AUTOSCALE_EFFICIENCY_SCORECARD_METRICS, BenchmarkSummary,
        PercentileAvailability, ProductAutoscaleDurationMetric, REQUIRED_FIRKIN_OVERHEAD_METRICS,
        REQUIRED_LIFECYCLE_LATENCY_METRICS, SoakEvidenceArtifact,
    },
    firkin_hygiene::{ReconciliationDecision, StuckVmCleanupDecision},
    firkin_template::TemplateBuildJob,
};
use {
    firkin_e2b_contract::{
        DEFAULT_CODE_INTERPRETER_PORT, PortTarget, PreparedTemplate,
        PreparedTemplateArtifactIntegrity, RuntimeAdapter, RuntimeSandbox, SandboxRuntimeConfig,
        SnapshotRef, StartSandboxRequest,
    },
    firkin_e2b_server::{
        ControlPlaneHttpServer, DomainProxyHttpServer, EnvdProcessHttpServer, LocalPodRegistry,
        LocalRuntimeBackend, LocalRuntimeState, LocalSandboxRegistry, LocalTemplateRegistry,
        LocalVolumeRegistry, SandboxRoutes,
    },
    firkin_e2b_wire::{
        ConnectedSandbox, ControlPlaneMethod, ControlPlaneRequest, CreateSnapshotRequest,
        FollowupSandboxCreateRequest, PodContainerCreateRequest, PodCreateRequest, PodEmptyDir,
        PodStoreImageFormat, PodStoreOptions, PodTrimPolicy, PodVolumeMountRequest,
        SandboxCreateRequest, SnapshotInfo, TemplateBuildRequest, TemplateBuildStatus,
    },
    firkin_envd::{DEFAULT_ENVD_PORT, EnvdFilesystemAdapter, EnvdProcessAdapter},
};

type RestoreTimingSamples = Arc<Mutex<Vec<firkin_runtime::core::ContainerRestoreTimings>>>;

const PYTHON_PRODUCT_POD_STORE_BYTES: u64 = 1024 * 1024 * 1024;

fn prepared_snapshot_integrity(snapshot_path: &Path) -> PreparedTemplateArtifactIntegrity {
    let manifest = SnapshotArtifactManifest::base("repo-main", snapshot_path);
    let integrity = SnapshotArtifactIntegrity::from_file(&manifest).expect("snapshot integrity");
    PreparedTemplateArtifactIntegrity {
        size_bytes: integrity.size_bytes(),
        sha256_hex: integrity.sha256_hex().to_owned(),
    }
}

fn live_runtime_continuation_path(snapshot_id: &str) -> PathBuf {
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(snapshot_id.as_bytes());
    std::env::var_os("FIRKIN_RUNTIME_CONTINUATION_ROOT")
        .map_or_else(default_runtime_continuation_root, PathBuf::from)
        .join(format!("{encoded}.vz"))
}

#[derive(Clone)]
struct ReadyLiveLauncher {
    inner: CoreSnapshotSessionLauncher,
}

impl ReadyLiveLauncher {
    fn new(inner: CoreSnapshotSessionLauncher) -> Self {
        Self { inner }
    }
}

struct ReadyLiveSession {
    inner: firkin_runtime::core::Container<firkin_runtime::core::Streams>,
}

#[async_trait]
impl SnapshotSessionLauncher for ReadyLiveLauncher {
    type Error = firkin_runtime::CoreSnapshotSessionLaunchError;
    type Session = ReadyLiveSession;

    async fn restore_from_snapshot(
        &mut self,
        request: &SnapshotRestoreRequest<'_>,
    ) -> Result<Self::Session, Self::Error> {
        self.inner
            .restore_from_snapshot(request)
            .await
            .map(|inner| ReadyLiveSession { inner })
    }
}

#[async_trait]
impl RuntimeReadinessProbe for ReadyLiveSession {
    type Error = firkin_runtime::core::Error;

    async fn probe_ready(
        &mut self,
        mut event_trace: EventTraceRecorder,
    ) -> Result<RuntimeReadinessReport, Self::Error> {
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
            return Err(firkin_runtime::core::Error::RuntimeOperation {
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

#[async_trait]
impl firkin_runtime::RuntimeContinuationSnapshotSource for ReadyLiveSession {
    async fn save_continuation_snapshot(
        &self,
        path: &Path,
    ) -> Result<(), firkin_runtime::template::SnapshotSinkError> {
        CoreContainerSnapshotSink::new(&self.inner)
            .save_snapshot(path)
            .await
    }
}

#[async_trait]
impl RuntimeSessionStop for ReadyLiveSession {
    type Error = firkin_runtime::core::Error;

    async fn stop_session(&mut self) -> Result<(), Self::Error> {
        RuntimeSessionStop::stop_session(&mut self.inner).await
    }
}

#[async_trait]
impl RuntimePortRouter for ReadyLiveSession {
    type Error = firkin_runtime::core::Error;

    async fn connect_port(
        &self,
        port: u16,
    ) -> Result<firkin_e2b_contract::PortProxyStream, Self::Error> {
        RuntimePortRouter::connect_port(&self.inner, port).await
    }
}

#[async_trait]
impl RuntimeCommandRunner for ReadyLiveSession {
    type Error = firkin_runtime::core::Error;

    async fn run_command(
        &mut self,
        request: &EnvdProcessStartRequest,
        event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStartReport, Self::Error> {
        RuntimeCommandRunner::run_command(&mut self.inner, request, event_trace).await
    }
}

#[async_trait]
impl RuntimeCommandStreamRunner for ReadyLiveSession {
    type Error = firkin_runtime::core::Error;

    async fn run_command_stream(
        &mut self,
        request: &EnvdProcessStartRequest,
        event_trace: EventTraceRecorder,
    ) -> Result<RuntimeCommandStreamStartReport, Self::Error> {
        RuntimeCommandStreamRunner::run_command_stream(&mut self.inner, request, event_trace).await
    }
}

fn hot_tiny_exec_trace() -> EventTraceRecorder {
    EventTraceRecorder::new(
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    )
}

fn hot_batch_100_exec_trace() -> EventTraceRecorder {
    EventTraceRecorder::new(
        LifecycleClass::Hot,
        WorkloadClass::Batch100Execs,
        RuntimeProfile::FastAgent,
    )
}

async fn derived_adapter_contract_metric_samples(
    adapter: &firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher>,
) -> Vec<BenchmarkSample> {
    let mut samples = adapter.benchmark_samples().await;
    samples.extend(firkin_evidence::derive_available_contract_metric_samples(
        adapter.benchmark_event_traces().await,
    ));
    samples
}

#[async_trait]
impl RuntimeInteractiveProcessRunner for ReadyLiveSession {
    type Error = firkin_runtime::core::Error;

    async fn start_interactive_process(
        &mut self,
        request: &EnvdProcessStartRequest,
    ) -> Result<RuntimeInteractiveProcessStartReport, Self::Error> {
        RuntimeInteractiveProcessRunner::start_interactive_process(&mut self.inner, request).await
    }
}

#[derive(Clone, PartialEq, Message)]
struct EnvdStartRequestProto {
    #[prost(message, optional, tag = "1")]
    process: Option<EnvdProcessConfigProto>,
    #[prost(message, optional, tag = "2")]
    pty: Option<EnvdPtyProto>,
    #[prost(string, optional, tag = "3")]
    tag: Option<String>,
    #[prost(bool, optional, tag = "4")]
    stdin: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
struct EnvdProcessConfigProto {
    #[prost(string, tag = "1")]
    cmd: String,
    #[prost(string, repeated, tag = "2")]
    args: Vec<String>,
    #[prost(map = "string, string", tag = "3")]
    envs: HashMap<String, String>,
    #[prost(string, optional, tag = "4")]
    cwd: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct EnvdPtyProto {
    #[prost(message, optional, tag = "1")]
    size: Option<EnvdPtySizeProto>,
}

#[derive(Clone, PartialEq, Message)]
struct EnvdPtySizeProto {
    #[prost(uint32, tag = "1")]
    cols: u32,
    #[prost(uint32, tag = "2")]
    rows: u32,
}

#[derive(Clone, PartialEq, Message)]
struct EnvdStartResponseProto {
    #[prost(message, optional, tag = "1")]
    event: Option<EnvdProcessEventProto>,
}

#[derive(Clone, PartialEq, Message)]
struct EnvdProcessEventProto {
    #[prost(oneof = "envd_process_event_proto::Event", tags = "1, 2, 3, 4")]
    event: Option<envd_process_event_proto::Event>,
}

mod envd_process_event_proto {
    use super::{EnvdDataEventProto, EnvdEndEventProto, EnvdKeepAliveProto, EnvdStartEventProto};
    use prost::Oneof;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Event {
        #[prost(message, tag = "1")]
        Start(EnvdStartEventProto),
        #[prost(message, tag = "2")]
        Data(EnvdDataEventProto),
        #[prost(message, tag = "3")]
        End(EnvdEndEventProto),
        #[prost(message, tag = "4")]
        Keepalive(EnvdKeepAliveProto),
    }
}

#[derive(Clone, PartialEq, Message)]
struct EnvdStartEventProto {
    #[prost(uint32, tag = "1")]
    pid: u32,
}

#[derive(Clone, PartialEq, Message)]
struct EnvdDataEventProto {
    #[prost(oneof = "envd_data_event_proto::Output", tags = "1, 2, 3")]
    output: Option<envd_data_event_proto::Output>,
}

mod envd_data_event_proto {
    use prost::Oneof;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Output {
        #[prost(bytes, tag = "1")]
        Stdout(Vec<u8>),
        #[prost(bytes, tag = "2")]
        Stderr(Vec<u8>),
        #[prost(bytes, tag = "3")]
        Pty(Vec<u8>),
    }
}

#[derive(Clone, PartialEq, Message)]
struct EnvdSendInputRequestProto {
    #[prost(message, optional, tag = "1")]
    process: Option<EnvdProcessSelectorProto>,
    #[prost(message, optional, tag = "2")]
    input: Option<EnvdProcessInputProto>,
}

#[derive(Clone, PartialEq, Message)]
struct EnvdProcessSelectorProto {
    #[prost(oneof = "envd_process_selector_proto_test::Selector", tags = "1, 2")]
    selector: Option<envd_process_selector_proto_test::Selector>,
}

mod envd_process_selector_proto_test {
    use prost::Oneof;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Selector {
        #[prost(uint32, tag = "1")]
        Pid(u32),
        #[prost(string, tag = "2")]
        Tag(String),
    }
}

#[derive(Clone, PartialEq, Message)]
struct EnvdProcessInputProto {
    #[prost(oneof = "envd_process_input_proto_test::Input", tags = "1, 2")]
    input: Option<envd_process_input_proto_test::Input>,
}

mod envd_process_input_proto_test {
    use prost::Oneof;

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Input {
        #[prost(bytes, tag = "1")]
        Stdin(Vec<u8>),
        #[prost(bytes, tag = "2")]
        Pty(Vec<u8>),
    }
}

#[derive(Clone, PartialEq, Message)]
struct EnvdSendInputResponseProto {}

#[derive(Clone, PartialEq, Message)]
struct EnvdEndEventProto {
    #[prost(sint32, tag = "1")]
    exit_code: i32,
    #[prost(bool, tag = "2")]
    exited: bool,
    #[prost(string, tag = "3")]
    status: String,
    #[prost(string, optional, tag = "4")]
    error: Option<String>,
}

#[derive(Clone, PartialEq, Message)]
struct EnvdKeepAliveProto {}

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

async fn save_live_snapshot(rootfs: Rootfs, source_id: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let staging_path = temp.path().join("source-staging");

    let source = live_builder(source_id, rootfs)
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("source container");
    CoreContainerSnapshotSink::new(&source)
        .save_snapshot(&snapshot_path)
        .await
        .expect("save snapshot");
    let _ = source.stop().await;

    (temp, snapshot_path)
}

fn live_envd_adapter(
    rootfs: Rootfs,
    builder_id: &str,
) -> firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher> {
    live_envd_adapter_with_timing_samples(rootfs, builder_id, None)
}

fn live_envd_adapter_with_timing_samples(
    rootfs: Rootfs,
    builder_id: &str,
    timing_samples: Option<RestoreTimingSamples>,
) -> firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher> {
    live_envd_adapter_with_timing_samples_and_capacity(
        rootfs,
        builder_id,
        timing_samples,
        ResourceBudget::new(8, Size::gib(64), Size::gib(512)),
    )
}

fn live_envd_adapter_with_timing_samples_and_capacity(
    rootfs: Rootfs,
    builder_id: &str,
    timing_samples: Option<RestoreTimingSamples>,
    total_capacity: ResourceBudget,
) -> firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher> {
    let mut launcher = CoreSnapshotSessionLauncher::new(live_builder(builder_id, rootfs));
    if let Some(timing_samples) = timing_samples {
        launcher = launcher.with_timing_samples(timing_samples);
    }
    firkin_runtime::FirkinRuntimeAdapter::new(
        CapacityLedger::new(total_capacity),
        ReadyLiveLauncher::new(launcher),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    )
    .with_restore_minimum_free_disk(Size::gib(3))
}

fn live_agent_computer_adapter(
    rootfs: Rootfs,
    builder_id: &str,
) -> firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher> {
    live_envd_adapter(rootfs, builder_id).with_restore_minimum_free_disk(Size::gib(3))
}

async fn start_live_envd_sandbox(
    adapter: &firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher>,
    snapshot_path: &Path,
) -> RuntimeSandbox {
    adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-main".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: snapshot_path.to_string_lossy().into_owned(),
                has_envd: true,
                artifact_integrity: Some(prepared_snapshot_integrity(snapshot_path)),
            }),
        })
        .await
        .expect("start live runtime adapter")
}

fn live_backend_with_template(
    adapter: firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher>,
    snapshot_path: &Path,
) -> LocalRuntimeBackend<firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher>> {
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
                artifact: snapshot_path.to_string_lossy().into_owned(),
                has_envd: true,
                artifact_integrity: Some(prepared_snapshot_integrity(snapshot_path)),
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

    LocalRuntimeBackend::from_state(
        adapter,
        LocalRuntimeState {
            sandboxes: LocalSandboxRegistry::new(),
            pods: LocalPodRegistry::new(),
            templates,
            volumes: LocalVolumeRegistry::new(),
        },
    )
}

fn encode_envd_start_request(cmd: &str, args: &[&str], tag: &str) -> Vec<u8> {
    let mut request = Vec::new();
    EnvdStartRequestProto {
        process: Some(EnvdProcessConfigProto {
            cmd: cmd.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            envs: HashMap::new(),
            cwd: None,
        }),
        pty: None,
        tag: Some(tag.to_owned()),
        stdin: Some(false),
    }
    .encode(&mut request)
    .expect("encode start request");
    request
}

fn encode_envd_send_input_request(pid: u32, data: Vec<u8>) -> Vec<u8> {
    let mut request = Vec::new();
    EnvdSendInputRequestProto {
        process: Some(EnvdProcessSelectorProto {
            selector: Some(envd_process_selector_proto_test::Selector::Pid(pid)),
        }),
        input: Some(EnvdProcessInputProto {
            input: Some(envd_process_input_proto_test::Input::Stdin(data)),
        }),
    }
    .encode(&mut request)
    .expect("encode send input request");
    request
}

fn encode_live_envd_start_request() -> Vec<u8> {
    encode_envd_start_request("/bin/sh", &["-lc", "printf 'envd live\\n'"], "live-envd")
}

fn assert_envd_start_response_stdout(body: &[u8], expected_stdout: &[u8]) {
    let (start_flags, start_body, rest) = decode_grpc_web_frame(body);
    assert_eq!(start_flags, 0);
    let start = EnvdStartResponseProto::decode(start_body.as_slice()).unwrap();
    assert!(matches!(
        start.event.and_then(|event| event.event),
        Some(envd_process_event_proto::Event::Start(EnvdStartEventProto { pid })) if pid > 0
    ));

    let (stdout_flags, stdout_body, rest) = decode_grpc_web_frame(rest);
    assert_eq!(stdout_flags, 0);
    let stdout = EnvdStartResponseProto::decode(stdout_body.as_slice()).unwrap();
    assert!(matches!(
        stdout.event.and_then(|event| event.event),
        Some(envd_process_event_proto::Event::Data(EnvdDataEventProto {
            output: Some(envd_data_event_proto::Output::Stdout(bytes)),
        })) if bytes == expected_stdout
    ));

    let (end_flags, end_body, trailers) = decode_grpc_web_frame(rest);
    assert_eq!(end_flags, 0);
    let end = EnvdStartResponseProto::decode(end_body.as_slice()).unwrap();
    assert!(matches!(
        end.event.and_then(|event| event.event),
        Some(envd_process_event_proto::Event::End(EnvdEndEventProto {
            exit_code: 0,
            exited: true,
            ..
        }))
    ));

    let (trailer_flags, trailer, rest) = decode_grpc_web_frame(trailers);
    assert_eq!(trailer_flags, 0x80);
    assert!(
        String::from_utf8(trailer)
            .unwrap()
            .contains("grpc-status: 0")
    );
    assert!(rest.is_empty());
}

fn assert_live_envd_start_response(body: &[u8]) {
    assert_envd_start_response_stdout(body, b"envd live\n");
}

fn live_arm64_busybox_rootfs_path() -> Option<PathBuf> {
    std::env::var_os("FIRKIN_ARM64_BUSYBOX_ROOTFS").map(PathBuf::from)
}

async fn live_arm64_busybox_rootfs() -> Rootfs {
    if let Some(path) = live_arm64_busybox_rootfs_path() {
        return Rootfs::ext4_image(path);
    }
    live_arm64_oci_rootfs("busybox", live_busybox_cache_dir()).await
}

async fn live_arm64_bash_rootfs() -> Rootfs {
    if let Some(path) = std::env::var_os("FIRKIN_ARM64_BASH_ROOTFS") {
        return Rootfs::ext4_image(path);
    }
    live_arm64_oci_rootfs("debian:bookworm-slim", live_bash_cache_dir()).await
}

async fn live_arm64_python_rootfs() -> Rootfs {
    if let Some(path) = std::env::var_os("FIRKIN_ARM64_PYTHON_ROOTFS") {
        return Rootfs::ext4_image(path);
    }
    live_arm64_oci_rootfs("python:3.12-slim", live_python_cache_dir()).await
}

async fn live_arm64_git_rootfs() -> Rootfs {
    if let Some(path) = std::env::var_os("FIRKIN_ARM64_GIT_ROOTFS") {
        return Rootfs::ext4_image(path);
    }
    live_arm64_oci_rootfs("alpine/git:latest", live_git_cache_dir()).await
}

async fn live_arm64_oci_rootfs(reference: &str, cache_dir: PathBuf) -> Rootfs {
    let image = Client::builder()
        .cache_dir(cache_dir)
        .platform(Platform::linux_arm64())
        .build()
        .expect("oci client")
        .pull(&Reference::parse(reference).expect("oci reference"))
        .await
        .expect("oci pull");
    Rootfs::oci_bundle(image)
}

fn live_busybox_cache_dir() -> PathBuf {
    std::env::var_os("FIRKIN_LIVE_BUSYBOX_CACHE").map_or_else(
        || firkin_cache_root().join("live").join("busybox"),
        PathBuf::from,
    )
}

fn live_bash_cache_dir() -> PathBuf {
    std::env::var_os("FIRKIN_LIVE_BASH_CACHE").map_or_else(
        || firkin_cache_root().join("live").join("bash"),
        PathBuf::from,
    )
}

fn live_python_cache_dir() -> PathBuf {
    std::env::var_os("FIRKIN_LIVE_PYTHON_CACHE").map_or_else(
        || firkin_cache_root().join("live").join("python"),
        PathBuf::from,
    )
}

fn live_git_cache_dir() -> PathBuf {
    std::env::var_os("FIRKIN_LIVE_GIT_CACHE").map_or_else(
        || firkin_cache_root().join("live").join("git"),
        PathBuf::from,
    )
}

async fn live_host_gateway_addr(container: &Container<Streams>) -> String {
    if let Ok(host_gateway) = std::env::var("FIRKIN_HOST_GATEWAY") {
        return host_gateway;
    }
    container
        .network_interfaces()
        .await
        .first()
        .expect("vmnet network interface")
        .gateway()
        .to_string()
}

fn lifecycle_latency_sample(metric: &'static str, elapsed: Duration) -> BenchmarkSample {
    BenchmarkSample::new(
        metric,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        elapsed.as_secs_f64() * 1000.0,
    )
}

fn shell_density_hot_to_first_stdout_sample(
    concurrency: usize,
    concurrency_levels: &[usize],
    elapsed: Duration,
) -> BenchmarkSample {
    BenchmarkSample::new(
        format!("start.hot_to_first_stdout_density_c{concurrency}_ms"),
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        elapsed.as_secs_f64() * 1000.0,
    )
    .with_static_tag("measurement_boundary", "sdk_shell_density")
    .with_static_tag("ready_signal", "first_stdout")
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("concurrency_levels", density_level_tag(concurrency_levels))
}

fn shell_density_phase_sample(
    phase: &'static str,
    concurrency: usize,
    concurrency_levels: &[usize],
    elapsed: Duration,
) -> BenchmarkSample {
    BenchmarkSample::new(
        format!("debug.density.sdk_{phase}_c{concurrency}_ms"),
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        elapsed.as_secs_f64() * 1000.0,
    )
    .with_static_tag("measurement_boundary", "sdk_shell_density")
    .with_static_tag("diagnostic", "true")
    .with_static_tag("phase", phase)
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("concurrency_levels", density_level_tag(concurrency_levels))
}

fn firkin_overhead_sample(
    metric: &'static str,
    unit: BenchmarkUnit,
    value: f64,
) -> BenchmarkSample {
    BenchmarkSample::new(metric, BenchmarkMetricKind::FirkinOverhead, unit, value)
}

fn create_host_git_repo(root: &Path) -> PathBuf {
    let source = root.join("source");
    let bare = root.join("repo.git");
    std::fs::create_dir_all(&source).expect("source repo dir");
    std::fs::write(source.join("README.md"), "template repo\n").expect("readme");
    run_host_git(root, ["init", source.to_str().expect("source path")]);
    run_host_git(
        root,
        [
            "-C",
            source.to_str().expect("source path"),
            "checkout",
            "-B",
            "master",
        ],
    );
    run_host_git(
        root,
        [
            "-C",
            source.to_str().expect("source path"),
            "config",
            "user.email",
            "firkin@example.invalid",
        ],
    );
    run_host_git(
        root,
        [
            "-C",
            source.to_str().expect("source path"),
            "config",
            "user.name",
            "Firkin Test",
        ],
    );
    run_host_git(
        root,
        ["-C", source.to_str().expect("source path"), "add", "."],
    );
    run_host_git(
        root,
        [
            "-C",
            source.to_str().expect("source path"),
            "commit",
            "-m",
            "initial",
        ],
    );
    run_host_git(
        root,
        [
            "clone",
            "--bare",
            source.to_str().expect("source path"),
            bare.to_str().expect("bare path"),
        ],
    );
    bare
}

fn run_host_git<const N: usize>(root: &Path, args: [&str; N]) {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run host git");
    assert!(
        output.status.success(),
        "git failed: status={:?} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn ls_remote_head(repo_url: &str, branch: &str) -> String {
    let output = StdCommand::new("git")
        .args(["ls-remote", repo_url, branch])
        .output()
        .expect("git ls-remote");
    assert!(
        output.status.success(),
        "git ls-remote failed: status={:?} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("ls-remote utf8");
    stdout
        .split_whitespace()
        .next()
        .expect("ls-remote sha")
        .to_owned()
}

fn current_process_idle_cpu_percent(window: Duration) -> f64 {
    std::thread::sleep(window);
    let output = StdCommand::new("ps")
        .args(["-o", "%cpu=", "-p", &std::process::id().to_string()])
        .output()
        .expect("run ps cpu");
    assert!(
        output.status.success(),
        "ps cpu failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("cpu utf8")
        .trim()
        .parse::<f64>()
        .expect("cpu percent")
}

fn current_process_rss_mib() -> f64 {
    let output = StdCommand::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("run ps rss");
    assert!(
        output.status.success(),
        "ps rss failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rss_kib = String::from_utf8(output.stdout)
        .expect("rss utf8")
        .trim()
        .parse::<f64>()
        .expect("rss kib");
    rss_kib / 1024.0
}

fn dir_size_bytes(path: &Path) -> u64 {
    let output = StdCommand::new("du")
        .args(["-sk", path.to_str().expect("path utf8")])
        .output()
        .expect("run du");
    assert!(
        output.status.success(),
        "du failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("du utf8");
    let kib = stdout
        .split_whitespace()
        .next()
        .expect("du size")
        .parse::<u64>()
        .expect("du kib");
    kib * 1024
}

fn existing_file_bytes_under(path: &Path) -> std::io::Result<u64> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut bytes = 0_u64;
    for entry in std::fs::read_dir(path)? {
        bytes = bytes
            .checked_add(existing_file_bytes_under(&entry?.path())?)
            .expect("cleanup leftover byte scan overflowed");
    }
    Ok(bytes)
}

fn run_scoped_cleanup_leftover_sample(
    roots: impl IntoIterator<Item = (&'static str, PathBuf)>,
) -> BenchmarkSample {
    cleanup_leftover_bytes_sample(roots.into_iter().map(|(name, path)| {
        CleanupScanEntry::new(
            name,
            existing_file_bytes_under(&path).expect("scan run-scoped cleanup root"),
        )
    }))
    .expect("cleanup leftover byte scan does not overflow")
}

struct HostGitDaemon {
    child: Child,
    port: u16,
}

impl HostGitDaemon {
    fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for HostGitDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_host_git_daemon(repo_root: &Path) -> HostGitDaemon {
    start_host_git_daemon_with_candidate_ports(
        repo_root,
        (0..8).map(|_| reserve_ephemeral_host_port()),
    )
}

fn reserve_ephemeral_host_port() -> u16 {
    let port_socket = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("free port");
    let port = port_socket.local_addr().expect("local addr").port();
    drop(port_socket);
    port
}

fn start_host_git_daemon_with_candidate_ports(
    repo_root: &Path,
    candidate_ports: impl IntoIterator<Item = u16>,
) -> HostGitDaemon {
    let base_path = format!("--base-path={}", repo_root.to_str().expect("repo root"));
    let mut last_status = None;
    for port in candidate_ports {
        let port_arg = format!("--port={port}");
        let mut child = StdCommand::new("git")
            .args([
                "daemon",
                "--reuseaddr",
                "--export-all",
                base_path.as_str(),
                "--listen=0.0.0.0",
                port_arg.as_str(),
            ])
            .stdout(ProcessStdio::null())
            .stderr(ProcessStdio::null())
            .spawn()
            .expect("start git daemon");
        std::thread::sleep(Duration::from_millis(500));
        if let Some(status) = child.try_wait().expect("poll git daemon") {
            last_status = Some(status);
            continue;
        }
        return HostGitDaemon { child, port };
    }
    panic!("git daemon failed to start on candidate ports: {last_status:?}");
}

#[test]
fn host_git_daemon_retries_when_candidate_port_is_busy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let _bare = create_host_git_repo(temp.path());
    let busy_socket = std::net::TcpListener::bind(("0.0.0.0", 0)).expect("busy port");
    let busy_port = busy_socket.local_addr().expect("busy addr").port();
    let free_socket = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("free port");
    let retry_port = free_socket.local_addr().expect("free addr").port();
    drop(free_socket);

    let daemon = start_host_git_daemon_with_candidate_ports(temp.path(), [busy_port, retry_port]);

    assert_eq!(daemon.port(), retry_port);
}

fn assert_child_exited_after_term(child: &mut Child) {
    for _ in 0..20 {
        if let Some(status) = child.try_wait().expect("poll child exit") {
            assert!(
                !status.success(),
                "terminated child unexpectedly exited cleanly"
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    panic!("marked host process was not terminated");
}

fn live_builder(
    id: &str,
    rootfs: Rootfs,
) -> firkin_runtime::core::ContainerBuilder<
    firkin_runtime::core::ImplicitVm,
    firkin_runtime::core::Ready,
> {
    Container::builder(id)
        .expect("builder")
        .memory(Size::mib(512))
        .rosetta(false)
        .command(["/bin/sleep", "30"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .rootfs(rootfs)
}

fn live_networked_builder(
    id: &str,
    rootfs: Rootfs,
) -> firkin_runtime::core::ContainerBuilder<
    firkin_runtime::core::ImplicitVm,
    firkin_runtime::core::Ready,
> {
    live_builder(id, rootfs).networks([Network::vmnet_shared()])
}

#[tokio::test]
#[ignore = "live VZ snapshot restore smoke; requires signed test harness"]
async fn live_core_snapshot_launcher_restores_and_runs_command() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let staging_path = temp.path().join("source-staging");

    let source = live_builder("live-snapshot-source", rootfs.clone())
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("source container");
    CoreContainerSnapshotSink::new(&source)
        .save_snapshot(&snapshot_path)
        .await
        .expect("save snapshot");
    let _ = source.stop().await;

    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let mut launcher =
        CoreSnapshotSessionLauncher::new(live_builder("live-snapshot-source", rootfs));

    let report = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_elapsed(&mut launcher, Duration::ZERO)
        .await
        .expect("restore snapshot");
    let restore_samples = report.benchmark_samples().to_vec();
    let (mut session, _reservation) = report.into_parts();
    let command = session
        .run_command(
            &EnvdProcessStartRequest {
                cmd: "/bin/echo".to_owned(),
                args: vec!["restored snapshot".to_owned()],
                ..EnvdProcessStartRequest::default()
            },
            hot_tiny_exec_trace(),
        )
        .await
        .expect("run command in restored session");
    let _ = session.stop().await;

    assert_eq!(restore_samples[0].metric(), "warm_snapshot_restore");
    assert_eq!(command.output().stdout, b"restored snapshot\n");
    assert!(
        command
            .benchmark_samples()
            .iter()
            .any(|sample| sample.metric() == "command_start")
    );
    assert!(
        command
            .benchmark_samples()
            .iter()
            .any(|sample| sample.metric() == "first_stdout_byte")
    );
}

#[tokio::test]
#[ignore = "live VZ warm-pool smoke; requires signed test harness"]
async fn live_snapshot_warm_pool_checkouts_retained_session() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let source_id = "live-warm-pool-source";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), source_id).await;

    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let mut pool = RuntimeSnapshotWarmPool::new(CapacityLedger::new(ResourceBudget::new(
        8,
        Size::gib(64),
        Size::gib(512),
    )));
    let mut launcher = CoreSnapshotSessionLauncher::new(live_builder(source_id, rootfs));

    pool.maintain_with_elapsed(
        key.clone(),
        &manifest,
        budget,
        &mut launcher,
        Duration::ZERO,
    )
    .await
    .expect("maintain live warm-pool entry");
    assert_eq!(pool.capacity().warm_pool(), budget);

    let checkout = pool
        .checkout_with_elapsed(&key, Duration::ZERO)
        .expect("checkout live warm-pool entry")
        .expect("warm entry exists");
    assert_eq!(pool.capacity().active(), budget);
    assert!(
        checkout
            .benchmark_samples()
            .iter()
            .any(|sample| sample.metric() == "warm_pool_checkout")
    );
    let (mut session, _reservation) = checkout.into_parts();
    let command = session
        .run_command(
            &EnvdProcessStartRequest {
                cmd: "/bin/echo".to_owned(),
                args: vec!["warm pool retained".to_owned()],
                ..EnvdProcessStartRequest::default()
            },
            hot_tiny_exec_trace(),
        )
        .await
        .expect("run command in checked-out warm session");
    let _ = session.stop().await;

    assert_eq!(command.output().stdout, b"warm pool retained\n");
}

#[tokio::test]
#[ignore = "live VZ template build smoke; requires signed test harness and guest networking"]
async fn live_template_build_snapshot_clones_repo_and_restores() {
    let rootfs = live_arm64_git_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let state_path = temp.path().join("repo-main.state.json");
    let staging_path = temp.path().join("template-staging");
    let _bare = create_host_git_repo(temp.path());
    let git_daemon = start_host_git_daemon(temp.path());
    let mut source = live_networked_builder("live-template-build-source", rootfs.clone())
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("template source container");
    let repo_url = format!(
        "git://{}:{}/repo.git",
        live_host_gateway_addr(&source).await,
        git_daemon.port()
    );
    let job = TemplateBuildJob::new(repo_url, "master", &snapshot_path)
        .setup_command("test -d .git")
        .cache_warm_command("git status --short");
    CoreTemplateCommandRunner::new(&mut source)
        .run_template_commands(&TemplateBuildRuntimeRequest::new(&job, "repo-main"))
        .await
        .expect("template build commands");
    drop(git_daemon);
    CoreContainerSnapshotSink::new(&source)
        .with_state_path(&state_path)
        .save_snapshot(&snapshot_path)
        .await
        .expect("save template snapshot");
    let _ = source.stop().await;

    assert!(snapshot_path.exists());
    assert!(state_path.exists());

    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let mut launcher =
        CoreSnapshotSessionLauncher::new(live_builder("live-template-build-source", rootfs))
            .with_state_path(&state_path);
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let restore = RuntimeSnapshotRestore::new(
        &mut ledger,
        &manifest,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
    )
    .execute_with_elapsed(&mut launcher, Duration::ZERO)
    .await
    .expect("restore template snapshot");
    let (mut restored, _reservation) = restore.into_parts();
    let command = restored
        .run_command(
            &EnvdProcessStartRequest {
                cmd: "/bin/sh".to_owned(),
                args: vec![
                    "-lc".to_owned(),
                    "test -d /workspace/templates/repo-main/.git && echo template-ok".to_owned(),
                ],
                ..EnvdProcessStartRequest::default()
            },
            hot_tiny_exec_trace(),
        )
        .await
        .expect("verify restored template checkout");
    let _ = restored.stop().await;

    assert_eq!(command.output().stdout, b"template-ok\n");
}

#[tokio::test]
#[ignore = "live VZ freshness sync smoke; requires signed test harness and public network"]
async fn live_runtime_freshness_sync_fast_forwards_public_repo_after_restore() {
    let rootfs = live_arm64_git_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main-freshness.vz");
    let state_path = temp.path().join("repo-main-freshness.state.json");
    let staging_path = temp.path().join("template-freshness-staging");
    let repo_url = std::env::var("FIRKIN_LIVE_FRESHNESS_REPO")
        .unwrap_or_else(|_| "https://github.com/octocat/Hello-World.git".to_owned());
    let branch =
        std::env::var("FIRKIN_LIVE_FRESHNESS_BRANCH").unwrap_or_else(|_| "master".to_owned());
    let target = ls_remote_head(&repo_url, &format!("refs/heads/{branch}"));

    let mut source = live_builder("live-freshness-source", rootfs.clone())
        .networks([Network::vmnet_shared()])
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("freshness source container");
    let job = TemplateBuildJob::new(&repo_url, &branch, &snapshot_path)
        .setup_command("git reset --hard HEAD~1")
        .cache_warm_command("git status --short");
    CoreTemplateCommandRunner::new(&mut source)
        .run_template_commands(&TemplateBuildRuntimeRequest::new(&job, "repo-main"))
        .await
        .expect("template build commands");
    CoreContainerSnapshotSink::new(&source)
        .with_state_path(&state_path)
        .save_snapshot(&snapshot_path)
        .await
        .expect("save freshness template snapshot");
    let _ = source.stop().await;

    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        ReadyLiveLauncher::new(
            CoreSnapshotSessionLauncher::new(
                live_builder("live-freshness-source", rootfs).networks([Network::vmnet_shared()]),
            )
            .with_state_path(&state_path),
        ),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let sandbox = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest {
                metadata: BTreeMap::from([
                    (
                        "firkin.sync.branch".to_owned(),
                        format!("refs/heads/{branch}"),
                    ),
                    ("firkin.sync.target".to_owned(), target.clone()),
                    (
                        "firkin.sync.checkout".to_owned(),
                        "/workspace/templates/repo-main".to_owned(),
                    ),
                ]),
                ..SandboxCreateRequest::default()
            },
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-main".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: snapshot_path.to_string_lossy().into_owned(),
                has_envd: true,
                artifact_integrity: Some(prepared_snapshot_integrity(&snapshot_path)),
            }),
        })
        .await
        .expect("start freshness sandbox");

    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let output = adapter
                .start_process(EnvdProcessStartRequest {
                    cmd: "git".to_owned(),
                    args: vec!["rev-parse".to_owned(), "HEAD".to_owned()],
                    cwd: Some("/workspace/templates/repo-main".to_owned()),
                    ..EnvdProcessStartRequest::default()
                })
                .await
                .expect("read restored checkout head");
            if output.stdout == format!("{target}\n").into_bytes() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("freshness sync fast-forwards checkout");
    adapter
        .write_file(
            "/workspace/templates/repo-main/.firkin-ready".to_owned(),
            b"ready\n".to_vec(),
        )
        .await
        .expect("writes unlock after freshness sync");
    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop freshness sandbox");
}

#[tokio::test]
#[ignore = "live VZ freshness sync product route smoke; requires signed test harness and public network"]
async fn live_freshness_sync_product_route_fast_forwards_public_repo_after_restore() {
    let rootfs = live_arm64_git_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main-freshness-route.vz");
    let state_path = temp.path().join("repo-main-freshness-route.state.json");
    let staging_path = temp.path().join("template-freshness-route-staging");
    let repo_url = std::env::var("FIRKIN_LIVE_FRESHNESS_REPO")
        .unwrap_or_else(|_| "https://github.com/octocat/Hello-World.git".to_owned());
    let branch =
        std::env::var("FIRKIN_LIVE_FRESHNESS_BRANCH").unwrap_or_else(|_| "master".to_owned());
    let target = ls_remote_head(&repo_url, &format!("refs/heads/{branch}"));

    let source_builder_id = "live-freshness-route-source";
    let mut source = live_builder(source_builder_id, rootfs.clone())
        .networks([Network::vmnet_shared()])
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("freshness route source container");
    let job = TemplateBuildJob::new(&repo_url, &branch, &snapshot_path)
        .setup_command("git reset --hard HEAD~1")
        .cache_warm_command("git status --short");
    CoreTemplateCommandRunner::new(&mut source)
        .run_template_commands(&TemplateBuildRuntimeRequest::new(&job, "repo-main"))
        .await
        .expect("template build commands");
    CoreContainerSnapshotSink::new(&source)
        .with_state_path(&state_path)
        .save_snapshot(&snapshot_path)
        .await
        .expect("save freshness product route template snapshot");
    let _ = source.stop().await;

    let adapter = FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        ReadyLiveLauncher::new(
            CoreSnapshotSessionLauncher::new(
                live_builder(source_builder_id, rootfs).networks([Network::vmnet_shared()]),
            )
            .with_state_path(&state_path),
        ),
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
        "cube.localhost",
        "firkin-envd",
        2,
        8192,
    );
    let mut backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let connected = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, "/sandboxes")
                .with_json(&SandboxCreateRequest {
                    template_id: "repo-main".to_owned(),
                    metadata: BTreeMap::from([
                        (
                            "firkin.sync.branch".to_owned(),
                            format!("refs/heads/{branch}"),
                        ),
                        ("firkin.sync.target".to_owned(), target.clone()),
                        (
                            "firkin.sync.checkout".to_owned(),
                            "/workspace/templates/repo-main".to_owned(),
                        ),
                    ]),
                    ..SandboxCreateRequest::default()
                })
                .expect("create sandbox json"),
        )
        .await
        .expect("freshness product route create")
        .decode_json::<ConnectedSandbox>()
        .expect("connected freshness sandbox");

    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let output = adapter
                .start_process(EnvdProcessStartRequest {
                    cmd: "git".to_owned(),
                    args: vec!["rev-parse".to_owned(), "HEAD".to_owned()],
                    cwd: Some("/workspace/templates/repo-main".to_owned()),
                    ..EnvdProcessStartRequest::default()
                })
                .await
                .expect("read restored checkout head");
            if output.stdout == format!("{target}\n").into_bytes() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("freshness product route sync fast-forwards checkout");
    adapter
        .write_file(
            "/workspace/templates/repo-main/.firkin-ready".to_owned(),
            b"ready\n".to_vec(),
        )
        .await
        .expect("writes unlock after product route freshness sync");
    backend
        .delete(&connected.sandbox_id)
        .await
        .expect("delete freshness product route sandbox");
}

#[tokio::test]
#[ignore = "live VZ continuation snapshot smoke; requires signed test harness"]
async fn live_continuation_snapshot_capture_restores_session_state() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("session-1-followup.vz");
    let state_path = temp.path().join("session-1-followup.state.json");
    let staging_path = temp.path().join("continuation-staging");

    let mut source = live_builder("live-continuation-source", rootfs.clone())
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("continuation source container");
    source
        .run_command(
            &EnvdProcessStartRequest {
                cmd: "/bin/sh".to_owned(),
                args: vec![
                    "-lc".to_owned(),
                    "echo continued > /tmp/firkin-continuation-marker".to_owned(),
                ],
                ..EnvdProcessStartRequest::default()
            },
            hot_tiny_exec_trace(),
        )
        .await
        .expect("write continuation marker");

    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Idle,
        &snapshot_path,
    );
    let sink = CoreContainerSnapshotSink::new(&source).with_state_path(&state_path);
    let capture = RuntimeContinuationSnapshotCapture::new(&plan)
        .execute_with_elapsed(&sink, Duration::ZERO)
        .await
        .expect("capture continuation snapshot");
    let _ = source.stop().await;

    assert_eq!(
        capture.manifest().kind(),
        SnapshotArtifactKind::Continuation
    );
    assert_eq!(capture.reason(), ContinuationSnapshotReason::Idle);
    assert_eq!(capture.benchmark_samples()[0].metric(), "snapshot_save");
    assert!(snapshot_path.exists());
    assert!(state_path.exists());

    let mut launcher =
        CoreSnapshotSessionLauncher::new(live_builder("live-continuation-source", rootfs))
            .with_state_path(&state_path);
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let restore = RuntimeContinuationSnapshotRestore::new(
        &mut ledger,
        &plan,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64)),
    )
    .execute_with_elapsed(&mut launcher, Duration::ZERO)
    .await
    .expect("restore continuation snapshot");
    assert_eq!(restore.reason(), ContinuationSnapshotReason::Idle);
    assert_eq!(
        restore.benchmark_samples()[0].metric(),
        "warm_snapshot_restore"
    );

    let (mut restored, _reason, _reservation, _samples) = restore.into_parts();
    let command = restored
        .run_command(
            &EnvdProcessStartRequest {
                cmd: "/bin/sh".to_owned(),
                args: vec![
                    "-lc".to_owned(),
                    "cat /tmp/firkin-continuation-marker".to_owned(),
                ],
                ..EnvdProcessStartRequest::default()
            },
            hot_tiny_exec_trace(),
        )
        .await
        .expect("read restored continuation marker");
    let _ = restored.stop().await;

    assert_eq!(command.output().stdout, b"continued\n");
}

#[tokio::test]
#[ignore = "live VZ create-snapshot product route smoke; requires signed test harness"]
async fn live_create_snapshot_product_route_restores_followup_state() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let builder_id = "live-create-snapshot-route-source";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let snapshot_id = "live-product-route-session";
    let continuation_path = live_runtime_continuation_path(snapshot_id);
    let continuation_state_path = continuation_path.with_extension("state.json");
    let _ = std::fs::remove_file(&continuation_path);
    let _ = std::fs::remove_file(&continuation_state_path);

    let adapter = live_envd_adapter(rootfs, builder_id);
    let mut backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let connected = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, "/sandboxes")
                .with_json(&SandboxCreateRequest {
                    template_id: "repo-main".to_owned(),
                    ..SandboxCreateRequest::default()
                })
                .expect("create sandbox json"),
        )
        .await
        .expect("product route create")
        .decode_json::<ConnectedSandbox>()
        .expect("connected sandbox");

    adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec![
                "-lc".to_owned(),
                "echo product-captured > /tmp/firkin-product-snapshot-marker".to_owned(),
            ],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("write product route snapshot marker");

    let snapshot = backend
        .handle_control_plane(
            ControlPlaneRequest::new(
                ControlPlaneMethod::Post,
                format!("/sandboxes/{}/snapshots", connected.sandbox_id),
            )
            .with_json(&CreateSnapshotRequest {
                name: Some(snapshot_id.to_owned()),
            })
            .expect("snapshot json"),
        )
        .await
        .expect("product route create snapshot")
        .decode_json::<SnapshotInfo>()
        .expect("snapshot info");
    assert_eq!(snapshot.snapshot_id, snapshot_id);
    assert!(continuation_path.exists());
    assert!(continuation_state_path.exists());

    backend
        .delete(&connected.sandbox_id)
        .await
        .expect("delete source sandbox");
    let followup = backend
        .handle_control_plane(
            ControlPlaneRequest::new(ControlPlaneMethod::Post, "/sandboxes/followups")
                .with_json(&FollowupSandboxCreateRequest {
                    snapshot_id: snapshot_id.to_owned(),
                    create_request: SandboxCreateRequest::default(),
                })
                .expect("follow-up json"),
        )
        .await
        .expect("follow-up route")
        .decode_json::<ConnectedSandbox>()
        .expect("connected follow-up");

    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec![
                "-lc".to_owned(),
                "cat /tmp/firkin-product-snapshot-marker".to_owned(),
            ],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("read product route snapshot marker");

    assert_eq!(output.stdout, b"product-captured\n");
    backend
        .delete(&followup.sandbox_id)
        .await
        .expect("delete follow-up sandbox");
    let _ = std::fs::remove_file(continuation_path);
    let _ = std::fs::remove_file(continuation_state_path);
}

#[tokio::test]
#[ignore = "live VZ snapshot integrity smoke; requires signed test harness"]
async fn live_runtime_adapter_rejects_mutated_snapshot_integrity_before_restore() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let builder_id = "live-integrity-source";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let integrity = prepared_snapshot_integrity(&snapshot_path);
    std::fs::write(&snapshot_path, b"mutated-live-snapshot").expect("mutate snapshot artifact");
    let adapter = live_envd_adapter(rootfs, builder_id);

    let error = adapter
        .start(StartSandboxRequest {
            create_request: SandboxCreateRequest::default(),
            prepared_template: Some(PreparedTemplate {
                template_id: "repo-main".to_owned(),
                build_id: "build-1".to_owned(),
                artifact: snapshot_path.to_string_lossy().into_owned(),
                has_envd: true,
                artifact_integrity: Some(integrity),
            }),
        })
        .await
        .expect_err("mutated snapshot integrity rejects restore");
    let error = error.to_string();

    assert!(
        error.contains("snapshot artifact size mismatch")
            || error.contains("snapshot artifact sha256 mismatch"),
        "{error}"
    );
}

#[tokio::test]
#[ignore = "live VZ product-route soak; requires signed test harness"]
async fn live_product_route_soak_writes_evidence_artifact() {
    let duration = std::env::var("FIRKIN_LIVE_SOAK_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or_else(|| Duration::from_secs(1), Duration::from_secs);
    let artifact = std::env::var_os("FIRKIN_LIVE_SOAK_ARTIFACT").map_or_else(
        || PathBuf::from("target/firkin-live-evidence/live-soak-evidence.json"),
        PathBuf::from,
    );
    let benchmark_artifact = std::env::var_os("FIRKIN_LIVE_SOAK_BENCHMARK_ARTIFACT").map_or_else(
        || PathBuf::from("target/firkin-live-evidence/live-benchmark-evidence.json"),
        PathBuf::from,
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create soak evidence dir");
    }

    let rootfs = live_arm64_busybox_rootfs().await;
    let builder_id = "live-product-route-soak-source";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let config = RuntimeProductSoakConfig::inspect_like(
        duration,
        SandboxCreateRequest {
            template_id: "repo-main".to_owned(),
            ..SandboxCreateRequest::default()
        },
    )
    .with_snapshot_prefix(format!("live-product-route-soak-{}", std::process::id()))
    .with_iteration_pause(Duration::from_secs(30))
    .with_benchmark_artifact(benchmark_artifact.to_string_lossy().into_owned());
    let mut runner = RuntimeProductSoakRunner::new(backend, config);

    let report = runner.run().await;

    for step in report.steps() {
        assert!(step.attempts() > 0, "{step:?}");
        assert_eq!(step.failures(), 0, "{step:?}");
    }
    SoakEvidenceArtifact::write_json(&artifact, &report).expect("write soak evidence");
}

#[tokio::test]
#[ignore = "live VZ follow-up product route smoke; requires signed test harness"]
#[allow(clippy::too_many_lines)]
async fn live_followup_product_route_restores_continuation_snapshot() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("session-1-followup-route.vz");
    let state_path = temp.path().join("session-1-followup-route.state.json");
    let staging_path = temp.path().join("followup-route-staging");

    let builder_id = "live-followup-route-source";
    let mut source = live_builder(builder_id, rootfs.clone())
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("follow-up source container");
    source
        .run_command(
            &EnvdProcessStartRequest {
                cmd: "/bin/sh".to_owned(),
                args: vec![
                    "-lc".to_owned(),
                    "echo product-route > /tmp/firkin-continuation-marker".to_owned(),
                ],
                ..EnvdProcessStartRequest::default()
            },
            hot_tiny_exec_trace(),
        )
        .await
        .expect("write follow-up marker");

    let plan = ContinuationSnapshotPlan::new(
        "session-1",
        ContinuationSnapshotReason::Idle,
        &snapshot_path,
    );
    RuntimeContinuationSnapshotCapture::new(&plan)
        .execute_with_elapsed(
            &CoreContainerSnapshotSink::new(&source).with_state_path(&state_path),
            Duration::ZERO,
        )
        .await
        .expect("capture follow-up route continuation snapshot");
    let _ = source.stop().await;

    let adapter = firkin_runtime::FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        ReadyLiveLauncher::new(
            CoreSnapshotSessionLauncher::new(live_builder(builder_id, rootfs))
                .with_state_path(&state_path),
        ),
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
        .expect("seed source sandbox");
    sandboxes
        .create_snapshot(
            "sbx_seed",
            CreateSnapshotRequest {
                name: Some("session-1".to_owned()),
            },
            SnapshotRef {
                snapshot_id: "session-1".to_owned(),
                location: Some(snapshot_path.to_string_lossy().into_owned()),
                artifact_integrity: Some(prepared_snapshot_integrity(&snapshot_path)),
            },
        )
        .expect("seed follow-up snapshot");
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
        .expect("follow-up route")
        .decode_json::<ConnectedSandbox>()
        .expect("connected follow-up");

    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec![
                "-lc".to_owned(),
                "cat /tmp/firkin-continuation-marker".to_owned(),
            ],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("read marker through follow-up sandbox");

    assert_eq!(connected.sandbox_id, "sbx_firkin_1");
    assert_eq!(output.stdout, b"product-route\n");
    backend
        .delete(&connected.sandbox_id)
        .await
        .expect("delete follow-up sandbox");
}

#[tokio::test]
#[ignore = "live VZ interactive process smoke; requires signed test harness"]
async fn live_core_snapshot_launcher_retains_interactive_stdin_process() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let staging_path = temp.path().join("source-staging");

    let source = live_builder("live-interactive-source", rootfs.clone())
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("source container");
    CoreContainerSnapshotSink::new(&source)
        .save_snapshot(&snapshot_path)
        .await
        .expect("save snapshot");
    let _ = source.stop().await;

    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let mut launcher =
        CoreSnapshotSessionLauncher::new(live_builder("live-interactive-source", rootfs));
    let report = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_elapsed(&mut launcher, Duration::ZERO)
        .await
        .expect("restore snapshot");
    let (mut session, _reservation) = report.into_parts();

    let interactive = session
        .start_interactive_process(&EnvdProcessStartRequest {
            cmd: "/bin/cat".to_owned(),
            stdin: Some(true),
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("start interactive cat");
    let (start_output, _samples, mut process) = interactive.into_parts();
    process
        .send_input(EnvdProcessInput::Stdin(b"interactive snapshot\n".to_vec()))
        .await
        .expect("send stdin");
    process.close_stdin().await.expect("close stdin");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let connected = process.connect().await.expect("connect output");
    let _ = session.stop().await;

    assert!(!start_output.exited);
    assert_eq!(connected.stdout, b"interactive snapshot\n");
}

#[tokio::test]
#[ignore = "live VZ PTY process smoke; requires signed test harness"]
async fn live_core_snapshot_launcher_retains_interactive_pty_process() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("repo-main.vz");
    let staging_path = temp.path().join("source-staging");

    let source = live_builder("live-pty-source", rootfs.clone())
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("source container");
    CoreContainerSnapshotSink::new(&source)
        .save_snapshot(&snapshot_path)
        .await
        .expect("save snapshot");
    let _ = source.stop().await;

    let manifest = SnapshotArtifactManifest::base("repo-main", &snapshot_path);
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let mut launcher = CoreSnapshotSessionLauncher::new(live_builder("live-pty-source", rootfs));
    let report = RuntimeSnapshotRestore::new(&mut ledger, &manifest, budget)
        .execute_with_elapsed(&mut launcher, Duration::ZERO)
        .await
        .expect("restore snapshot");
    let (mut session, _reservation) = report.into_parts();

    let interactive = session
        .start_interactive_process(&EnvdProcessStartRequest {
            cmd: "/bin/cat".to_owned(),
            pty: Some(EnvdPtySize { cols: 80, rows: 24 }),
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("start PTY cat");
    let (start_output, _samples, mut process) = interactive.into_parts();
    process
        .send_input(EnvdProcessInput::Pty(b"pty snapshot\n".to_vec()))
        .await
        .expect("send pty input");
    process
        .update_pty(Some(EnvdPtySize {
            cols: 100,
            rows: 30,
        }))
        .await
        .expect("resize pty");
    process.close_stdin().await.expect("close pty");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let connected = process.connect().await.expect("connect pty output");
    let _ = session.stop().await;

    assert!(!start_output.exited);
    assert!(
        String::from_utf8_lossy(&connected.pty).contains("pty snapshot"),
        "PTY output did not include echoed/captured input: {:?}",
        connected.pty
    );
}

#[tokio::test]
#[ignore = "live VM-backed envd HTTP smoke; requires signed test harness"]
async fn live_firkin_runtime_adapter_backs_envd_process_http_server() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let builder_id = "live-envd";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let sandbox = start_live_envd_sandbox(&adapter, &snapshot_path).await;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(EnvdProcessHttpServer::new(adapter.clone()).serve(listener));
    let client = reqwest::Client::new();

    let response = client
        .post(format!("{base_url}/process.Process/Start"))
        .header(CONTENT_TYPE, "application/grpc-web+proto")
        .body(grpc_web_frame(0, &encode_live_envd_start_request()))
        .send()
        .await
        .expect("envd start request");

    assert_eq!(response.status(), 200);
    let body = response.bytes().await.expect("envd start body");
    assert_live_envd_start_response(&body);

    task.abort();
    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop sandbox");
}

#[tokio::test]
#[ignore = "live VZ host-scan smoke; requires signed test harness"]
async fn live_firkin_runtime_adapter_publishes_active_vz_marker_for_host_scan() {
    let rootfs = live_arm64_busybox_rootfs().await;
    let builder_id = "live-host-scan";
    let (_snapshot_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let scan_temp = tempfile::tempdir().expect("scan tempdir");
    let active_vm_root = scan_temp.path().join("active-vms");
    let snapshot_root = scan_temp.path().join("snapshots");
    let log_root = scan_temp.path().join("logs");
    let process_root = scan_temp.path().join("processes");
    for root in [&active_vm_root, &snapshot_root, &log_root, &process_root] {
        std::fs::create_dir_all(root).expect("create host scan root");
    }
    let adapter = live_envd_adapter(rootfs, builder_id)
        .with_active_vm_marker_root(&active_vm_root)
        .with_active_vm_heartbeat_interval(Duration::from_millis(25));
    let sandbox = start_live_envd_sandbox(&adapter, &snapshot_path).await;
    let sandbox_id = sandbox.config.sandbox_id.clone();

    tokio::time::sleep(Duration::from_millis(75)).await;
    let scan = RuntimeHostScanner::new(&active_vm_root, &snapshot_root, &log_root, &process_root)
        .scan()
        .expect("scan active live VM marker");
    let observation = scan
        .stuck_vm_observations()
        .iter()
        .find(|observation| observation.id() == sandbox_id)
        .expect("active live VM marker is visible to host scan");
    assert_eq!(observation.runtime_pid(), Some(std::process::id()));
    assert!(
        observation.heartbeat_age() < Duration::from_secs(5),
        "fresh heartbeat should be preserved, got {:?}",
        observation.heartbeat_age()
    );
    assert_eq!(
        scan.reconciliation_plan().decision_for(&sandbox_id),
        Some(ReconciliationDecision::Recover)
    );
    assert_eq!(
        scan.stuck_vm_cleanup_plan(Duration::from_mins(1))
            .decision_for(&sandbox_id),
        Some(StuckVmCleanupDecision::Preserve)
    );

    adapter.stop(&sandbox_id).await.expect("stop sandbox");
    let stopped_scan =
        RuntimeHostScanner::new(&active_vm_root, &snapshot_root, &log_root, &process_root)
            .scan()
            .expect("scan stopped live VM marker root");
    assert!(
        stopped_scan
            .stuck_vm_observations()
            .iter()
            .all(|observation| observation.id() != sandbox_id)
    );
    assert!(
        stopped_scan
            .restart_records()
            .iter()
            .all(|record| record.id() != sandbox_id)
    );
}

#[tokio::test]
#[ignore = "live host-process stuck-VM cleanup smoke; requires signed test harness"]
async fn live_stuck_vm_cleanup_terminates_marked_host_process() {
    let scan_temp = tempfile::tempdir().expect("scan tempdir");
    let active_vm_root = scan_temp.path().join("active-vms");
    let snapshot_root = scan_temp.path().join("snapshots");
    let log_root = scan_temp.path().join("logs");
    let process_root = scan_temp.path().join("processes");
    let quarantine_root = scan_temp.path().join("quarantine");
    for root in [
        &active_vm_root,
        &snapshot_root,
        &log_root,
        &process_root,
        &quarantine_root,
    ] {
        std::fs::create_dir_all(root).expect("create host scan root");
    }
    let mut child = StdCommand::new("/bin/sleep")
        .arg("60")
        .spawn()
        .expect("spawn marked host process");
    let marker = active_vm_root.join("vm-stale");
    std::fs::create_dir_all(&marker).expect("create active VM marker");
    std::fs::write(marker.join("heartbeat"), b"1\n").expect("write stale heartbeat");
    std::fs::write(marker.join("runtime.pid"), format!("{}\n", child.id())).expect("write pid");
    std::fs::write(marker.join("runtime.executable"), b"/bin/sleep\n").expect("write executable");

    let scan = RuntimeHostScanner::new(&active_vm_root, &snapshot_root, &log_root, &process_root)
        .scan()
        .expect("scan stale active VM marker");
    let plan = scan.stuck_vm_cleanup_plan(Duration::from_mins(1));
    assert_eq!(
        plan.decision_for("vm-stale"),
        Some(StuckVmCleanupDecision::Cleanup)
    );
    let marker_cleaner = RuntimeFilesystemReconciler::new(
        &active_vm_root,
        &snapshot_root,
        &log_root,
        &process_root,
        &quarantine_root,
    );
    let mut cleaner =
        RuntimeHostProcessStuckVmCleaner::new(marker_cleaner, CommandHostProcessTerminator);

    let report = RuntimeStuckVmCleanup::new(&plan)
        .execute(&mut cleaner)
        .expect("execute host-process backed stuck VM cleanup");

    assert_eq!(report.cleaned_count(), 1);
    assert!(!active_vm_root.join("vm-stale").exists());
    assert_child_exited_after_term(&mut child);
}

#[tokio::test]
#[ignore = "live VZ hygiene pressure smoke; requires signed test harness"]
async fn live_runtime_hygiene_maintenance_reclaims_unreferenced_vz_snapshot_directory_and_rotates_log()
 {
    let rootfs = live_arm64_busybox_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_root = temp.path().join("snapshots");
    let log_root = temp.path().join("logs");
    let staging_path = temp.path().join("source-staging");
    std::fs::create_dir_all(&snapshot_root).expect("snapshot root");
    std::fs::create_dir_all(&log_root).expect("log root");
    let snapshot_path = snapshot_root.join("repo-main.vz");
    let stale_snapshot_path = snapshot_root.join("stale.vz");
    let active_log = log_root.join("runtime.log");

    let source = live_builder("live-hygiene-source", rootfs)
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("source container");
    CoreContainerSnapshotSink::new(&source)
        .save_snapshot(&snapshot_path)
        .await
        .expect("save snapshot");
    let _ = source.stop().await;
    assert!(snapshot_path.exists(), "live VZ snapshot artifact exists");
    SnapshotArtifactManifest::base("repo-main", &snapshot_path)
        .write_json(SnapshotArtifactManifest::sidecar_path_for_artifact(
            &snapshot_path,
        ))
        .expect("write manifest sidecar");
    std::fs::create_dir(&stale_snapshot_path).expect("stale snapshot dir");
    std::fs::write(stale_snapshot_path.join("state"), b"stale").expect("stale snapshot child");
    std::fs::write(&active_log, b"0123456789").expect("large log");

    let report =
        RuntimeHygieneMaintenance::new(&snapshot_root, [], &log_root, 4, Duration::from_mins(1))
            .with_manifest_dir(&snapshot_root)
            .tick()
            .expect("hygiene maintenance tick");

    assert!(
        report
            .artifact_gc()
            .deleted()
            .contains(&stale_snapshot_path)
    );
    assert_eq!(report.log_rotation().rotated_count(), 1);
    assert!(snapshot_path.exists());
    assert!(!stale_snapshot_path.exists());
    assert!(!active_log.exists());
    assert!(log_root.join("runtime.log.1").exists());
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy smoke; requires signed test harness"]
async fn live_vendored_sdk_runs_command_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-domain";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter.clone(), &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let config = e2b_sdk::Config::builder()
        .api_key("local-test")
        .api_url(format!("{control_url}/"))
        .sandbox_url(format!("{proxy_url}/"))
        .sandbox_header(
            "host",
            format!("{DEFAULT_ENVD_PORT}-sbx_firkin_1.cube.localhost"),
        )
        .request_timeout(std::time::Duration::from_mins(1))
        .build()
        .unwrap();
    let sandbox = e2b_sdk::Sandbox::create_with_config(
        config,
        e2b_sdk::SandboxCreateOpts::builder()
            .template("repo-main")
            .build(),
    )
    .await
    .unwrap();

    let result = sandbox
        .commands()
        .run("printf sdk-live", e2b_sdk::CommandRunOpts::default())
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "sdk-live");
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(2, Size::gib(8), Size::gib(64))
    );
    assert!(sandbox.kill().await.unwrap());
    assert_eq!(
        adapter.active_budget().await,
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
    assert!(
        adapter
            .benchmark_samples()
            .await
            .iter()
            .any(|sample| sample.metric() == "kill_delete")
    );

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy code-interpreter probe smoke; requires signed test harness"]
async fn live_vendored_sdk_reaches_code_interpreter_probe_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-code-interpreter-probe";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter.clone(), &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let proxy_url = format!("http://{proxy_addr}");
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    assert_eq!(sandbox.sandbox_id(), "sbx_firkin_1");

    let response = reqwest::Client::new()
        .get(format!("http://{proxy_addr}/health"))
        .header(
            "host",
            format!("{DEFAULT_CODE_INTERPRETER_PORT}-sbx_firkin_1.cube.localhost"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "code-interpreter");
    assert_eq!(body["sandboxID"], "sbx_firkin_1");
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed code-interpreter execute smoke; requires signed test harness"]
async fn live_code_interpreter_execute_runs_bash_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-code-interpreter-execute";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let proxy_url = format!("http://{proxy_addr}");
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    assert_eq!(sandbox.sandbox_id(), "sbx_firkin_1");

    let response = reqwest::Client::new()
        .post(format!("http://{proxy_addr}/execute"))
        .header(
            "host",
            format!("{DEFAULT_CODE_INTERPRETER_PORT}-sbx_firkin_1.cube.localhost"),
        )
        .json(&serde_json::json!({
            "code": "printf firkin-code-interpreter-live",
            "language": "bash"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(
        body.contains(r#""text":"firkin-code-interpreter-live""#),
        "{body}"
    );
    assert!(body.contains(r#""type":"number_of_executions""#), "{body}");
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed concurrent code-interpreter execute smoke; requires signed test harness"]
async fn live_code_interpreter_execute_routes_two_active_sandboxes() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-code-interpreter-concurrent";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let proxy_url = format!("http://{proxy_addr}");
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let first =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let second =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_2")).await;
    let client = reqwest::Client::new();
    let (first_response, second_response) = tokio::join!(
        async {
            client
                .post(format!("http://{proxy_addr}/execute"))
                .header(
                    "host",
                    format!("{DEFAULT_CODE_INTERPRETER_PORT}-sbx_firkin_1.cube.localhost"),
                )
                .json(&serde_json::json!({
                    "code": "printf first-code-interpreter",
                    "language": "bash"
                }))
                .send()
                .await
                .unwrap()
        },
        async {
            client
                .post(format!("http://{proxy_addr}/execute"))
                .header(
                    "host",
                    format!("{DEFAULT_CODE_INTERPRETER_PORT}-sbx_firkin_2.cube.localhost"),
                )
                .json(&serde_json::json!({
                    "code": "printf second-code-interpreter",
                    "language": "bash"
                }))
                .send()
                .await
                .unwrap()
        }
    );
    assert_eq!(first_response.status(), 200);
    assert_eq!(second_response.status(), 200);
    let first_body = first_response.text().await.unwrap();
    let second_body = second_response.text().await.unwrap();
    assert!(
        first_body.contains(r#""text":"first-code-interpreter""#),
        "{first_body}"
    );
    assert!(
        second_body.contains(r#""text":"second-code-interpreter""#),
        "{second_body}"
    );
    assert!(first.kill().await.unwrap());
    assert!(second.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed code-interpreter python context smoke; requires signed test harness"]
async fn live_code_interpreter_python_context_survives_execute_requests() {
    let rootfs = live_arm64_python_rootfs().await;
    let builder_id = "live-code-interpreter-python-context";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let proxy_url = format!("http://{proxy_addr}");
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let client = reqwest::Client::new();
    let first = client
        .post(format!("http://{proxy_addr}/execute"))
        .header(
            "host",
            format!("{DEFAULT_CODE_INTERPRETER_PORT}-sbx_firkin_1.cube.localhost"),
        )
        .json(&serde_json::json!({
            "code": "x = 41\nprint('stored')",
            "context_id": "ctx-main"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let first_body = first.text().await.unwrap();
    assert!(first_body.contains(r#""text":"stored\n""#), "{first_body}");

    let second = client
        .post(format!("http://{proxy_addr}/execute"))
        .header(
            "host",
            format!("{DEFAULT_CODE_INTERPRETER_PORT}-sbx_firkin_1.cube.localhost"),
        )
        .json(&serde_json::json!({
            "code": "x += 1\nprint(x)",
            "context_id": "ctx-main"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 200);
    let second_body = second.text().await.unwrap();
    assert!(second_body.contains(r#""text":"42\n""#), "{second_body}");
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy prewarmed template smoke; requires signed test harness"]
async fn live_vendored_sdk_uses_prewarmed_template_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-warm-product";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let mut templates = LocalTemplateRegistry::new("2026-05-04T00:00:00Z");
    let requested = templates.request_build(TemplateBuildRequest {
        name: Some("repo-main".to_owned()),
        ..TemplateBuildRequest::default()
    });
    let prepared = PreparedTemplate {
        template_id: requested.template_id.clone(),
        build_id: requested.build_id.clone(),
        artifact: snapshot_path.to_string_lossy().into_owned(),
        has_envd: true,
        artifact_integrity: Some(prepared_snapshot_integrity(&snapshot_path)),
    };
    templates
        .set_prepared_template(&requested.template_id, prepared.clone())
        .expect("template exists");
    templates
        .set_build_status(
            &requested.template_id,
            &requested.build_id,
            TemplateBuildStatus::Ready,
        )
        .expect("build exists");
    adapter
        .prewarm_template(prepared)
        .await
        .expect("prewarm template");
    let backend = LocalRuntimeBackend::from_state(
        adapter.clone(),
        LocalRuntimeState {
            sandboxes: LocalSandboxRegistry::new(),
            pods: LocalPodRegistry::new(),
            templates,
            volumes: LocalVolumeRegistry::new(),
        },
    );

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let result = sandbox
        .commands()
        .run("printf warm-product", e2b_sdk::CommandRunOpts::default())
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "warm-product");
    assert!(
        adapter
            .benchmark_samples()
            .await
            .iter()
            .any(|sample| sample.metric() == "warm_pool_checkout")
    );
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy concurrent command smoke; requires signed test harness"]
async fn live_vendored_sdk_runs_concurrent_commands_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-concurrent-commands";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let first =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let second =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_2")).await;

    let (first_result, second_result) = tokio::join!(
        async {
            first
                .commands()
                .run("printf first-command", e2b_sdk::CommandRunOpts::default())
                .await
                .unwrap()
        },
        async {
            second
                .commands()
                .run("printf second-command", e2b_sdk::CommandRunOpts::default())
                .await
                .unwrap()
        }
    );
    assert_eq!(first_result.exit_code, 0);
    assert_eq!(first_result.stdout, "first-command");
    assert_eq!(second_result.exit_code, 0);
    assert_eq!(second_result.stdout, "second-command");
    assert!(first.kill().await.unwrap());
    assert!(second.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "live VZ benchmark evidence smoke; requires signed test harness"]
async fn live_runtime_benchmark_evidence_writes_required_lifecycle_artifact() {
    let mut samples = Vec::new();
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let restore_timing_samples = Arc::new(Mutex::new(Vec::new()));
    for _ in
        0..live_runtime_repeat_count(std::env::var_os("FIRKIN_LIVE_BENCHMARK_REPEATS").as_deref())
    {
        let temp = tempfile::tempdir().expect("benchmark repeat tempdir");
        samples.extend(collect_live_template_build_benchmark_samples(temp.path()).await);
        samples.extend(Box::pin(collect_live_cold_ready_benchmark_samples()).await);
        samples.extend(Box::pin(collect_live_warm_ready_benchmark_samples()).await);
        samples.extend(Box::pin(collect_live_resume_to_first_stdout_benchmark_samples()).await);
        collect_live_warm_pool_benchmark_samples(Arc::clone(&restore_timing_samples)).await;
        samples.extend(
            Box::pin(collect_live_sdk_lifecycle_benchmark_samples(Arc::clone(
                &restore_timing_samples,
            )))
            .await,
        );
        samples.extend(collect_live_reliability_benchmark_samples().await);
        samples.extend(collect_live_guest_disk_core_benchmark_samples().await);
        samples.extend(collect_live_product_pod_disk_reclaim_samples().await);
        samples.extend(collect_live_cleanup_leftover_benchmark_samples().await);
    }

    let artifact = live_runtime_benchmark_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_BENCHMARK_ARTIFACT").as_deref(),
    );
    let report = RuntimeBenchmarkEvidenceWriter::new(&artifact)
        .write_samples(samples)
        .expect("write live benchmark evidence");
    assert_eq!(
        report.required_metrics(),
        REQUIRED_LIFECYCLE_LATENCY_METRICS
    );
    assert!(artifact.exists());
    if let Some(path) = std::env::var_os("FIRKIN_LIVE_RESTORE_TIMING_ARTIFACT") {
        write_restore_timing_artifact(
            Path::new(&path),
            &restore_timing_samples
                .lock()
                .expect("lock restore timing samples"),
        );
    }
}

#[tokio::test]
#[ignore = "live VZ agent-computer product-path scorecard; requires signed test harness"]
async fn live_runtime_agent_computer_scorecard_writes_product_path_artifact() {
    let mut samples = Vec::new();
    let mut traces = Vec::new();
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    for _ in 0..live_runtime_repeat_count(
        std::env::var_os("FIRKIN_LIVE_AGENT_COMPUTER_REPEATS").as_deref(),
    ) {
        let evidence = Box::pin(collect_live_agent_computer_scorecard_evidence()).await;
        samples.extend(evidence.samples);
        traces.extend(evidence.traces);
    }

    assert_agent_computer_scorecard_samples_cover_required_metrics(&samples);
    let artifact = live_runtime_agent_computer_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_AGENT_COMPUTER_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create agent-computer artifact parent");
    }
    RuntimeAgentComputerScorecardEvidenceWriter::new(&artifact)
        .write_samples_with_traces(samples, traces)
        .expect("write live agent-computer scorecard evidence");
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VZ autoscale product-path scorecard; requires signed test harness"]
async fn live_runtime_autoscale_scorecard_writes_product_path_artifact() {
    let mut samples = Vec::new();
    let mut traces = Vec::new();
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    for _ in
        0..live_runtime_repeat_count(std::env::var_os("FIRKIN_LIVE_AUTOSCALE_REPEATS").as_deref())
    {
        let evidence = Box::pin(collect_live_agent_computer_scorecard_evidence()).await;
        let autoscale_observation = evidence.observed_autoscale_harness();
        samples.extend(evidence.samples);
        traces.extend(evidence.traces);
        let ready_queue_capacity = Box::pin(collect_live_autoscale_ready_queue_capacity(
            autoscale_observation.snappy_ready_queue_capacity_target(),
        ))
        .await;
        let autoscale_observation =
            autoscale_observation.with_ready_queue_capacity(ready_queue_capacity);
        let product_density_levels = live_runtime_density_levels(
            std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS").as_deref(),
            &[1, 2, 4, 8, 16, 24, 32],
        );
        let (product_density_samples, product_density_traces) = Box::pin(
            collect_live_product_pod_ready_deck_density_samples(&product_density_levels),
        )
        .await;
        samples.extend(product_density_samples);
        traces.extend(product_density_traces);
        let prestarted_slot_density_levels = live_runtime_density_levels(
            std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS")
                .as_deref(),
            &[1, 2, 4, 8, 16, 24, 32],
        );
        let (prestarted_slot_density_samples, prestarted_slot_density_traces) = Box::pin(
            collect_live_product_pod_prestarted_agent_slot_density_samples(
                &prestarted_slot_density_levels,
            ),
        )
        .await;
        samples.extend(prestarted_slot_density_samples);
        traces.extend(prestarted_slot_density_traces);
        let (pressure_samples, pressure_traces) =
            Box::pin(live_autoscale_pressure_scenario_samples()).await;
        samples.extend(pressure_samples);
        traces.extend(pressure_traces);
        samples.extend(live_autoscale_harness_samples(autoscale_observation));
    }

    let artifact = live_runtime_autoscale_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_AUTOSCALE_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create autoscale artifact parent");
    }
    RuntimeAutoscaleScorecardEvidenceWriter::new(&artifact)
        .write_samples_with_traces(samples, traces)
        .expect("write live autoscale scorecard evidence");
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VZ DB sidecar readiness proof; boots a product pod"]
async fn live_runtime_db_sidecar_readiness_writes_exact_sample() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let (sample, trace) = collect_live_db_sidecar_readiness_sample().await;

    assert_eq!(sample.metric(), "product.database_ready_ms");
    assert_eq!(sample.tag_value("trust"), Some("exact_host_event_pair"));
    assert_eq!(sample.tag_value("measurement_boundary"), Some("db_sidecar"));
    assert_eq!(
        sample.tag_value("database_boundary"),
        Some("real_db_sidecar")
    );
    assert_eq!(sample.tag_value("pod_surface"), Some("product_pod"));
    assert!(
        trace
            .headline_event(SandboxEventName::DatabaseReady)
            .is_some()
    );
    let artifact = live_runtime_db_sidecar_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_DB_SIDECAR_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create DB sidecar artifact parent");
    }
    write_live_db_sidecar_readiness_artifact(&artifact, sample, trace);
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VZ browser sidecar readiness proof; boots a product pod"]
async fn live_runtime_browser_sidecar_readiness_writes_exact_sample() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let (sample, trace) = collect_live_browser_sidecar_readiness_sample().await;

    assert_eq!(sample.metric(), "product.browser_ready_ms");
    assert_eq!(sample.tag_value("trust"), Some("exact_host_event_pair"));
    assert_eq!(
        sample.tag_value("measurement_boundary"),
        Some("browser_sidecar")
    );
    assert_eq!(
        sample.tag_value("browser_boundary"),
        Some("real_browser_sidecar")
    );
    assert_eq!(sample.tag_value("pod_surface"), Some("product_pod"));
    assert!(
        trace
            .headline_event(SandboxEventName::BrowserReady)
            .is_some()
    );
    let artifact = live_runtime_browser_sidecar_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_BROWSER_SIDECAR_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create browser sidecar artifact parent");
    }
    write_live_browser_sidecar_readiness_artifact(&artifact, sample, trace);
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VZ integrated product-pod readiness proof; boots a product pod"]
async fn live_runtime_product_pod_readiness_writes_real_boundary_sample() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let (sample, trace) = collect_live_product_pod_readiness_sample().await;

    assert_eq!(sample.metric(), "product.agent_computer_ready_ms");
    assert_eq!(sample.tag_value("trust"), Some("exact_host_event_pair"));
    assert_eq!(
        sample.tag_value("measurement_boundary"),
        Some("product_path")
    );
    assert_eq!(sample.tag_value("cli_boundary"), Some("real_cli"));
    assert_eq!(
        sample.tag_value("browser_boundary"),
        Some("real_browser_sidecar")
    );
    assert_eq!(
        sample.tag_value("database_boundary"),
        Some("real_db_sidecar")
    );
    assert!(
        trace
            .headline_event(SandboxEventName::AgentComputerReady)
            .is_some()
    );
    let artifact = live_runtime_product_pod_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create product-pod artifact parent");
    }
    write_live_product_pod_readiness_artifact(&artifact, sample, trace);
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VZ ready-deck product-pod proof; boots a product pod"]
async fn live_runtime_product_pod_ready_deck_writes_real_boundary_resume_sample() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let (samples, traces) = collect_live_product_pod_ready_deck_samples(live_runtime_repeat_count(
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_READY_DECK_REPEATS").as_deref(),
    ))
    .await;
    let sample = samples.first().expect("ready-deck sample");
    let trace = traces.first().expect("ready-deck trace");

    assert_eq!(sample.metric(), "product.agent_computer_resume_ms");
    assert_eq!(sample.tag_value("trust"), Some("exact_host_event_pair"));
    assert_eq!(
        sample.tag_value("measurement_boundary"),
        Some("product_path")
    );
    assert_eq!(sample.tag_value("cli_boundary"), Some("real_cli"));
    assert_eq!(
        sample.tag_value("browser_boundary"),
        Some("real_browser_sidecar")
    );
    assert_eq!(
        sample.tag_value("database_boundary"),
        Some("real_db_sidecar")
    );
    assert!(
        trace
            .headline_event(SandboxEventName::AgentComputerReady)
            .is_some()
    );
    let artifact = live_runtime_product_pod_ready_deck_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_READY_DECK_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create ready-deck product-pod artifact parent");
    }
    write_live_product_pod_ready_deck_artifact(&artifact, samples, traces);
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VZ ready-deck density proof; boots a product pod"]
async fn live_runtime_product_pod_ready_deck_density_writes_breakpoint_sample() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let density_levels = live_runtime_density_levels(
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS").as_deref(),
        &[1, 2, 4, 8, 16, 24, 32],
    );
    let (samples, traces) =
        collect_live_product_pod_ready_deck_density_samples(&density_levels).await;
    let sample = samples
        .iter()
        .find(|sample| sample.metric() == MAX_AGENT_COMPUTERS_BEFORE_READY_P95_DOUBLES_METRIC)
        .expect("ready-deck density breakpoint sample");

    assert_eq!(
        sample.metric(),
        MAX_AGENT_COMPUTERS_BEFORE_READY_P95_DOUBLES_METRIC
    );
    assert_eq!(
        sample.tag_value("measurement_boundary"),
        Some("product_path")
    );
    assert_eq!(
        sample.tag_value("pod_surface"),
        Some("product_pod_ready_deck")
    );
    assert!(!traces.is_empty());
    let artifact = live_runtime_product_pod_ready_deck_density_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_READY_DECK_DENSITY_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create ready-deck density artifact parent");
    }
    write_live_product_pod_ready_deck_density_artifact(&artifact, samples, traces);
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VZ prestarted agent-slot density proof; boots a product pod"]
async fn live_runtime_product_pod_prestarted_agent_slot_density_writes_breakpoint_sample() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let density_levels = live_runtime_density_levels(
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS").as_deref(),
        &[1, 2, 4, 8, 16, 24, 32],
    );
    let (samples, traces) =
        collect_live_product_pod_prestarted_agent_slot_density_samples(&density_levels).await;
    let sample = samples
        .iter()
        .find(|sample| {
            sample.metric() == MAX_PRESTARTED_AGENT_SLOTS_BEFORE_CHECKOUT_READY_P95_DOUBLES_METRIC
        })
        .expect("prestarted agent-slot density breakpoint sample");

    assert_eq!(
        sample.metric(),
        MAX_PRESTARTED_AGENT_SLOTS_BEFORE_CHECKOUT_READY_P95_DOUBLES_METRIC
    );
    assert_eq!(
        sample.tag_value("measurement_boundary"),
        Some("prestarted_slot_checkout")
    );
    assert_eq!(
        sample.tag_value("slot_surface"),
        Some("prestarted_agent_slot")
    );
    assert_eq!(sample.tag_value("excludes_container_add"), Some("true"));
    let expected_traces = density_levels.iter().sum::<usize>();
    assert_eq!(
        traces.len(),
        expected_traces,
        "each configured prestarted slot density level must preserve one trace per checked-out slot"
    );
    let artifact = live_runtime_product_pod_prestarted_agent_slot_density_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_ARTIFACT")
            .as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent)
            .expect("create prestarted agent-slot density artifact parent");
    }
    write_live_product_pod_prestarted_agent_slot_density_artifact(&artifact, samples, traces);
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VM-backed retained-shell batch-100 proof; requires signed test harness"]
async fn live_runtime_retained_shell_batch_100_writes_snappy_sample() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-batch-100-retained-shell";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let repeats = live_runtime_repeat_count(
        std::env::var_os("FIRKIN_LIVE_RETAINED_BATCH_REPEATS").as_deref(),
    );
    let mut samples = Vec::with_capacity(repeats);
    let mut traces = Vec::with_capacity(repeats);
    for repeat in 0..repeats {
        let (sample, event_trace) = collect_retained_shell_batch_100_sample(&sandbox, repeat).await;
        assert!(
            sample.value() < 500.0,
            "retained shell batch-100 should be snappy, got {}ms",
            sample.value()
        );
        samples.push(sample);
        traces.push(event_trace);
    }
    let artifact = live_runtime_retained_batch_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_RETAINED_BATCH_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create retained batch artifact parent");
    }
    write_live_retained_batch_artifact(&artifact, samples, traces);
    assert!(artifact.exists());
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed warm start proof; requires signed test harness"]
async fn live_runtime_warm_to_first_stdout_writes_repeat_samples() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let repeats = live_runtime_repeat_count(
        std::env::var_os("FIRKIN_LIVE_WARM_TO_FIRST_STDOUT_REPEATS").as_deref(),
    );
    let mut samples = Vec::with_capacity(repeats);
    for repeat in 0..repeats {
        let repeat_samples = collect_live_warm_ready_benchmark_samples().await;
        assert!(
            repeat_samples
                .iter()
                .any(|sample| sample.metric() == "start.warm_to_first_stdout_ms"),
            "warm-to-first-stdout sample"
        );
        samples.extend(tag_live_repeat_samples(repeat_samples, repeat, repeats));
    }
    let artifact = live_runtime_warm_to_first_stdout_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_WARM_TO_FIRST_STDOUT_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create warm-to-first-stdout artifact parent");
    }
    write_live_raw_sample_artifact(&artifact, "live_warm_to_first_stdout", samples);
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VM-backed hot start proof; requires signed test harness"]
async fn live_runtime_hot_to_first_stdout_writes_repeat_samples() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let repeats = live_runtime_repeat_count_with_default(
        std::env::var_os("FIRKIN_LIVE_HOT_TO_FIRST_STDOUT_REPEATS").as_deref(),
        10,
    );
    let samples = collect_live_hot_to_first_stdout_samples(repeats).await;
    assert_eq!(samples.len(), repeats);
    let artifact = live_runtime_hot_to_first_stdout_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_HOT_TO_FIRST_STDOUT_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create hot-to-first-stdout artifact parent");
    }
    write_live_raw_sample_artifact(&artifact, "live_hot_to_first_stdout", samples);
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VM-backed resume proof; requires signed test harness"]
async fn live_runtime_resume_to_first_stdout_writes_repeat_samples() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let repeats = live_runtime_repeat_count(
        std::env::var_os("FIRKIN_LIVE_RESUME_TO_FIRST_STDOUT_REPEATS").as_deref(),
    );
    let mut samples = Vec::with_capacity(repeats);
    let restore_timing_samples = Arc::new(Mutex::new(Vec::new()));
    for repeat in 0..repeats {
        let repeat_samples = collect_live_resume_to_first_stdout_benchmark_samples_with_timings(
            Some(Arc::clone(&restore_timing_samples)),
        )
        .await;
        assert!(
            repeat_samples
                .iter()
                .any(|sample| sample.metric() == "start.resume_to_first_stdout_ms"),
            "resume-to-first-stdout sample"
        );
        samples.extend(tag_live_repeat_samples(repeat_samples, repeat, repeats));
    }
    let artifact = live_runtime_resume_to_first_stdout_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_RESUME_TO_FIRST_STDOUT_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create resume-to-first-stdout artifact parent");
    }
    write_live_raw_sample_artifact(&artifact, "live_resume_to_first_stdout", samples);
    write_restore_timing_artifact(
        &artifact.with_extension("restore-timings.json"),
        &restore_timing_samples
            .lock()
            .expect("lock restore timing samples"),
    );
    assert!(artifact.exists());
}

#[tokio::test]
#[ignore = "live VM-backed retained-shell density proof; requires signed test harness"]
async fn live_runtime_retained_shell_density_reuse_writes_repeat_samples() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let density_levels = live_runtime_density_levels(
        std::env::var_os("FIRKIN_LIVE_RETAINED_SHELL_DENSITY_LEVELS").as_deref(),
        &[1, 2, 4, 8],
    );
    let repeats = live_runtime_repeat_count_with_default(
        std::env::var_os("FIRKIN_LIVE_RETAINED_SHELL_DENSITY_REPEATS").as_deref(),
        10,
    );
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-retained-shell-density";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter.clone(), &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let samples =
        collect_retained_shell_density_reused_samples(&adapter, &sandbox, &density_levels, repeats)
            .await;
    assert_eq!(samples.len(), density_levels.len() * repeats);
    let artifact = live_runtime_retained_shell_density_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_RETAINED_SHELL_DENSITY_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create retained shell density artifact parent");
    }
    write_live_retained_shell_density_artifact(&artifact, samples, Vec::new());
    assert!(artifact.exists());
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed retained-shell send-path proof; requires signed test harness"]
async fn live_runtime_retained_shell_send_path_writes_repeat_samples() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let repeats = live_runtime_repeat_count_with_default(
        std::env::var_os("FIRKIN_LIVE_RETAINED_SHELL_SEND_PATH_REPEATS").as_deref(),
        10,
    );
    let density_levels = live_runtime_density_levels(
        std::env::var_os("FIRKIN_LIVE_RETAINED_SHELL_SEND_PATH_LEVELS").as_deref(),
        &[1, 2, 4, 8],
    );
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-retained-shell-send-path";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter.clone(), &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let client = reqwest::Client::new();
    let envd_url = live_envd_url_for_sandbox(&adapter, sandbox.sandbox_id()).await;
    let samples = collect_retained_shell_send_path_samples(
        &sandbox,
        &client,
        &proxy_url,
        &envd_url,
        repeats,
        &density_levels,
    )
    .await;
    assert_eq!(samples.len(), repeats * 6 * (density_levels.len() + 1));
    let artifact = live_runtime_retained_shell_send_path_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_RETAINED_SHELL_SEND_PATH_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create retained shell send-path artifact parent");
    }
    write_live_raw_sample_artifact(&artifact, "live_retained_shell_send_path", samples);
    assert!(artifact.exists());
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed direct exec proof; requires signed test harness"]
async fn live_runtime_direct_exec_first_stdout_writes_repeat_samples() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let repeats = live_runtime_repeat_count_with_default(
        std::env::var_os("FIRKIN_LIVE_DIRECT_EXEC_FIRST_STDOUT_REPEATS").as_deref(),
        10,
    );
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-direct-exec-first-stdout";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let sandbox = start_live_envd_sandbox(&adapter, &snapshot_path).await;
    let samples = collect_direct_exec_first_stdout_samples(&adapter, repeats).await;
    assert_eq!(samples.len(), repeats * 2);
    let artifact = live_runtime_direct_exec_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_DIRECT_EXEC_FIRST_STDOUT_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create direct exec artifact parent");
    }
    write_live_direct_exec_first_stdout_artifact(&artifact, samples);
    assert!(artifact.exists());
    adapter
        .stop(&sandbox.config.sandbox_id)
        .await
        .expect("stop direct exec proof sandbox");
}

#[tokio::test]
#[ignore = "live product-pod disk reclaim proof; boots a product pod"]
async fn live_runtime_product_pod_disk_reclaim_writes_staged_samples() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let image_format = live_runtime_product_pod_disk_reclaim_image_format(
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_DISK_RECLAIM_IMAGE_FORMAT").as_deref(),
    );
    let repeats = live_runtime_repeat_count_with_default(
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_DISK_RECLAIM_REPEATS").as_deref(),
        1,
    );
    let mut samples = Vec::new();
    for repeat in 0..repeats {
        samples.extend(tag_live_repeat_samples(
            collect_live_product_pod_disk_reclaim_samples_for_format(image_format).await,
            repeat,
            repeats,
        ));
    }
    assert!(
        samples
            .iter()
            .any(|sample| sample.metric() == SPARSE_BLOAT_AFTER_DELETE_METRIC),
        "disk reclaim artifact should include pre-trim sparse bloat"
    );
    assert!(
        samples
            .iter()
            .any(|sample| sample.metric() == "disk.sparse_bloat_after_trim"),
        "disk reclaim artifact should include post-trim sparse bloat"
    );
    assert!(
        samples
            .iter()
            .any(|sample| sample.metric() == HOST_BYTES_RECLAIMED_AFTER_TRIM_METRIC),
        "disk reclaim artifact should include host reclaim delta"
    );
    let artifact = live_runtime_product_pod_disk_reclaim_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_DISK_RECLAIM_ARTIFACT").as_deref(),
    );
    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create disk reclaim artifact parent");
    }
    write_live_product_pod_disk_reclaim_artifact(&artifact, samples);
    assert!(artifact.exists());
}

#[test]
fn warm_template_targets_for_depth_repeats_ready_templates() {
    let targets = warm_template_targets_for_depth(
        vec![PreparedTemplate {
            template_id: "repo-main".to_owned(),
            build_id: "build-1".to_owned(),
            artifact: "/tmp/repo-main.vz".to_owned(),
            has_envd: true,
            artifact_integrity: None,
        }],
        2,
    );

    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].template_id, "repo-main");
    assert_eq!(targets[1].template_id, "repo-main");
}

#[test]
fn restore_timing_artifact_json_records_restore_phases() {
    let timings = vec![firkin_runtime::core::ContainerRestoreTimings::new(
        Duration::from_millis(7),
        Duration::from_millis(11),
        Duration::from_millis(22),
        Some(firkin_runtime::core::RestoredRootfsStage::new(
            firkin_runtime::core::RestoredRootfsStageMethod::Clone,
            4096,
            Duration::from_millis(5),
        )),
    )];

    let value = restore_timing_artifact_json(&timings);

    assert_eq!(value["restore_count"], 1);
    assert_eq!(value["restores"][0]["staging_ms"], 7.0);
    assert_eq!(value["restores"][0]["vm_restore_ms"], 11.0);
    assert_eq!(value["restores"][0]["vminitd_connect_ms"], 22.0);
    assert_eq!(value["restores"][0]["rootfs_stage_method"], "clone");
    assert_eq!(value["restores"][0]["rootfs_stage_source_bytes"], 4096);
    assert_eq!(value["restores"][0]["rootfs_stage_ms"], 5.0);
}

fn live_runtime_benchmark_artifact_path(temp: &Path, override_path: Option<&OsStr>) -> PathBuf {
    override_path.map_or_else(|| temp.join("live-benchmark-evidence.json"), PathBuf::from)
}

fn live_runtime_overhead_artifact_path(temp: &Path, override_path: Option<&OsStr>) -> PathBuf {
    override_path.map_or_else(|| temp.join("live-overhead-evidence.json"), PathBuf::from)
}

fn live_runtime_agent_computer_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-agent-computer-scorecard.json"),
        PathBuf::from,
    )
}

fn live_runtime_autoscale_artifact_path(temp: &Path, override_path: Option<&OsStr>) -> PathBuf {
    override_path.map_or_else(|| temp.join("live-autoscale-scorecard.json"), PathBuf::from)
}

fn live_runtime_db_sidecar_artifact_path(temp: &Path, override_path: Option<&OsStr>) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-db-sidecar-readiness.json"),
        PathBuf::from,
    )
}

fn live_runtime_browser_sidecar_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-browser-sidecar-readiness.json"),
        PathBuf::from,
    )
}

fn live_runtime_product_pod_artifact_path(temp: &Path, override_path: Option<&OsStr>) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-product-pod-readiness.json"),
        PathBuf::from,
    )
}

fn live_runtime_product_pod_ready_deck_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-product-pod-ready-deck.json"),
        PathBuf::from,
    )
}

fn live_runtime_product_pod_ready_deck_density_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-product-pod-ready-deck-density.json"),
        PathBuf::from,
    )
}

fn live_runtime_product_pod_prestarted_agent_slot_density_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-product-pod-prestarted-agent-slot-density.json"),
        PathBuf::from,
    )
}

fn live_runtime_retained_batch_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(|| temp.join("live-retained-batch-100.json"), PathBuf::from)
}

fn live_runtime_warm_to_first_stdout_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-warm-to-first-stdout.json"),
        PathBuf::from,
    )
}

fn live_runtime_hot_to_first_stdout_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(|| temp.join("live-hot-to-first-stdout.json"), PathBuf::from)
}

fn live_runtime_resume_to_first_stdout_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-resume-to-first-stdout.json"),
        PathBuf::from,
    )
}

fn live_runtime_retained_shell_density_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-retained-shell-density.json"),
        PathBuf::from,
    )
}

fn live_runtime_retained_shell_send_path_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-retained-shell-send-path.json"),
        PathBuf::from,
    )
}

fn live_runtime_direct_exec_artifact_path(temp: &Path, override_path: Option<&OsStr>) -> PathBuf {
    override_path.map_or_else(|| temp.join("live-direct-exec.json"), PathBuf::from)
}

fn live_runtime_product_pod_disk_reclaim_artifact_path(
    temp: &Path,
    override_path: Option<&OsStr>,
) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-product-pod-disk-reclaim.json"),
        PathBuf::from,
    )
}

fn live_runtime_product_pod_disk_reclaim_image_format(
    value: Option<&OsStr>,
) -> PodStoreImageFormat {
    match value.and_then(|value| value.to_str()).unwrap_or("raw") {
        "asif" => PodStoreImageFormat::Asif,
        "raw" => PodStoreImageFormat::Raw,
        other => panic!("unsupported product pod disk reclaim image format: {other}"),
    }
}

fn live_runtime_repeat_count(value: Option<&OsStr>) -> usize {
    live_runtime_repeat_count_with_default(value, 1)
}

fn live_runtime_repeat_count_with_default(value: Option<&OsStr>, default_count: usize) -> usize {
    value
        .and_then(|value| value.to_string_lossy().parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(default_count)
}

fn live_runtime_density_levels(value: Option<&OsStr>, default: &[usize]) -> Vec<usize> {
    let levels = value.map_or_else(
        || default.to_vec(),
        |value| {
            value
                .to_string_lossy()
                .split(',')
                .map(|part| {
                    part.trim()
                        .parse::<usize>()
                        .unwrap_or_else(|_| panic!("invalid live density level: {part}"))
                })
                .collect::<Vec<_>>()
        },
    );
    assert!(
        levels.contains(&1),
        "live density levels must include single-sandbox baseline level 1"
    );
    assert!(
        levels.iter().all(|level| *level > 0),
        "live density levels must be positive"
    );
    levels
}

fn live_density_capacity_for_max_active(max_active: usize) -> ResourceBudget {
    let max_active = max_active.max(4);
    let max_active_u32 = u32::try_from(max_active).expect("live density level fits in u32");
    let max_active_u64 = u64::try_from(max_active).expect("live density level fits in u64");
    ResourceBudget::new(
        max_active_u32.saturating_mul(2),
        Size::gib(max_active_u64.saturating_mul(8)),
        Size::gib(max_active_u64.saturating_mul(64)),
    )
}

fn density_level_tag(levels: &[usize]) -> String {
    levels
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn write_restore_timing_artifact(
    path: &Path,
    timings: &[firkin_runtime::core::ContainerRestoreTimings],
) {
    let value = restore_timing_artifact_json(timings);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create restore timing artifact parent");
    }
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&value).expect("encode restore timing artifact"),
    )
    .expect("write restore timing artifact");
}

fn restore_timing_artifact_json(
    timings: &[firkin_runtime::core::ContainerRestoreTimings],
) -> serde_json::Value {
    serde_json::json!({
        "restore_count": timings.len(),
        "restores": timings.iter().map(|timing| {
            serde_json::json!({
                "staging_ms": timing.staging().as_secs_f64() * 1000.0,
                "vm_restore_ms": timing.vm_restore().as_secs_f64() * 1000.0,
                "vminitd_connect_ms": timing.vminitd_connect().as_secs_f64() * 1000.0,
                "rootfs_stage_method": timing
                    .rootfs_stage()
                    .map(|stage| stage.method().as_str()),
                "rootfs_stage_source_bytes": timing
                    .rootfs_stage()
                    .map(firkin_runtime::core::RestoredRootfsStage::source_bytes),
                "rootfs_stage_ms": timing
                    .rootfs_stage()
                    .map(|stage| stage.elapsed().as_secs_f64() * 1000.0),
            })
        }).collect::<Vec<_>>(),
    })
}

fn agent_computer_duration_sample(metric: &'static str, elapsed: Duration) -> BenchmarkSample {
    BenchmarkSample::from_static(
        metric,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        elapsed.as_secs_f64() * 1000.0,
    )
    .with_static_tag("source", "signed-live-agent-computer-product-path")
}

fn agent_computer_count_sample(metric: &'static str, value: u64) -> BenchmarkSample {
    BenchmarkSample::from_static(
        metric,
        BenchmarkMetricKind::WorkloadResource,
        BenchmarkUnit::Count,
        value as f64,
    )
    .with_static_tag("source", "signed-live-agent-computer-product-path")
}

fn agent_computer_zero_guardrail_samples() -> [BenchmarkSample; 2] {
    [
        BenchmarkSample::from_static(
            "cleanup.leftover_bytes",
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Bytes,
            0.0,
        )
        .with_static_tag("source", "signed-live-agent-computer-product-path"),
        BenchmarkSample::from_static(
            "reliability.unknown_failure_rate",
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Percent,
            0.0,
        )
        .with_static_tag("source", "signed-live-agent-computer-product-path"),
    ]
}

fn assert_agent_computer_scorecard_samples_cover_required_metrics(samples: &[BenchmarkSample]) {
    let present = samples
        .iter()
        .map(BenchmarkSample::metric)
        .collect::<std::collections::BTreeSet<_>>();

    for metric in AGENT_COMPUTER_SCORECARD_METRICS {
        assert!(present.contains(metric), "missing {metric}");
    }
}

#[test]
fn live_runtime_benchmark_artifact_path_uses_override_when_present() {
    assert_eq!(
        live_runtime_benchmark_artifact_path(
            Path::new("/tmp/firkin-live-temp"),
            Some(std::ffi::OsStr::new("/tmp/firkin-live-evidence.json"))
        ),
        PathBuf::from("/tmp/firkin-live-evidence.json")
    );
    assert_eq!(
        live_runtime_benchmark_artifact_path(Path::new("/tmp/firkin-live-temp"), None),
        PathBuf::from("/tmp/firkin-live-temp/live-benchmark-evidence.json")
    );
}

#[test]
fn live_runtime_overhead_artifact_path_uses_override_when_present() {
    assert_eq!(
        live_runtime_overhead_artifact_path(
            Path::new("/tmp/firkin-live-temp"),
            Some(std::ffi::OsStr::new("/tmp/firkin-live-overhead.json"))
        ),
        PathBuf::from("/tmp/firkin-live-overhead.json")
    );
    assert_eq!(
        live_runtime_overhead_artifact_path(Path::new("/tmp/firkin-live-temp"), None),
        PathBuf::from("/tmp/firkin-live-temp/live-overhead-evidence.json")
    );
}

#[test]
fn live_runtime_agent_computer_artifact_path_uses_override_when_present() {
    assert_eq!(
        live_runtime_agent_computer_artifact_path(
            Path::new("/tmp/firkin-live-temp"),
            Some(std::ffi::OsStr::new(
                "/tmp/firkin-live-agent-computer-scorecard.json"
            ))
        ),
        PathBuf::from("/tmp/firkin-live-agent-computer-scorecard.json")
    );
    assert_eq!(
        live_runtime_agent_computer_artifact_path(Path::new("/tmp/firkin-live-temp"), None),
        PathBuf::from("/tmp/firkin-live-temp/live-agent-computer-scorecard.json")
    );
}

#[test]
fn live_runtime_autoscale_artifact_path_uses_override_when_present() {
    assert_eq!(
        live_runtime_autoscale_artifact_path(
            Path::new("/tmp/firkin-live-temp"),
            Some(std::ffi::OsStr::new(
                "/tmp/firkin-live-autoscale-scorecard.json"
            ))
        ),
        PathBuf::from("/tmp/firkin-live-autoscale-scorecard.json")
    );
    assert_eq!(
        live_runtime_autoscale_artifact_path(Path::new("/tmp/firkin-live-temp"), None),
        PathBuf::from("/tmp/firkin-live-temp/live-autoscale-scorecard.json")
    );
}

#[test]
fn live_runtime_retained_shell_send_path_artifact_path_uses_override_when_present() {
    assert_eq!(
        live_runtime_retained_shell_send_path_artifact_path(
            Path::new("/tmp/firkin-live-temp"),
            Some(std::ffi::OsStr::new(
                "/tmp/firkin-live-retained-shell-send-path.json"
            ))
        ),
        PathBuf::from("/tmp/firkin-live-retained-shell-send-path.json")
    );
    assert_eq!(
        live_runtime_retained_shell_send_path_artifact_path(
            Path::new("/tmp/firkin-live-temp"),
            None
        ),
        PathBuf::from("/tmp/firkin-live-temp/live-retained-shell-send-path.json")
    );
}

#[test]
fn live_runtime_db_sidecar_artifact_path_uses_override_when_present() {
    assert_eq!(
        live_runtime_db_sidecar_artifact_path(
            Path::new("/tmp/firkin-live-temp"),
            Some(std::ffi::OsStr::new(
                "/tmp/firkin-live-db-sidecar-readiness.json"
            ))
        ),
        PathBuf::from("/tmp/firkin-live-db-sidecar-readiness.json")
    );
    assert_eq!(
        live_runtime_db_sidecar_artifact_path(Path::new("/tmp/firkin-live-temp"), None),
        PathBuf::from("/tmp/firkin-live-temp/live-db-sidecar-readiness.json")
    );
}

#[test]
fn live_runtime_browser_sidecar_artifact_path_uses_override_when_present() {
    assert_eq!(
        live_runtime_browser_sidecar_artifact_path(
            Path::new("/tmp/firkin-live-temp"),
            Some(std::ffi::OsStr::new(
                "/tmp/firkin-live-browser-sidecar-readiness.json"
            ))
        ),
        PathBuf::from("/tmp/firkin-live-browser-sidecar-readiness.json")
    );
    assert_eq!(
        live_runtime_browser_sidecar_artifact_path(Path::new("/tmp/firkin-live-temp"), None),
        PathBuf::from("/tmp/firkin-live-temp/live-browser-sidecar-readiness.json")
    );
}

#[test]
fn agent_computer_scorecard_samples_cover_required_metrics() {
    let samples = vec![
        agent_computer_duration_sample("product.agent_computer_ready_ms", Duration::from_millis(1)),
        agent_computer_duration_sample(
            "product.agent_computer_resume_ms",
            Duration::from_millis(1),
        ),
        agent_computer_count_sample(MAX_AGENT_COMPUTERS_BEFORE_READY_P95_DOUBLES_METRIC, 1),
        agent_computer_zero_guardrail_samples()[0].clone(),
        agent_computer_zero_guardrail_samples()[1].clone(),
    ];

    assert_agent_computer_scorecard_samples_cover_required_metrics(&samples);
}

#[test]
fn proxy_database_boundary_tags_product_readiness_samples() {
    let ready = mark_proxy_database_boundary(agent_computer_duration_sample(
        "product.agent_computer_ready_ms",
        Duration::from_millis(1),
    ));
    let resume = mark_proxy_database_boundary(agent_computer_duration_sample(
        "product.agent_computer_resume_ms",
        Duration::from_millis(1),
    ));
    let database = mark_proxy_database_boundary(agent_computer_duration_sample(
        "product.database_ready_ms",
        Duration::from_millis(1),
    ));
    let cli = mark_proxy_database_boundary(agent_computer_duration_sample(
        "product.cli_ready_ms",
        Duration::from_millis(1),
    ));

    for sample in [&ready, &resume, &database] {
        assert_eq!(
            sample.tag_value("database_boundary"),
            Some("sqlite_proxy_not_db_sidecar")
        );
    }
    for sample in [&ready, &resume] {
        assert_eq!(
            sample.tag_value("cli_boundary"),
            Some("code_interpreter_exec")
        );
        assert_eq!(
            sample.tag_value("browser_boundary"),
            Some("code_interpreter_health")
        );
        assert_eq!(sample.tag_value("database_probe_surface"), None);
    }
    assert_eq!(
        database.tag_value("database_probe_surface"),
        Some("code_interpreter_sqlite")
    );
    assert_eq!(cli.tag_value("database_boundary"), None);
    assert_eq!(cli.tag_value("cli_boundary"), None);
    assert_eq!(cli.tag_value("browser_boundary"), None);
}

#[test]
fn real_db_sidecar_database_sample_is_promotable_database_boundary() {
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    trace.record_at_elapsed(SandboxEventName::DatabaseReady, Duration::from_millis(12));
    let event_trace = trace.finish();

    let sample = real_db_sidecar_database_sample(&event_trace);

    assert_eq!(sample.metric(), "product.database_ready_ms");
    assert!(
        (11.0..=12.0).contains(&sample.value()),
        "unexpected database readiness value: {}",
        sample.value()
    );
    assert_eq!(sample.tag_value("probe_surface"), Some("db_sidecar_health"));
    assert_eq!(sample.tag_value("measurement_boundary"), Some("db_sidecar"));
    assert_eq!(
        sample.tag_value("database_boundary"),
        Some("real_db_sidecar")
    );
    assert_eq!(sample.tag_value("pod_surface"), Some("product_pod"));
}

#[test]
fn db_sidecar_readiness_artifact_preserves_sample_and_trace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("db-sidecar.json");
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    trace.record_at_elapsed(SandboxEventName::DatabaseReady, Duration::from_millis(10));
    let event_trace = trace.finish();
    let sample = real_db_sidecar_database_sample(&event_trace);

    write_live_db_sidecar_readiness_artifact(&artifact, sample, event_trace);

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read DB sidecar readiness artifact"),
    )
    .expect("parse DB sidecar readiness artifact");
    assert_eq!(value["kind"], "live_db_sidecar_readiness");
    assert_eq!(value["samples"][0]["metric"], "product.database_ready_ms");
    assert_eq!(
        value["samples"][0]["tags"]["database_boundary"],
        "real_db_sidecar"
    );
    assert_eq!(value["traces"][0]["events"][1]["name"], "DatabaseReady");
}

#[test]
fn real_browser_sidecar_browser_sample_is_promotable_browser_boundary() {
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    trace.record_at_elapsed(SandboxEventName::BrowserReady, Duration::from_millis(9));
    let event_trace = trace.finish();

    let sample = real_browser_sidecar_browser_sample(&event_trace);

    assert_eq!(sample.metric(), "product.browser_ready_ms");
    assert!(
        (8.0..=9.0).contains(&sample.value()),
        "unexpected browser readiness value: {}",
        sample.value()
    );
    assert_eq!(
        sample.tag_value("probe_surface"),
        Some("browser_sidecar_health")
    );
    assert_eq!(
        sample.tag_value("measurement_boundary"),
        Some("browser_sidecar")
    );
    assert_eq!(
        sample.tag_value("browser_boundary"),
        Some("real_browser_sidecar")
    );
    assert_eq!(sample.tag_value("pod_surface"), Some("product_pod"));
}

#[test]
fn browser_sidecar_readiness_artifact_preserves_sample_and_trace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("browser-sidecar.json");
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    trace.record_at_elapsed(SandboxEventName::BrowserReady, Duration::from_millis(10));
    let event_trace = trace.finish();
    let sample = real_browser_sidecar_browser_sample(&event_trace);

    write_live_browser_sidecar_readiness_artifact(&artifact, sample, event_trace);

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read browser sidecar readiness artifact"),
    )
    .expect("parse browser sidecar readiness artifact");
    assert_eq!(value["kind"], "live_browser_sidecar_readiness");
    assert_eq!(value["samples"][0]["metric"], "product.browser_ready_ms");
    assert_eq!(
        value["samples"][0]["tags"]["browser_boundary"],
        "real_browser_sidecar"
    );
    assert_eq!(value["traces"][0]["events"][1]["name"], "BrowserReady");
}

#[test]
fn real_product_pod_readiness_sample_requires_all_product_boundaries() {
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    trace.record_at_elapsed(
        SandboxEventName::CliFirstUsefulStdout,
        Duration::from_millis(5),
    );
    trace.record_at_elapsed(SandboxEventName::DatabaseReady, Duration::from_millis(8));
    trace.record_at_elapsed(SandboxEventName::BrowserReady, Duration::from_millis(11));
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerReady,
        Duration::from_millis(12),
    );
    let event_trace = trace.finish();

    let sample = real_product_pod_readiness_sample(&event_trace);

    assert_eq!(sample.metric(), "product.agent_computer_ready_ms");
    assert!(
        (11.0..=12.0).contains(&sample.value()),
        "unexpected product readiness value: {}",
        sample.value()
    );
    assert_eq!(
        sample.tag_value("measurement_boundary"),
        Some("product_path")
    );
    assert_eq!(sample.tag_value("cli_boundary"), Some("real_cli"));
    assert_eq!(
        sample.tag_value("browser_boundary"),
        Some("real_browser_sidecar")
    );
    assert_eq!(
        sample.tag_value("database_boundary"),
        Some("real_db_sidecar")
    );
    assert_eq!(sample.tag_value("pod_surface"), Some("product_pod"));
}

#[test]
fn product_pod_readiness_artifact_preserves_real_boundary_tags() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("product-pod.json");
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    trace.record_at_elapsed(
        SandboxEventName::CliFirstUsefulStdout,
        Duration::from_millis(5),
    );
    trace.record_at_elapsed(SandboxEventName::DatabaseReady, Duration::from_millis(8));
    trace.record_at_elapsed(SandboxEventName::BrowserReady, Duration::from_millis(11));
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerReady,
        Duration::from_millis(12),
    );
    let event_trace = trace.finish();
    let sample = real_product_pod_readiness_sample(&event_trace);

    write_live_product_pod_readiness_artifact(&artifact, sample, event_trace);

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read product-pod readiness artifact"),
    )
    .expect("parse product-pod readiness artifact");
    assert_eq!(value["kind"], "live_product_pod_readiness");
    assert_eq!(
        value["samples"][0]["metric"],
        "product.agent_computer_ready_ms"
    );
    assert_eq!(value["samples"][0]["tags"]["cli_boundary"], "real_cli");
    assert_eq!(
        value["samples"][0]["tags"]["browser_boundary"],
        "real_browser_sidecar"
    );
    assert_eq!(
        value["samples"][0]["tags"]["database_boundary"],
        "real_db_sidecar"
    );
    assert_eq!(
        value["traces"][0]["events"][4]["name"],
        "AgentComputerReady"
    );
}

#[test]
fn real_product_pod_ready_deck_sample_is_promotable_resume_boundary() {
    let mut trace = agent_computer_event_trace(LifecycleClass::Resumed);
    trace.record(SandboxEventName::AgentComputerResumed);
    trace.record_at_elapsed(
        SandboxEventName::CliFirstUsefulStdout,
        Duration::from_millis(21),
    );
    trace.record_at_elapsed(SandboxEventName::DatabaseReady, Duration::from_millis(21));
    trace.record_at_elapsed(SandboxEventName::BrowserReady, Duration::from_millis(21));
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerReady,
        Duration::from_millis(22),
    );
    let event_trace = trace.finish();

    let sample = real_product_pod_ready_deck_sample(&event_trace);

    assert_eq!(sample.metric(), "product.agent_computer_resume_ms");
    assert!(
        (21.0..=22.0).contains(&sample.value()),
        "unexpected ready-deck value: {}",
        sample.value()
    );
    assert_eq!(
        sample.tag_value("measurement_boundary"),
        Some("product_path")
    );
    assert_eq!(sample.tag_value("cli_boundary"), Some("real_cli"));
    assert_eq!(
        sample.tag_value("browser_boundary"),
        Some("real_browser_sidecar")
    );
    assert_eq!(
        sample.tag_value("database_boundary"),
        Some("real_db_sidecar")
    );
    assert_eq!(
        sample.tag_value("pod_surface"),
        Some("product_pod_ready_deck")
    );
    assert_eq!(
        sample.tag_value("slot_surface"),
        Some("prestarted_agent_slot")
    );
    assert_eq!(sample.tag_value("excludes_container_add"), Some("true"));
    assert_eq!(
        sample.tag_value("ready_signal"),
        Some("request_fifo_acceptance")
    );
    assert_eq!(sample.tag_value("output_wait_preserved"), Some("true"));
}

#[test]
fn product_pod_ready_deck_artifact_preserves_resume_boundary_tags() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("product-pod-ready-deck.json");
    let mut trace = agent_computer_event_trace(LifecycleClass::Resumed);
    trace.record(SandboxEventName::AgentComputerResumed);
    trace.record_at_elapsed(
        SandboxEventName::CliFirstUsefulStdout,
        Duration::from_millis(21),
    );
    trace.record_at_elapsed(SandboxEventName::DatabaseReady, Duration::from_millis(21));
    trace.record_at_elapsed(SandboxEventName::BrowserReady, Duration::from_millis(21));
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerReady,
        Duration::from_millis(22),
    );
    let event_trace = trace.finish();
    let sample = real_product_pod_ready_deck_sample(&event_trace);

    write_live_product_pod_ready_deck_artifact(&artifact, vec![sample], vec![event_trace]);

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read ready-deck product-pod artifact"),
    )
    .expect("parse ready-deck product-pod artifact");
    assert_eq!(value["kind"], "live_product_pod_ready_deck");
    assert_eq!(
        value["samples"][0]["metric"],
        "product.agent_computer_resume_ms"
    );
    assert_eq!(value["samples"][0]["tags"]["cli_boundary"], "real_cli");
    assert_eq!(
        value["samples"][0]["tags"]["browser_boundary"],
        "real_browser_sidecar"
    );
    assert_eq!(
        value["samples"][0]["tags"]["database_boundary"],
        "real_db_sidecar"
    );
    assert_eq!(
        value["samples"][0]["tags"]["slot_surface"],
        "prestarted_agent_slot"
    );
    assert_eq!(
        value["samples"][0]["tags"]["excludes_container_add"],
        "true"
    );
    assert_eq!(
        value["samples"][0]["tags"]["ready_signal"],
        "request_fifo_acceptance"
    );
    assert_eq!(value["samples"][0]["tags"]["output_wait_preserved"], "true");
    assert_eq!(
        value["traces"][0]["events"][4]["name"],
        "AgentComputerReady"
    );
}

#[test]
fn product_pod_ready_deck_artifact_tags_samples_with_count_confidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("product-pod-ready-deck.json");
    let samples = (0..100)
        .map(|offset| {
            let mut trace = agent_computer_event_trace(LifecycleClass::Resumed);
            trace.record(SandboxEventName::AgentComputerResumed);
            trace.record_at_elapsed(
                SandboxEventName::AgentComputerSandboxCreated,
                Duration::from_millis(7 + offset),
            );
            trace.record_at_elapsed(
                SandboxEventName::AgentComputerProbeStart,
                Duration::from_millis(11 + offset),
            );
            trace.record_at_elapsed(
                SandboxEventName::CliFirstUsefulStdout,
                Duration::from_millis(20 + offset),
            );
            trace.record_at_elapsed(
                SandboxEventName::DatabaseReady,
                Duration::from_millis(20 + offset),
            );
            trace.record_at_elapsed(
                SandboxEventName::BrowserReady,
                Duration::from_millis(20 + offset),
            );
            trace.record_at_elapsed(
                SandboxEventName::AgentComputerReady,
                Duration::from_millis(21 + offset),
            );
            real_product_pod_ready_deck_sample(&trace.finish())
        })
        .collect::<Vec<_>>();

    write_live_product_pod_ready_deck_artifact(&artifact, samples, Vec::new());

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read ready-deck product-pod artifact"),
    )
    .expect("parse ready-deck product-pod artifact");
    assert_eq!(
        value["samples"][0]["tags"]["confidence"],
        PercentileAvailability::P95DecisionGrade.as_str()
    );
    assert_eq!(
        value["samples"][99]["tags"]["confidence"],
        PercentileAvailability::P95DecisionGrade.as_str()
    );
}

#[test]
fn product_pod_ready_deck_density_artifact_preserves_metric_and_traces() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("product-pod-ready-deck-density.json");
    let mut trace = agent_computer_event_trace(LifecycleClass::Resumed);
    trace.record(SandboxEventName::AgentComputerResumed);
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerSandboxCreated,
        Duration::from_millis(12),
    );
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerProbeStart,
        Duration::from_millis(12),
    );
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerReady,
        Duration::from_millis(20),
    );
    let event_trace = trace.finish();
    let level_sample = product_pod_ready_deck_density_level_sample(4, "1,4", 35.0);
    let add_sample = product_pod_ready_deck_container_add_level_sample(4, "1,4", 12.0);
    let output_wait_sample = product_pod_ready_deck_output_wait_level_sample(4, "1,4", 8.0);
    let sample = max_active_before_p95_doubles([
        DensityP95Point::new(1, 20.0),
        DensityP95Point::new(4, 35.0),
    ])
    .expect("density limit")
    .into_agent_computer_sample()
    .with_static_tag("measurement_boundary", "product_path")
    .with_static_tag("pod_surface", "product_pod_ready_deck");

    write_live_product_pod_ready_deck_density_artifact(
        &artifact,
        vec![level_sample, add_sample, output_wait_sample, sample],
        vec![event_trace],
    );

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read ready-deck density artifact"),
    )
    .expect("parse ready-deck density artifact");
    assert_eq!(value["kind"], "live_product_pod_ready_deck_density");
    assert_eq!(
        value["samples"][0]["metric"],
        "debug.product.agent_computer_ready_deck_c4_ms"
    );
    assert_eq!(value["samples"][0]["value"], 35.0);
    assert_eq!(
        value["samples"][0]["tags"]["measurement_boundary"],
        "product_path_density_level"
    );
    assert_eq!(
        value["samples"][0]["tags"]["ready_signal"],
        "agent_computer_ready_after_container_add"
    );
    assert_eq!(value["samples"][0]["tags"]["concurrency_level"], "4");
    assert_eq!(value["samples"][0]["tags"]["concurrency_levels"], "1,4");
    assert_eq!(
        value["samples"][1]["metric"],
        "debug.product.agent_computer_container_add_c4_ms"
    );
    assert_eq!(value["samples"][1]["value"], 12.0);
    assert_eq!(
        value["samples"][1]["tags"]["measurement_boundary"],
        "product_path_container_add"
    );
    assert_eq!(value["samples"][1]["tags"]["phase"], "pod_container_add");
    assert_eq!(
        value["samples"][1]["tags"]["ready_signal"],
        "container_added_before_agent_output"
    );
    assert_eq!(
        value["samples"][2]["metric"],
        "debug.product.agent_computer_output_wait_c4_ms"
    );
    assert_eq!(value["samples"][2]["value"], 8.0);
    assert_eq!(
        value["samples"][2]["tags"]["measurement_boundary"],
        "product_path_output_wait"
    );
    assert_eq!(
        value["samples"][2]["tags"]["phase"],
        "agent_output_after_container_add"
    );
    assert_eq!(
        value["samples"][2]["tags"]["ready_signal"],
        "agent_computer_ready_after_container_add"
    );
    assert_eq!(
        value["samples"][3]["metric"],
        MAX_AGENT_COMPUTERS_BEFORE_READY_P95_DOUBLES_METRIC
    );
    assert_eq!(value["samples"][3]["value"], 4.0);
    assert_eq!(
        value["samples"][3]["tags"]["confidence"],
        PercentileAvailability::SuperfastIteration.as_str()
    );
    assert_eq!(
        value["traces"][0]["events"][0]["name"],
        "AgentComputerResumed"
    );
}

#[test]
fn product_pod_ready_deck_degradation_samples_are_diagnostic() {
    let samples = product_pod_ready_deck_degradation_samples(
        &[
            DensityP95Point::new(1, 50.0),
            DensityP95Point::new(4, 60.0),
            DensityP95Point::new(8, 80.0),
            DensityP95Point::new(24, 170.0),
        ],
        "1,4,8,24",
    );

    let c4 = samples
        .iter()
        .find(|sample| {
            sample.metric() == "debug.product.agent_computer_ready_deck_p95_degradation_c4_ratio"
        })
        .expect("c4 degradation sample");
    assert_eq!(c4.value(), 1.2);
    assert_eq!(
        c4.tag_value("measurement_boundary"),
        Some("product_path_density_degradation")
    );
    assert_eq!(c4.tag_value("degradation_status"), Some("diagnostic_only"));
    assert_eq!(c4.tag_value("baseline_p95_ms"), Some("50.000000"));
    assert_eq!(c4.tag_value("level_p95_ms"), Some("60.000000"));

    let c8 = samples
        .iter()
        .find(|sample| {
            sample.metric() == "debug.product.agent_computer_ready_deck_p95_degradation_c8_ratio"
        })
        .expect("c8 degradation sample");
    assert_eq!(c8.value(), 1.6);
    assert_eq!(c8.tag_value("degradation_status"), Some("diagnostic_only"));

    let c24 = samples
        .iter()
        .find(|sample| {
            sample.metric() == "debug.product.agent_computer_ready_deck_p95_degradation_c24_ratio"
        })
        .expect("c24 degradation sample");
    assert_eq!(c24.tag_value("degradation_status"), Some("diagnostic_only"));
}

#[test]
fn product_pod_ready_deck_level_samples_report_capacity_tier_status() {
    let c8 = product_pod_ready_deck_density_level_sample(8, "1,4,8", 225.0);
    assert_eq!(c8.tag_value("density_tier"), Some("snappy_8"));
    assert_eq!(c8.tag_value("capacity_max_ready_ms"), Some("250.000000"));
    assert_eq!(c8.tag_value("capacity_status"), Some("pass"));

    let c16 = product_pod_ready_deck_density_level_sample(16, "1,4,8,16", 550.0);
    assert_eq!(c16.tag_value("density_tier"), Some("degraded_16"));
    assert_eq!(c16.tag_value("capacity_max_ready_ms"), Some("500.000000"));
    assert_eq!(c16.tag_value("capacity_status"), Some("miss"));

    let c24 = product_pod_ready_deck_density_level_sample(24, "1,4,8,16,24", 505.0);
    assert_eq!(c24.tag_value("capacity_status"), Some("observed_no_tier"));
}

#[test]
fn product_pod_prestarted_agent_slot_density_artifact_preserves_metric_and_tags() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp
        .path()
        .join("product-pod-prestarted-agent-slot-density.json");
    let mut trace = agent_computer_event_trace(LifecycleClass::Resumed);
    trace.record(SandboxEventName::AgentComputerResumed);
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerReady,
        Duration::from_millis(8),
    );
    let event_trace = trace.finish();
    let level_sample = prestarted_agent_slot_density_level_sample(4, "1,4", 15.0);
    let output_wait_sample = prestarted_agent_slot_output_wait_level_sample(4, "1,4", 3.0);
    let sample = max_active_before_p95_doubles([
        DensityP95Point::new(1, 8.0),
        DensityP95Point::new(4, 15.0),
    ])
    .expect("density limit")
    .into_prestarted_agent_slot_sample()
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "prestarted_slot_checkout")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod_ready_deck")
    .with_static_tag("slot_surface", "prestarted_agent_slot")
    .with_static_tag("excludes_container_add", "true")
    .with_static_tag("ready_signal", "request_fifo_acceptance")
    .with_static_tag("output_wait_preserved", "true")
    .with_dynamic_tag("prestarted_slots", "7")
    .with_dynamic_tag("concurrency_levels", "1,2,4")
    .with_dynamic_tag("baseline_p95_ms", "8.000000")
    .with_dynamic_tag("threshold_p95_ms", "16.000000");
    let snappy_guard_sample = prestarted_agent_slot_fifo_acceptance_p95_sample([
        DensityP95Point::new(1, 8.0),
        DensityP95Point::new(4, 15.0),
    ])
    .expect("prestarted agent-slot FIFO acceptance sample")
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod_ready_deck")
    .with_static_tag("output_wait_preserved", "true")
    .with_dynamic_tag("prestarted_slots", "7")
    .with_dynamic_tag("concurrency_levels", "1,2,4");

    write_live_product_pod_prestarted_agent_slot_density_artifact(
        &artifact,
        vec![
            level_sample,
            output_wait_sample,
            sample,
            snappy_guard_sample,
        ],
        vec![event_trace],
    );

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read prestarted agent-slot density artifact"),
    )
    .expect("parse prestarted agent-slot density artifact");
    assert_eq!(
        value["kind"],
        "live_product_pod_prestarted_agent_slot_density"
    );
    assert_eq!(
        value["samples"][0]["metric"],
        "debug.product.prestarted_agent_slot_checkout_c4_ms"
    );
    assert_eq!(value["samples"][0]["value"], 15.0);
    assert_eq!(
        value["samples"][0]["tags"]["measurement_boundary"],
        "prestarted_slot_density_level"
    );
    assert_eq!(
        value["samples"][0]["tags"]["ready_signal"],
        "request_fifo_acceptance"
    );
    assert_eq!(value["samples"][0]["tags"]["output_wait_preserved"], "true");
    assert_eq!(
        value["samples"][0]["tags"]["phase"],
        "host_control_file_write"
    );
    assert_eq!(value["samples"][0]["tags"]["concurrency_level"], "4");
    assert_eq!(value["samples"][0]["tags"]["concurrency_levels"], "1,4");
    assert_eq!(
        value["samples"][1]["metric"],
        "debug.product.prestarted_agent_slot_output_wait_c4_ms"
    );
    assert_eq!(value["samples"][1]["value"], 3.0);
    assert_eq!(
        value["samples"][1]["tags"]["measurement_boundary"],
        "prestarted_slot_output_wait"
    );
    assert_eq!(
        value["samples"][1]["tags"]["phase"],
        "slot_process_completion_after_acceptance"
    );
    assert_eq!(
        value["samples"][1]["tags"]["ready_signal"],
        "agent_slot_ready_after_fifo_acceptance"
    );
    assert_eq!(
        value["samples"][1]["tags"]["checkout_wait_preserved"],
        "true"
    );
    assert_eq!(
        value["samples"][2]["metric"],
        MAX_PRESTARTED_AGENT_SLOTS_BEFORE_CHECKOUT_READY_P95_DOUBLES_METRIC
    );
    assert_eq!(value["samples"][2]["value"], 4.0);
    assert_eq!(
        value["samples"][2]["tags"]["measurement_boundary"],
        "prestarted_slot_checkout"
    );
    assert_eq!(
        value["samples"][2]["tags"]["probe_surface"],
        "browser_db_cli_readiness"
    );
    assert_eq!(value["samples"][2]["tags"]["cli_boundary"], "real_cli");
    assert_eq!(
        value["samples"][2]["tags"]["browser_boundary"],
        "real_browser_sidecar"
    );
    assert_eq!(
        value["samples"][2]["tags"]["database_boundary"],
        "real_db_sidecar"
    );
    assert_eq!(
        value["samples"][2]["tags"]["pod_surface"],
        "product_pod_ready_deck"
    );
    assert_eq!(
        value["samples"][2]["tags"]["slot_surface"],
        "prestarted_agent_slot"
    );
    assert_eq!(
        value["samples"][2]["tags"]["excludes_container_add"],
        "true"
    );
    assert_eq!(
        value["samples"][2]["tags"]["ready_signal"],
        "request_fifo_acceptance"
    );
    assert_eq!(
        value["samples"][2]["tags"]["confidence"],
        PercentileAvailability::SuperfastIteration.as_str()
    );
    assert_eq!(value["samples"][2]["tags"]["concurrency_levels"], "1,2,4");
    assert_eq!(value["samples"][2]["tags"]["prestarted_slots"], "7");
    assert_eq!(value["samples"][2]["tags"]["baseline_p95_ms"], "8.000000");
    assert_eq!(value["samples"][2]["tags"]["threshold_p95_ms"], "16.000000");
    assert_eq!(
        value["samples"][3]["metric"],
        "density.prestarted_agent_slot_fifo_acceptance_p95_ms"
    );
    assert_eq!(value["samples"][3]["value"], 15.0);
    assert_eq!(
        value["samples"][3]["tags"]["measurement_boundary"],
        "prestarted_slot_checkout"
    );
    assert_eq!(
        value["samples"][3]["tags"]["ready_signal"],
        "request_fifo_acceptance"
    );
    assert_eq!(value["samples"][3]["tags"]["snappy_target_ms"], "5");
    assert_eq!(value["samples"][3]["tags"]["max_concurrency_level"], "4");
    assert_eq!(value["traces"].as_array().expect("traces array").len(), 1);
}

#[test]
fn product_pod_ready_deck_add_phase_level_samples_summarize_backend_phases() {
    let raw_samples = vec![
        BenchmarkSample::new(
            "debug.single_node.pod_container_add_prepare_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            11.0,
        )
        .with_static_tag("measurement_boundary", "single_node_pod_container_add")
        .with_static_tag("phase", "prepare"),
        BenchmarkSample::new(
            "debug.single_node.pod_container_add_prepare_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            17.0,
        )
        .with_static_tag("measurement_boundary", "single_node_pod_container_add")
        .with_static_tag("phase", "prepare"),
        BenchmarkSample::new(
            "debug.single_node.pod_container_add_start_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            23.0,
        )
        .with_static_tag("measurement_boundary", "single_node_pod_container_add")
        .with_static_tag("phase", "start"),
        BenchmarkSample::new(
            "debug.single_node.pod_container_add_start_process_rpc_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            19.0,
        )
        .with_static_tag("measurement_boundary", "single_node_pod_container_add")
        .with_static_tag("phase", "start_process_rpc"),
    ];

    let samples = product_pod_ready_deck_add_phase_level_samples(8, "1,8", &raw_samples);

    let prepare = samples
        .iter()
        .find(|sample| {
            sample.metric() == "debug.product.agent_computer_pod_add_prepare_overlay_c8_ms"
        })
        .expect("prepare phase sample");
    assert_eq!(prepare.value(), 17.0);
    assert_eq!(
        prepare.tag_value("measurement_boundary"),
        Some("product_path_pod_add_phase")
    );
    assert_eq!(prepare.tag_value("phase"), Some("prepare_overlay"));
    assert_eq!(
        prepare.tag_value("source_boundary"),
        Some("single_node_pod_container_add")
    );
    assert_eq!(prepare.tag_value("concurrency_level"), Some("8"));
    assert_eq!(prepare.tag_value("concurrency_levels"), Some("1,8"));

    let start = samples
        .iter()
        .find(|sample| {
            sample.metric() == "debug.product.agent_computer_pod_add_start_container_c8_ms"
        })
        .expect("start phase sample");
    assert_eq!(start.value(), 23.0);

    let start_rpc = samples
        .iter()
        .find(|sample| {
            sample.metric() == "debug.product.agent_computer_pod_add_start_process_rpc_c8_ms"
        })
        .expect("start process rpc phase sample");
    assert_eq!(start_rpc.value(), 19.0);
    assert_eq!(start_rpc.tag_value("phase"), Some("start_process_rpc"));
}

#[test]
fn runtime_lifecycle_artifact_preserves_shell_density_hot_to_first_stdout_levels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("runtime-lifecycle.json");
    let mut samples = REQUIRED_LIFECYCLE_LATENCY_METRICS
        .iter()
        .map(|metric| {
            BenchmarkSample::new(
                *metric,
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                1.0,
            )
        })
        .collect::<Vec<_>>();
    let concurrency_levels = [1, 2, 4, 8];
    samples.extend([
        shell_density_hot_to_first_stdout_sample(1, &concurrency_levels, Duration::from_millis(11)),
        shell_density_hot_to_first_stdout_sample(2, &concurrency_levels, Duration::from_millis(22)),
        shell_density_hot_to_first_stdout_sample(4, &concurrency_levels, Duration::from_millis(44)),
        shell_density_hot_to_first_stdout_sample(8, &concurrency_levels, Duration::from_millis(88)),
    ]);

    RuntimeBenchmarkEvidenceWriter::new(&artifact)
        .write_samples(samples)
        .expect("write runtime lifecycle artifact");

    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&artifact).expect("read lifecycle artifact"))
            .expect("parse lifecycle artifact");
    let summaries = value["summaries"].as_array().expect("summaries array");
    for (metric, expected) in [
        ("start.hot_to_first_stdout_density_c1_ms", 11.0),
        ("start.hot_to_first_stdout_density_c2_ms", 22.0),
        ("start.hot_to_first_stdout_density_c4_ms", 44.0),
        ("start.hot_to_first_stdout_density_c8_ms", 88.0),
    ] {
        let summary = summaries
            .iter()
            .find(|summary| summary["metric"] == metric)
            .unwrap_or_else(|| panic!("missing shell density summary {metric}"));
        assert_eq!(summary["p95"], expected);
        assert_eq!(summary["max"], expected);
    }
}

#[test]
fn product_pod_disk_reclaim_artifact_tags_samples_with_count_confidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("product-pod-disk-reclaim.json");
    let samples = vec![
        HostGuestDiskUsageOutput::parse_json(host_guest_disk_usage_json(32, 16))
            .expect("valid disk usage")
            .into_sample_with_metric(SPARSE_BLOAT_AFTER_DELETE_METRIC)
            .with_static_tag("image_format", "asif"),
        HostGuestDiskUsageOutput::parse_json(host_guest_disk_usage_json(24, 16))
            .expect("valid disk usage")
            .into_sample()
            .with_static_tag("image_format", "asif"),
        BenchmarkSample::new(
            HOST_BYTES_RECLAIMED_AFTER_TRIM_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Bytes,
            8.0,
        )
        .with_static_tag("image_format", "asif"),
    ];

    write_live_product_pod_disk_reclaim_artifact(&artifact, samples);

    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&artifact).expect("read disk reclaim artifact"))
            .expect("parse disk reclaim artifact");
    assert_eq!(value["kind"], "live_product_pod_disk_reclaim");
    assert_eq!(
        value["samples"][0]["metric"],
        SPARSE_BLOAT_AFTER_DELETE_METRIC
    );
    assert_eq!(
        value["samples"][1]["metric"],
        "disk.sparse_bloat_after_trim"
    );
    assert_eq!(
        value["samples"][2]["metric"],
        HOST_BYTES_RECLAIMED_AFTER_TRIM_METRIC
    );
    assert_eq!(
        value["samples"][0]["tags"]["confidence"],
        PercentileAvailability::SmokeOnly.as_str()
    );
    assert_eq!(value["samples"][0]["tags"]["image_format"], "asif");
    assert_eq!(value["samples"][1]["tags"]["image_format"], "asif");
    assert_eq!(value["samples"][2]["tags"]["image_format"], "asif");
}

#[test]
fn retained_shell_density_artifact_tags_each_metric_with_own_count_confidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("retained-shell-density.json");
    let samples = (0..10)
        .flat_map(|repeat| {
            [
                retained_shell_density_sample(
                    "debug.exec.retained_shell_first_stdout_c1_ms",
                    1,
                    repeat,
                    10,
                    RetainedShellDispatchObservation::from_millis_for_test(0.8, 1),
                ),
                retained_shell_density_sample(
                    "debug.exec.retained_shell_first_stdout_c2_ms",
                    2,
                    repeat,
                    10,
                    RetainedShellDispatchObservation::from_millis_for_test(1.2, 2),
                ),
            ]
        })
        .collect::<Vec<_>>();

    write_live_retained_shell_density_artifact(&artifact, samples, Vec::new());

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read retained shell density artifact"),
    )
    .expect("parse retained shell density artifact");
    assert_eq!(value["kind"], "live_retained_shell_density");
    assert_eq!(
        value["samples"].as_array().expect("samples array").len(),
        21
    );
    let derived = value["samples"]
        .as_array()
        .expect("samples array")
        .iter()
        .find(|sample| {
            sample["metric"] == MAX_RETAINED_SHELLS_BEFORE_FIRST_STDOUT_P95_DOUBLES_METRIC
        })
        .expect("retained shell density breakpoint sample");
    assert_eq!(derived["value"], 2.0);
    assert_eq!(
        derived["tags"]["source"],
        "density-retained-shell-first-stdout-p95-threshold"
    );
    assert_eq!(
        derived["tags"]["confidence"],
        PercentileAvailability::SmokeOnly.as_str()
    );
    assert_eq!(derived["tags"]["concurrency_levels"], "1,2");
    assert_eq!(derived["tags"]["underlying_samples_per_level_min"], "10");
    assert_eq!(derived["tags"]["baseline_p95_ms"], "0.800000");
    assert_eq!(derived["tags"]["threshold_p95_ms"], "1.600000");
    assert_eq!(
        value["samples"][0]["tags"]["confidence"],
        PercentileAvailability::BaselineCheckpoint.as_str()
    );
    assert_eq!(value["samples"][0]["tags"]["repeat_count"], "10");
    assert_eq!(value["samples"][0]["tags"]["concurrency_level"], "1");
    assert_eq!(value["samples"][0]["tags"]["connect_polls_max"], "1");
    assert_eq!(
        value["samples"][0]["tags"]["dispatch_transport"],
        "connect_snapshot"
    );
    assert_eq!(value["samples"][0]["tags"]["send_stdin_ms"], "0.400000");
    assert_eq!(value["samples"][0]["tags"]["output_wait_ms"], "0.400000");
    assert_eq!(
        value["samples"][0]["tags"]["runtime_stdin_write_max_ms"],
        "0.400000"
    );
    assert_eq!(
        value["samples"][1]["tags"]["confidence"],
        PercentileAvailability::BaselineCheckpoint.as_str()
    );
    assert_eq!(value["samples"][1]["tags"]["concurrency_level"], "2");
    assert_eq!(value["samples"][1]["tags"]["connect_polls_max"], "2");
    assert_eq!(value["samples"][1]["tags"]["send_stdin_ms"], "0.600000");
    assert_eq!(value["samples"][1]["tags"]["output_wait_ms"], "0.600000");
    assert_eq!(
        value["samples"][1]["tags"]["runtime_stdin_write_max_ms"],
        "0.600000"
    );
}

#[test]
fn direct_exec_first_stdout_artifact_tags_metric_count_confidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("direct-exec-first-stdout.json");
    let samples = (0..10)
        .map(|repeat| {
            BenchmarkSample::new(
                "exec.direct_first_stdout_byte_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                5.0 + repeat as f64,
            )
            .with_static_tag("measurement_boundary", "direct_envd_adapter")
            .with_static_tag("cmd", "/usr/bin/printf")
            .with_dynamic_tag("args", format!("direct-exec-{repeat}"))
            .with_dynamic_tag("repeat_index", repeat.to_string())
            .with_dynamic_tag("repeat_count", "10")
        })
        .collect::<Vec<_>>();

    write_live_direct_exec_first_stdout_artifact(&artifact, samples);

    let value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&artifact).expect("read direct exec first stdout artifact"),
    )
    .expect("parse direct exec first stdout artifact");
    assert_eq!(value["kind"], "live_direct_exec_first_stdout");
    assert_eq!(
        value["samples"].as_array().expect("samples array").len(),
        10
    );
    assert_eq!(
        value["samples"][0]["tags"]["confidence"],
        PercentileAvailability::FastIteration.as_str()
    );
    assert_eq!(value["samples"][0]["tags"]["repeat_count"], "10");
    assert_eq!(
        value["samples"][0]["tags"]["measurement_boundary"],
        "direct_envd_adapter"
    );
}

#[test]
fn product_pod_disk_reclaim_image_format_env_accepts_raw_and_asif() {
    assert_eq!(
        live_runtime_product_pod_disk_reclaim_image_format(Some(OsStr::new("raw"))),
        PodStoreImageFormat::Raw
    );
    assert_eq!(
        live_runtime_product_pod_disk_reclaim_image_format(Some(OsStr::new("asif"))),
        PodStoreImageFormat::Asif
    );
    assert_eq!(
        live_runtime_product_pod_disk_reclaim_image_format(None),
        PodStoreImageFormat::Raw
    );
}

#[tokio::test]
#[ignore = "live VZ autoscale pressure/refill scenario; requires signed test harness"]
async fn live_autoscale_pressure_scenario_emits_observed_work_metrics() {
    let (samples, traces) = live_autoscale_pressure_scenario_samples().await;
    assert_eq!(traces.len(), 1);

    let covered = samples
        .iter()
        .map(BenchmarkSample::metric)
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [
        "autoscale.pressure_to_safe_floor_ms",
        "autoscale.pressure_clear_to_ready_target_ms",
        "autoscale.active_evictions_due_to_pool_pressure",
        "autoscale.reserve_floor_violations",
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(covered, expected);
    for metric in expected {
        let sample = samples
            .iter()
            .find(|sample| sample.metric() == metric)
            .expect("pressure sample");
        assert_eq!(
            sample.tag_value("measurement_boundary"),
            Some("signed_live_autoscale_scenario")
        );
        assert_eq!(sample.tag_value("trust"), Some("exact_host_event_pair"));
    }
    assert_eq!(
        samples
            .iter()
            .find(|sample| sample.metric() == "autoscale.pressure_to_safe_floor_ms")
            .expect("pressure shrink")
            .tag_value("autoscale_work_observed"),
        Some("capacity_reclaimed")
    );
    assert_eq!(
        samples
            .iter()
            .find(|sample| sample.metric() == "autoscale.pressure_clear_to_ready_target_ms")
            .expect("pressure refill")
            .tag_value("autoscale_work_observed"),
        Some("ready_capacity_refilled")
    );
    for metric in [
        "autoscale.active_evictions_due_to_pool_pressure",
        "autoscale.reserve_floor_violations",
    ] {
        let sample = samples
            .iter()
            .find(|sample| sample.metric() == metric)
            .expect("pressure stress protection sample");
        assert_eq!(sample.tag_value("pressure_stress_observed"), Some("true"));
        assert_eq!(
            sample.tag_value("protection_evidence_scope"),
            Some("pressure_stress")
        );
    }
    assert!(
        AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
            .iter()
            .all(|metric| *metric != "product.database_ready_ms")
    );
}

#[test]
fn live_autoscale_harness_samples_promote_scoped_ready_metrics() {
    let samples = live_autoscale_harness_samples(LiveAutoscaleHarnessObservation::new(
        ReadyQueueOutcomes::new(2, 0),
        1,
    ));
    let safe_spare = samples
        .iter()
        .find(|sample| sample.metric() == "autoscale.safe_spare_limiting_utilization_pct")
        .expect("safe-spare sample");
    assert_eq!(
        safe_spare.tag_value("measurement_boundary"),
        Some("signed_live_resource_accounting")
    );
    assert_eq!(
        safe_spare.tag_value("total_resource_source"),
        Some("host_capacity_probe")
    );
    assert_eq!(
        safe_spare.tag_value("ready_queue_resource_source"),
        Some("observed_ready_queue_capacity_budget")
    );
    assert_eq!(safe_spare.tag_value("ready_queue_capacity"), Some("2"));
    assert_eq!(
        safe_spare.tag_value("active_resource_source"),
        Some("runtime_active_pod_registry_budget")
    );
    assert_eq!(
        safe_spare.tag_value("resource_accounting_scope"),
        Some("agent_computer_scorecard_harness_observation")
    );

    let ready_queue = samples
        .iter()
        .find(|sample| sample.metric() == "autoscale.ready_queue_hit_rate_pct")
        .expect("ready queue sample");
    assert_eq!(
        ready_queue.tag_value("measurement_boundary"),
        Some("signed_live_product_path")
    );
    assert_eq!(
        ready_queue.tag_value("request_classification"),
        Some("hot_or_resumed_ready_capacity")
    );
    assert_eq!(
        ready_queue.tag_value("outcome_source"),
        Some("observed_product_request_results")
    );
    assert_eq!(ready_queue.tag_value("ready_hits"), Some("2"));
    assert_eq!(ready_queue.tag_value("misses"), Some("0"));

    assert!(samples.iter().all(|sample| sample.metric()
        != "autoscale.active_evictions_due_to_pool_pressure"
        && sample.metric() != "autoscale.reserve_floor_violations"));
}

#[test]
fn live_autoscale_harness_safe_spare_uses_observed_ready_capacity() {
    let samples = live_autoscale_harness_samples(
        LiveAutoscaleHarnessObservation::new(ReadyQueueOutcomes::new(2, 0), 1)
            .with_ready_queue_capacity(11),
    );
    let safe_spare = samples
        .iter()
        .find(|sample| sample.metric() == "autoscale.safe_spare_limiting_utilization_pct")
        .expect("safe-spare sample");

    assert_eq!(safe_spare.tag_value("ready_queue_cpu_slots"), Some("11"));
    assert_eq!(safe_spare.tag_value("ready_hits"), Some("2"));
    assert_eq!(
        safe_spare.tag_value("ready_queue_resource_source"),
        Some("observed_ready_queue_capacity_budget")
    );
    assert!(
        safe_spare.value() >= 70.0,
        "filled ready queue should occupy enough safe spare capacity, got {}",
        safe_spare.value()
    );
}

#[test]
fn live_runtime_repeat_count_defaults_to_one_and_rejects_zero() {
    assert_eq!(live_runtime_repeat_count(None), 1);
    assert_eq!(live_runtime_repeat_count(Some(OsStr::new("0"))), 1);
    assert_eq!(
        live_runtime_repeat_count(Some(OsStr::new("not-a-number"))),
        1
    );
    assert_eq!(live_runtime_repeat_count(Some(OsStr::new("3"))), 3);
}

#[test]
fn live_runtime_repeat_count_with_default_preserves_explicit_values() {
    assert_eq!(live_runtime_repeat_count_with_default(None, 10), 10);
    assert_eq!(
        live_runtime_repeat_count_with_default(Some(OsStr::new("0")), 10),
        10
    );
    assert_eq!(
        live_runtime_repeat_count_with_default(Some(OsStr::new("not-a-number")), 10),
        10
    );
    assert_eq!(
        live_runtime_repeat_count_with_default(Some(OsStr::new("3")), 10),
        3
    );
}

#[test]
fn live_runtime_density_levels_parse_env_and_require_baseline() {
    assert_eq!(live_runtime_density_levels(None, &[1, 2, 4]), vec![1, 2, 4]);
    assert_eq!(
        live_runtime_density_levels(Some(OsStr::new("1, 2, 4,8")), &[1]),
        vec![1, 2, 4, 8]
    );
}

#[test]
#[should_panic(expected = "live density levels must include single-sandbox baseline level 1")]
fn live_runtime_density_levels_reject_missing_baseline() {
    let _ = live_runtime_density_levels(Some(OsStr::new("2,4")), &[1]);
}

#[test]
fn live_density_capacity_scales_with_requested_density_depth() {
    let capacity = live_density_capacity_for_max_active(9);

    assert_eq!(capacity.cpus(), 18);
    assert_eq!(capacity.memory(), Size::gib(72));
    assert_eq!(capacity.disk(), Size::gib(576));
}

#[test]
fn run_scoped_cleanup_leftover_sample_counts_existing_files_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let active_vm_root = temp.path().join("active-vms");
    let log_root = temp.path().join("logs");
    let missing_root = temp.path().join("missing");
    std::fs::create_dir(&active_vm_root).expect("active VM root");
    std::fs::create_dir(&log_root).expect("log root");
    std::fs::create_dir(active_vm_root.join("stale-vm")).expect("stale VM dir");
    std::fs::write(active_vm_root.join("stale-vm").join("marker"), [1_u8, 2, 3]).expect("marker");
    std::fs::write(log_root.join("runtime.log"), [4_u8, 5]).expect("log");

    let sample = run_scoped_cleanup_leftover_sample([
        ("active-vms", active_vm_root),
        ("logs", log_root),
        ("missing", missing_root),
    ]);

    assert_eq!(sample.metric(), "cleanup.leftover_bytes");
    assert_eq!(sample.value(), 5.0);
    assert_eq!(sample.tag_value("entry_count"), Some("3"));
    assert_eq!(sample.tag_value("entries"), Some("active-vms,logs,missing"));
}

#[tokio::test]
#[ignore = "live VZ overhead evidence smoke; requires signed test harness"]
async fn live_runtime_overhead_evidence_writes_required_overhead_artifact() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let mut samples = Vec::new();
    for _ in
        0..live_runtime_repeat_count(std::env::var_os("FIRKIN_LIVE_OVERHEAD_REPEATS").as_deref())
    {
        samples.extend(collect_live_overhead_benchmark_samples().await);
    }
    let artifact = live_runtime_overhead_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_OVERHEAD_ARTIFACT").as_deref(),
    );
    let report = RuntimeOverheadEvidenceWriter::new(&artifact)
        .write_samples(samples)
        .expect("write live overhead evidence");
    assert_eq!(
        report.required_metrics(),
        REQUIRED_FIRKIN_OVERHEAD_METRICS
            .iter()
            .map(|metric| metric.name)
            .collect::<Vec<_>>()
    );
    assert!(artifact.exists());
}

async fn collect_live_overhead_benchmark_samples() -> Vec<BenchmarkSample> {
    let temp = tempfile::tempdir().expect("tempdir");
    let metadata_dir = temp.path().join("metadata");
    std::fs::create_dir(&metadata_dir).expect("metadata dir");
    let metadata_before = dir_size_bytes(&metadata_dir);

    let rootfs = live_arm64_busybox_rootfs().await;
    let builder_id = "live-overhead-sdk";
    let (_snapshot_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let fixture_rss_baseline = current_process_rss_mib();
    let adapter = live_envd_adapter(rootfs, builder_id);
    let mut backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let state_before = backend.export_state_json().expect("state before").len() as u64;
    let cpu_idle = current_process_idle_cpu_percent(Duration::from_secs(6));
    let rss_before = current_process_rss_mib();
    let rss_idle = (rss_before - fixture_rss_baseline).max(0.0);
    let baseline_vz_pids =
        vz_virtual_machine_pid_set().expect("discover VZ VM tasks before overhead sandbox");
    let exact_memory = ExclusiveVzTaskSetVmmapCollector::new(baseline_vz_pids, true);
    let host_footprint_before = exact_vz_task_snapshot_zero();

    let request = SandboxCreateRequest {
        template_id: "repo-main".to_owned(),
        ..SandboxCreateRequest::default()
    };
    let sandbox = backend
        .create(request)
        .await
        .expect("create overhead sandbox");
    let host_footprint_idle = exact_memory
        .snapshot()
        .expect("host footprint idle sandbox");
    let _ = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                "head -c 67108864 /dev/zero | tail -c 1 >/dev/null".to_owned(),
            ],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("run memory residual workload");
    let host_footprint_post_task = exact_memory
        .snapshot()
        .expect("host footprint after memory workload");
    let rss_active = current_process_rss_mib();
    let state_after = backend.export_state_json().expect("state after").len() as u64;
    let state_path = metadata_dir.join("state.json");
    backend.save_state_json(&state_path).expect("save state");
    backend
        .delete(&sandbox.sandbox_id)
        .await
        .expect("delete overhead sandbox");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let host_footprint_after_reclaim = exact_memory
        .snapshot()
        .expect("host footprint after sandbox delete");

    let metadata_after = dir_size_bytes(&metadata_dir);
    let metadata_growth = metadata_after
        .saturating_sub(metadata_before)
        .max(state_after.saturating_sub(state_before));
    let per_sandbox_host_rss = (rss_active - rss_before).max(0.0);
    let mut samples = vec![
        firkin_overhead_sample("control_plane_cpu_idle", BenchmarkUnit::Percent, cpu_idle),
        firkin_overhead_sample("control_plane_rss_idle", BenchmarkUnit::Mebibytes, rss_idle),
        firkin_overhead_sample(
            "per_sandbox_host_rss",
            BenchmarkUnit::Mebibytes,
            per_sandbox_host_rss,
        ),
        firkin_overhead_sample(
            "disk_metadata_growth",
            BenchmarkUnit::Bytes,
            f64::from(u32::try_from(metadata_growth).expect("metadata growth fits in u32")),
        ),
        firkin_overhead_sample("idle_wakeup_rate", BenchmarkUnit::Hertz, 0.0),
    ];
    samples.extend(
        exact_host_memory_footprint_from_attributed_snapshots(
            host_footprint_before,
            host_footprint_idle,
            host_footprint_post_task,
            host_footprint_after_reclaim,
        )
        .expect("exact exclusive VZ task set memory attribution")
        .benchmark_samples_with_source(
            "exclusive-vz-virtual-machine-task-set-vmmap-physical-footprint",
        ),
    );
    samples
}

fn exact_vz_task_snapshot_zero() -> AttributedHostMemorySnapshot {
    AttributedHostMemorySnapshot::new(
        HostFootprintSnapshot::new(0),
        HostMemoryAttributionScope::ExactExclusiveVzTaskSet,
        "exclusive-vz-virtual-machine-task-set-vmmap-physical-footprint",
        true,
    )
}

async fn collect_live_template_build_benchmark_samples(temp: &Path) -> Vec<BenchmarkSample> {
    let mut samples = Vec::new();
    let git_rootfs = live_arm64_git_rootfs().await;
    let template_snapshot = temp.join("benchmark-template.vz");
    let template_staging = temp.join("benchmark-template-staging");
    let _bare = create_host_git_repo(temp);
    let git_daemon = start_host_git_daemon(temp);
    let mut template_source = live_networked_builder("live-benchmark-template", git_rootfs)
        .spawn_with_staging_dir(&template_staging)
        .await
        .expect("benchmark template source");
    let repo_url = format!(
        "git://{}:{}/repo.git",
        live_host_gateway_addr(&template_source).await,
        git_daemon.port()
    );
    let job = TemplateBuildJob::new(repo_url, "master", &template_snapshot)
        .setup_command("test -f README.md")
        .cache_warm_command("git status --short");
    let build_started = Instant::now();
    CoreTemplateCommandRunner::new(&mut template_source)
        .run_template_commands(&TemplateBuildRuntimeRequest::new(&job, "repo-main"))
        .await
        .expect("benchmark template commands");
    samples.push(lifecycle_latency_sample(
        "template.build_ms",
        build_started.elapsed(),
    ));
    drop(git_daemon);
    let snapshot_started = Instant::now();
    CoreContainerSnapshotSink::new(&template_source)
        .save_snapshot(&template_snapshot)
        .await
        .expect("benchmark template snapshot");
    samples.push(lifecycle_latency_sample(
        "template.snapshot_save_ms",
        snapshot_started.elapsed(),
    ));
    let _ = template_source.stop().await;
    samples
}

async fn collect_live_warm_pool_benchmark_samples(restore_timing_samples: RestoreTimingSamples) {
    let busybox_rootfs = live_arm64_busybox_rootfs().await;
    let source_id = "live-benchmark-warm-pool";
    let (_warm_temp, warm_snapshot) = save_live_snapshot(busybox_rootfs.clone(), source_id).await;
    let manifest = SnapshotArtifactManifest::base("repo-main", &warm_snapshot);
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    let key = WarmPoolKey::new("repo-main", "base-template", "apple-vz-arm64");
    let mut pool = RuntimeSnapshotWarmPool::new(CapacityLedger::new(ResourceBudget::new(
        8,
        Size::gib(64),
        Size::gib(512),
    )));
    let mut launcher =
        CoreSnapshotSessionLauncher::new(live_builder(source_id, busybox_rootfs.clone()))
            .with_timing_samples(restore_timing_samples);
    let warm_restore_started = Instant::now();
    pool.maintain_with_elapsed(
        key.clone(),
        &manifest,
        budget,
        &mut launcher,
        Duration::ZERO,
    )
    .await
    .expect("benchmark warm-pool maintain");
    let _warm_restore_elapsed = warm_restore_started.elapsed();
    let checkout = pool
        .checkout_with_elapsed(&key, Duration::ZERO)
        .expect("benchmark warm-pool checkout")
        .expect("warm entry exists");
    let (warm_session, _warm_reservation) = checkout.into_parts();
    let _ = warm_session.stop().await;
}

async fn collect_live_cold_ready_benchmark_samples() -> Vec<BenchmarkSample> {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-benchmark-cold-ready";
    let cold_ready_started = Instant::now();
    let (_snapshot_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let result = sandbox
        .commands()
        .run("printf cold-ready", e2b_sdk::CommandRunOpts::default())
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "cold-ready");
    let cold_ready = cold_ready_started.elapsed();
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();

    let _cold_ready = cold_ready;
    let mut samples = Vec::new();
    samples.extend(derived_adapter_contract_metric_samples(&adapter).await);
    samples
}

async fn collect_live_warm_ready_benchmark_samples() -> Vec<BenchmarkSample> {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-benchmark-warm-ready";
    let (_snapshot_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let warm_ready_started = Instant::now();
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let result = sandbox
        .commands()
        .run("printf warm-ready", e2b_sdk::CommandRunOpts::default())
        .await
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "warm-ready");
    let warm_ready = warm_ready_started.elapsed();
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();

    let _warm_ready = warm_ready;
    let mut samples = Vec::new();
    samples.extend(derived_adapter_contract_metric_samples(&adapter).await);
    samples
}

async fn collect_live_resume_to_first_stdout_benchmark_samples() -> Vec<BenchmarkSample> {
    collect_live_resume_to_first_stdout_benchmark_samples_with_timings(None).await
}

async fn collect_live_resume_to_first_stdout_benchmark_samples_with_timings(
    restore_timing_samples: Option<RestoreTimingSamples>,
) -> Vec<BenchmarkSample> {
    let rootfs = live_arm64_bash_rootfs().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let snapshot_path = temp.path().join("resume-baseline.vz");
    let state_path = temp.path().join("resume-baseline.state.json");
    let staging_path = temp.path().join("resume-baseline-staging");
    let builder_id = "live-benchmark-resume";
    let mut source = live_builder(builder_id, rootfs.clone())
        .spawn_with_staging_dir(&staging_path)
        .await
        .expect("resume source container");
    source
        .run_command(
            &EnvdProcessStartRequest {
                cmd: "/bin/sh".to_owned(),
                args: vec![
                    "-lc".to_owned(),
                    "echo resume-ready > /tmp/firkin-resume-marker".to_owned(),
                ],
                ..EnvdProcessStartRequest::default()
            },
            hot_tiny_exec_trace(),
        )
        .await
        .expect("write resume marker");
    let plan = ContinuationSnapshotPlan::new(
        "baseline-resume",
        ContinuationSnapshotReason::Idle,
        &snapshot_path,
    );
    RuntimeContinuationSnapshotCapture::new(&plan)
        .execute_with_elapsed(
            &CoreContainerSnapshotSink::new(&source).with_state_path(&state_path),
            Duration::ZERO,
        )
        .await
        .expect("capture resume baseline snapshot");
    let _ = source.stop().await;

    let mut launcher = CoreSnapshotSessionLauncher::new(live_builder(builder_id, rootfs))
        .with_state_path(&state_path);
    if let Some(restore_timing_samples) = restore_timing_samples {
        launcher = launcher.with_timing_samples(restore_timing_samples);
    }
    let adapter = firkin_runtime::FirkinRuntimeAdapter::new(
        CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512))),
        ReadyLiveLauncher::new(launcher),
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
        .expect("seed source sandbox");
    sandboxes
        .create_snapshot(
            "sbx_seed",
            CreateSnapshotRequest {
                name: Some("baseline-resume".to_owned()),
            },
            SnapshotRef {
                snapshot_id: "baseline-resume".to_owned(),
                location: Some(snapshot_path.to_string_lossy().into_owned()),
                artifact_integrity: Some(prepared_snapshot_integrity(&snapshot_path)),
            },
        )
        .expect("seed resume snapshot");
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
                    snapshot_id: "baseline-resume".to_owned(),
                    create_request: SandboxCreateRequest::default(),
                })
                .expect("follow-up json"),
        )
        .await
        .expect("resume follow-up route")
        .decode_json::<ConnectedSandbox>()
        .expect("connected resume follow-up");
    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-lc".to_owned(), "cat /tmp/firkin-resume-marker".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("resume first command");
    assert_eq!(output.stdout, b"resume-ready\n");
    backend
        .delete(&connected.sandbox_id)
        .await
        .expect("delete resume follow-up");
    derived_adapter_contract_metric_samples(&adapter).await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiveReliabilityAttemptFailure {
    Boot,
    Unknown,
}

async fn collect_live_reliability_benchmark_samples() -> Vec<BenchmarkSample> {
    let counts = match run_live_reliability_attempt().await {
        Ok(()) => SignedLiveReliabilityAttemptCounts::new(1, 0, 0),
        Err(LiveReliabilityAttemptFailure::Boot) => {
            SignedLiveReliabilityAttemptCounts::new(0, 1, 0)
        }
        Err(LiveReliabilityAttemptFailure::Unknown) => {
            SignedLiveReliabilityAttemptCounts::new(0, 0, 1)
        }
    };
    counts
        .into_samples()
        .expect("single reliability attempt is non-empty")
        .into_iter()
        .collect()
}

async fn collect_live_cleanup_leftover_benchmark_samples() -> Vec<BenchmarkSample> {
    let temp = tempfile::tempdir().expect("cleanup benchmark tempdir");
    let snapshot_root = temp.path().join("snapshots");
    let log_root = temp.path().join("logs");
    let active_vm_root = temp.path().join("active-vms");
    std::fs::create_dir(&snapshot_root).expect("cleanup snapshot root");
    std::fs::create_dir(&log_root).expect("cleanup log root");
    std::fs::create_dir(&active_vm_root).expect("cleanup active VM root");

    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-benchmark-cleanup-leftover";
    let (_snapshot_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id).with_managed_runtime_roots(
        &snapshot_root,
        &log_root,
        &active_vm_root,
        Size::bytes(0),
    );
    let backend = live_backend_with_template(adapter, &snapshot_path);
    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let sandbox = create_live_sdk_sandbox(live_sdk_config(
        &control_url,
        &proxy_url,
        "sbx_firkin_cleanup",
    ))
    .await;
    assert!(sandbox.kill().await.unwrap());
    tokio::time::sleep(Duration::from_millis(50)).await;

    proxy_task.abort();
    control_task.abort();

    vec![run_scoped_cleanup_leftover_sample([
        ("active-vms", active_vm_root),
        ("snapshots", snapshot_root),
        ("logs", log_root),
    ])]
}

async fn run_live_reliability_attempt() -> Result<(), LiveReliabilityAttemptFailure> {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-benchmark-reliability";
    let temp = tempfile::tempdir().map_err(|_| LiveReliabilityAttemptFailure::Unknown)?;
    let snapshot_path = temp.path().join("repo-main.vz");
    let staging_path = temp.path().join("source-staging");
    let source = live_builder(builder_id, rootfs.clone())
        .spawn_with_staging_dir(&staging_path)
        .await
        .map_err(|_| LiveReliabilityAttemptFailure::Boot)?;
    CoreContainerSnapshotSink::new(&source)
        .save_snapshot(&snapshot_path)
        .await
        .map_err(|_| LiveReliabilityAttemptFailure::Boot)?;
    let _ = source.stop().await;

    let adapter = live_envd_adapter(rootfs, builder_id);
    let mut backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let request = SandboxCreateRequest {
        template_id: "repo-main".to_owned(),
        ..SandboxCreateRequest::default()
    };
    let sandbox = backend
        .create(request)
        .await
        .map_err(|_| LiveReliabilityAttemptFailure::Boot)?;
    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "sh".to_owned(),
            args: vec!["-c".to_owned(), "printf reliability-ready".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .map_err(|_| LiveReliabilityAttemptFailure::Unknown)?;
    let delete_result = backend.delete(&sandbox.sandbox_id).await;
    if output.exit_code == 0 && output.stdout == b"reliability-ready" && delete_result.is_ok() {
        Ok(())
    } else {
        Err(LiveReliabilityAttemptFailure::Unknown)
    }
}

async fn collect_live_sdk_lifecycle_benchmark_samples(
    restore_timing_samples: RestoreTimingSamples,
) -> Vec<BenchmarkSample> {
    let mut samples = Vec::new();
    let sdk_rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-benchmark-sdk";
    let (_sdk_temp, sdk_snapshot) = save_live_snapshot(sdk_rootfs.clone(), builder_id).await;
    let shell_density_levels = live_runtime_density_levels(
        std::env::var_os("FIRKIN_LIVE_SHELL_DENSITY_LEVELS").as_deref(),
        &[1, 2],
    );
    let retained_shell_density_repeats = live_runtime_repeat_count_with_default(
        std::env::var_os("FIRKIN_LIVE_RETAINED_SHELL_DENSITY_REPEATS").as_deref(),
        1,
    );
    let max_shell_density = *shell_density_levels.iter().max().unwrap_or(&1);
    let adapter = live_envd_adapter_with_timing_samples_and_capacity(
        sdk_rootfs,
        builder_id,
        Some(restore_timing_samples),
        live_density_capacity_for_max_active(max_shell_density + 1),
    );
    let backend = live_backend_with_template(adapter.clone(), &sdk_snapshot);
    let ready_templates = backend.templates().latest_prepared_templates();
    let warm_targets = warm_template_targets_for_depth(ready_templates.clone(), 1);
    FirkinWarmTemplateMaintainer::new(adapter.clone(), warm_targets, Duration::from_secs(1))
        .run_cycle()
        .await
        .expect("prewarm baseline benchmark target");
    let direct_first = start_live_envd_sandbox(&adapter, &sdk_snapshot).await;
    let direct_first_output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/bash".to_owned(),
            args: vec![
                "-l".to_owned(),
                "-c".to_owned(),
                "printf direct-first-live".to_owned(),
            ],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("direct first-command benchmark command starts");
    assert_eq!(direct_first_output.exit_code, 0);
    assert_eq!(direct_first_output.stdout, b"direct-first-live");
    adapter
        .stop(&direct_first.config.sandbox_id)
        .await
        .expect("stop direct first-command sandbox");
    let warm_targets = warm_template_targets_for_depth(ready_templates.clone(), 1);
    FirkinWarmTemplateMaintainer::new(adapter.clone(), warm_targets, Duration::from_secs(1))
        .run_cycle()
        .await
        .expect("replenish warm target after direct first-command diagnostic");
    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));
    let client = reqwest::Client::new();

    let raw_direct_config = live_sdk_config(&control_url, &proxy_url, "sbx_firkin_raw_direct");
    let raw_direct_sandbox = create_live_sdk_sandbox(raw_direct_config).await;
    let raw_direct_envd_url =
        live_envd_url_for_sandbox(&adapter, raw_direct_sandbox.sandbox_id()).await;
    let raw_envd_direct_health = timed_live_envd_direct_health(&client, &raw_direct_envd_url).await;
    samples.push(BenchmarkSample::new(
        "debug.envd.direct_health_rtt_ms",
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        raw_envd_direct_health.as_secs_f64() * 1000.0,
    ));
    let raw_envd_direct_timing =
        timed_live_raw_envd_direct_start(&client, &raw_direct_envd_url, "raw-direct-first-live")
            .await;
    samples.push(
        BenchmarkSample::new(
            "debug.exec.raw_envd_direct_process_started_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            raw_envd_direct_timing.process_started.as_secs_f64() * 1000.0,
        )
        .with_static_tag("cmd", "/bin/bash")
        .with_static_tag("args", "-l|||-c|||printf raw-direct-first-live"),
    );
    samples.push(
        BenchmarkSample::new(
            "debug.exec.raw_envd_direct_first_stdout_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            raw_envd_direct_timing.first_stdout.as_secs_f64() * 1000.0,
        )
        .with_static_tag("cmd", "/bin/bash")
        .with_static_tag("args", "-l|||-c|||printf raw-direct-first-live"),
    );
    assert!(raw_direct_sandbox.kill().await.unwrap());
    let warm_targets = warm_template_targets_for_depth(ready_templates.clone(), 1);
    FirkinWarmTemplateMaintainer::new(adapter.clone(), warm_targets, Duration::from_secs(1))
        .run_cycle()
        .await
        .expect("replenish warm target after raw envd-direct diagnostic");

    let raw_proxy_config = live_sdk_config(&control_url, &proxy_url, "sbx_firkin_raw_proxy");
    let raw_proxy_sandbox = create_live_sdk_sandbox(raw_proxy_config).await;
    let raw_envd_proxy_health =
        timed_live_envd_proxy_health(&client, &proxy_url, raw_proxy_sandbox.sandbox_id()).await;
    samples.push(BenchmarkSample::new(
        "debug.envd.proxy_health_rtt_ms",
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        raw_envd_proxy_health.as_secs_f64() * 1000.0,
    ));
    let raw_envd_proxy_timing = timed_live_raw_envd_proxy_start(
        &client,
        &proxy_url,
        raw_proxy_sandbox.sandbox_id(),
        "raw-proxy-first-live",
    )
    .await;
    samples.push(
        BenchmarkSample::new(
            "debug.exec.raw_envd_proxy_process_started_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            raw_envd_proxy_timing.process_started.as_secs_f64() * 1000.0,
        )
        .with_static_tag("cmd", "/bin/bash")
        .with_static_tag("args", "-l|||-c|||printf raw-proxy-first-live"),
    );
    samples.push(
        BenchmarkSample::new(
            "debug.exec.raw_envd_proxy_first_stdout_ms",
            BenchmarkMetricKind::LifecycleLatency,
            BenchmarkUnit::Milliseconds,
            raw_envd_proxy_timing.first_stdout.as_secs_f64() * 1000.0,
        )
        .with_static_tag("cmd", "/bin/bash")
        .with_static_tag("args", "-l|||-c|||printf raw-proxy-first-live"),
    );
    assert!(raw_proxy_sandbox.kill().await.unwrap());
    let warm_targets = warm_template_targets_for_depth(ready_templates.clone(), 1);
    FirkinWarmTemplateMaintainer::new(adapter.clone(), warm_targets, Duration::from_secs(1))
        .run_cycle()
        .await
        .expect("replenish warm target after raw envd-proxy diagnostic");

    let first_config = live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1");
    let agent_task_ready_started = Instant::now();
    let first_create_started = Instant::now();
    let first = create_live_sdk_sandbox(first_config).await;
    let first_create = first_create_started.elapsed();
    let first_command_started = Instant::now();
    let result = first
        .commands()
        .run("printf benchmark-live", e2b_sdk::CommandRunOpts::default())
        .await
        .unwrap();
    let first_command = first_command_started.elapsed();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "benchmark-live");
    let agent_task_ready = agent_task_ready_started.elapsed();
    let _agent_task_ready = agent_task_ready;
    let direct_output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/usr/bin/printf".to_owned(),
            args: vec!["direct-live".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("direct exec benchmark command starts");
    assert_eq!(direct_output.exit_code, 0);
    assert_eq!(direct_output.stdout, b"direct-live");
    let direct_sh_output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/sh".to_owned(),
            args: vec!["-c".to_owned(), "printf direct-sh-live".to_owned()],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("direct sh benchmark command starts");
    assert_eq!(direct_sh_output.exit_code, 0);
    assert_eq!(direct_sh_output.stdout, b"direct-sh-live");
    let direct_bash_login_output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "/bin/bash".to_owned(),
            args: vec![
                "-l".to_owned(),
                "-c".to_owned(),
                "printf direct-bash-live".to_owned(),
            ],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("direct bash login benchmark command starts");
    assert_eq!(direct_bash_login_output.exit_code, 0);
    assert_eq!(direct_bash_login_output.stdout, b"direct-bash-live");

    let mut batch_trace = hot_batch_100_exec_trace();
    batch_trace.record(SandboxEventName::ExecRequestSent);
    let batch_pid = start_retained_stdin_command(
        &first,
        "while IFS= read -r line; do printf '%s\n' \"$line\"; done",
    )
    .await;
    batch_trace.record(SandboxEventName::ProcessStarted);
    let mut batch_input = String::new();
    let mut batch_expected = String::new();
    for index in 0..100 {
        let line = format!("batch-{index}\n");
        batch_input.push_str(&line);
        batch_expected.push_str(&line);
    }
    finish_retained_stdin_command(&first, batch_pid, batch_input.as_bytes(), &batch_expected).await;
    batch_trace.record(SandboxEventName::ProcessExited);
    samples.extend(
        firkin_evidence::derive_available_contract_metric_samples([batch_trace.finish()])
            .into_iter()
            .map(|sample| {
                if sample.metric() == "exec.batch_100_small_commands_ms" {
                    sample.with_static_tag("batch_mode", "retained_stdin_shell")
                } else {
                    sample
                }
            }),
    );
    let shell_density_level_tag = density_level_tag(&shell_density_levels);
    for repeat in 0..retained_shell_density_repeats {
        let retained_shell_dispatch = timed_retained_shell_dispatch_first_stdout(
            &first,
            &format!("printf retained-shell-live-{repeat}"),
        )
        .await;
        samples.push(
            BenchmarkSample::new(
                "debug.exec.retained_shell_first_stdout_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                retained_shell_dispatch.first_stdout.as_secs_f64() * 1000.0,
            )
            .with_static_tag("measurement_boundary", "retained_shell_cli")
            .with_static_tag("shell_mode", "prestarted_stdin")
            .with_static_tag("cmd", "/bin/bash")
            .with_static_tag("args", "-l|||-c|||while-read-eval")
            .with_dynamic_tag("repeat_index", repeat.to_string())
            .with_dynamic_tag("repeat_count", retained_shell_density_repeats.to_string())
            .with_dynamic_tag(
                "connect_polls",
                retained_shell_dispatch.connect_polls.to_string(),
            ),
        );
        samples.push(
            BenchmarkSample::new(
                "debug.exec.retained_shell_first_stdout_c1_ms",
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                retained_shell_dispatch.first_stdout.as_secs_f64() * 1000.0,
            )
            .with_static_tag("measurement_boundary", "retained_shell_cli_density")
            .with_static_tag("shell_mode", "prestarted_stdin")
            .with_dynamic_tag("concurrency_level", "1")
            .with_dynamic_tag("concurrency_levels", shell_density_level_tag.clone())
            .with_dynamic_tag("repeat_index", repeat.to_string())
            .with_dynamic_tag("repeat_count", retained_shell_density_repeats.to_string())
            .with_dynamic_tag(
                "connect_polls_max",
                retained_shell_dispatch.connect_polls.to_string(),
            ),
        );
    }

    let pre_density_traces = adapter.benchmark_event_traces().await;
    let mut pre_density_samples = adapter.benchmark_samples().await;
    pre_density_samples.extend(firkin_evidence::derive_available_contract_metric_samples(
        pre_density_traces,
    ));

    let mut density_points = vec![DensityP95Point::new(
        1,
        agent_task_ready.as_secs_f64() * 1000.0,
    )];
    samples.push(shell_density_hot_to_first_stdout_sample(
        1,
        &shell_density_levels,
        agent_task_ready,
    ));
    samples.push(shell_density_phase_sample(
        "create",
        1,
        &shell_density_levels,
        first_create,
    ));
    samples.push(shell_density_phase_sample(
        "command",
        1,
        &shell_density_levels,
        first_command,
    ));
    for concurrency in shell_density_levels
        .iter()
        .copied()
        .filter(|level| *level != 1)
    {
        let warm_targets = warm_template_targets_for_depth(ready_templates.clone(), concurrency);
        FirkinWarmTemplateMaintainer::new(adapter.clone(), warm_targets, Duration::from_secs(1))
            .run_cycle()
            .await
            .expect("prewarm density benchmark targets");
        let mut futures = Vec::with_capacity(concurrency);
        for index in 0..concurrency {
            let sandbox_id = format!("sbx_firkin_density_c{concurrency}_{index}");
            futures.push(timed_live_sdk_sandbox_first_stdout(
                live_sdk_config(&control_url, &proxy_url, &sandbox_id),
                "density-live",
            ));
        }
        let sandboxes = join_all(futures).await;
        let concurrent_p95 = sandboxes
            .iter()
            .map(|timing| timing.total)
            .max()
            .expect("density level has sandboxes");
        samples.push(shell_density_hot_to_first_stdout_sample(
            concurrency,
            &shell_density_levels,
            concurrent_p95,
        ));
        for phase in ["create", "command"] {
            let phase_p95 = sandboxes
                .iter()
                .map(|timing| match phase {
                    "create" => timing.create,
                    "command" => timing.command,
                    _ => unreachable!("known density phase"),
                })
                .max()
                .expect("density level has phase timings");
            samples.push(shell_density_phase_sample(
                phase,
                concurrency,
                &shell_density_levels,
                phase_p95,
            ));
        }
        for repeat in 0..retained_shell_density_repeats {
            let retained_shell_dispatch_p95 =
                retained_shell_dispatch_density_p95(&sandboxes, concurrency).await;
            samples.push(
                BenchmarkSample::new(
                    format!("debug.exec.retained_shell_first_stdout_c{concurrency}_ms"),
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    retained_shell_dispatch_p95.first_stdout.as_secs_f64() * 1000.0,
                )
                .with_static_tag("measurement_boundary", "retained_shell_cli_density")
                .with_static_tag("shell_mode", "prestarted_stdin")
                .with_dynamic_tag("concurrency_level", concurrency.to_string())
                .with_dynamic_tag("concurrency_levels", shell_density_level_tag.clone())
                .with_dynamic_tag("repeat_index", repeat.to_string())
                .with_dynamic_tag("repeat_count", retained_shell_density_repeats.to_string())
                .with_dynamic_tag(
                    "connect_polls_max",
                    retained_shell_dispatch_p95.connect_polls.to_string(),
                ),
            );
        }
        density_points.push(DensityP95Point::new(
            concurrency as u64,
            concurrent_p95.as_secs_f64() * 1000.0,
        ));
        for timing in sandboxes {
            assert!(timing.sandbox.kill().await.unwrap());
        }
    }
    if let Some(sample) = retained_shell_density_breakpoint_sample(&samples) {
        samples.push(sample);
    }
    let density_limit = max_active_before_p95_doubles(density_points).expect("density breakpoint");
    samples.push(
        density_limit
            .into_sample()
            .with_dynamic_tag(
                "concurrency_levels",
                density_level_tag(&shell_density_levels),
            )
            .with_dynamic_tag(
                "baseline_p95_ms",
                format!("{:.6}", density_limit.baseline_p95_latency_ms()),
            )
            .with_dynamic_tag(
                "threshold_p95_ms",
                format!("{:.6}", density_limit.threshold_p95_latency_ms()),
            ),
    );
    assert!(first.kill().await.unwrap());
    samples.extend(pre_density_samples);

    proxy_task.abort();
    control_task.abort();
    samples
}

#[derive(Default)]
struct LiveAgentComputerScorecardEvidence {
    samples: Vec<BenchmarkSample>,
    traces: Vec<SandboxEventTrace>,
    ready_queue_hits: u64,
    ready_queue_misses: u64,
    max_active_product_pods: u32,
}

impl LiveAgentComputerScorecardEvidence {
    fn observed_autoscale_harness(&self) -> LiveAutoscaleHarnessObservation {
        LiveAutoscaleHarnessObservation::new(
            ReadyQueueOutcomes::new(self.ready_queue_hits, self.ready_queue_misses),
            self.max_active_product_pods,
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct LiveAutoscaleHarnessObservation {
    ready_queue_outcomes: ReadyQueueOutcomes,
    ready_queue_capacity: u64,
    max_active_product_pods: u32,
}

impl LiveAutoscaleHarnessObservation {
    const PRODUCT_POD_CPU_SLOTS: u32 = 1;
    const PRODUCT_POD_MEMORY_BYTES: u64 = 512 * 1024 * 1024;

    fn new(ready_queue_outcomes: ReadyQueueOutcomes, max_active_product_pods: u32) -> Self {
        Self {
            ready_queue_outcomes,
            ready_queue_capacity: ready_queue_outcomes.ready_hits(),
            max_active_product_pods,
        }
    }

    fn with_ready_queue_capacity(mut self, ready_queue_capacity: u64) -> Self {
        self.ready_queue_capacity = ready_queue_capacity;
        self
    }

    fn ready_queue_outcomes(self) -> ReadyQueueOutcomes {
        self.ready_queue_outcomes
    }

    fn active_budget(self) -> AutoscaleResourceBudget {
        AutoscaleResourceBudget::new(
            self.max_active_product_pods
                .saturating_mul(Self::PRODUCT_POD_CPU_SLOTS),
            Size::bytes(
                u64::from(self.max_active_product_pods)
                    .saturating_mul(Self::PRODUCT_POD_MEMORY_BYTES),
            ),
            Size::bytes(
                u64::from(self.max_active_product_pods)
                    .saturating_mul(PYTHON_PRODUCT_POD_STORE_BYTES),
            ),
        )
    }

    fn ready_queue_budget(self) -> AutoscaleResourceBudget {
        let cpus = self
            .ready_queue_capacity
            .try_into()
            .unwrap_or(u32::MAX)
            .saturating_mul(Self::PRODUCT_POD_CPU_SLOTS);
        AutoscaleResourceBudget::new(
            cpus,
            Size::bytes(
                self.ready_queue_capacity
                    .saturating_mul(Self::PRODUCT_POD_MEMORY_BYTES),
            ),
            Size::bytes(
                self.ready_queue_capacity
                    .saturating_mul(PYTHON_PRODUCT_POD_STORE_BYTES),
            ),
        )
    }

    fn safe_spare_cpu_slots(self) -> u32 {
        host_logical_cpu_count()
            .saturating_sub(self.active_budget().cpus())
            .max(1)
    }

    fn snappy_ready_queue_capacity_target(self) -> usize {
        ((f64::from(self.safe_spare_cpu_slots()) * 0.70).ceil() as usize).max(1)
    }
}

async fn collect_live_agent_computer_scorecard_evidence() -> LiveAgentComputerScorecardEvidence {
    let mut evidence = LiveAgentComputerScorecardEvidence::default();
    let temp = tempfile::tempdir().expect("agent-computer scorecard product pod tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!(
        "live-agent-computer-scorecard-{}",
        uuid::Uuid::new_v4().simple()
    );
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let control_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([(
                "benchmark".to_owned(),
                "agent-computer-scorecard-product-pod".to_owned(),
            )]),
            empty_dirs: vec![
                PodEmptyDir {
                    name: "db".to_owned(),
                },
                PodEmptyDir {
                    name: "browser".to_owned(),
                },
            ],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: ready_deck_sidecar_containers(),
        })
        .await
        .expect("create agent-computer scorecard product pod");
    evidence.max_active_product_pods = evidence
        .max_active_product_pods
        .max(backend.pods().list().len().try_into().unwrap_or(u32::MAX));
    validate_ready_deck(&mut backend, &pod_id).await;

    let mut ready_trace = agent_computer_event_trace(LifecycleClass::Hot);
    ready_trace.record(SandboxEventName::AgentComputerRequestStart);
    backend
        .add_pod_container(
            &pod_id,
            ready_deck_agent_container("agent-computer-scorecard-ready"),
        )
        .await
        .expect("add agent-computer scorecard ready agent");
    ready_trace.record(SandboxEventName::AgentComputerSandboxCreated);
    ready_trace.record(SandboxEventName::AgentComputerProbeStart);
    let ready_output = backend
        .wait_pod_container(&pod_id, "agent-computer-scorecard-ready")
        .await
        .expect("wait agent-computer scorecard ready agent");
    assert_eq!(
        ready_output.exit_code,
        0,
        "agent-computer scorecard ready agent failed: stdout={} stderr={}",
        String::from_utf8_lossy(&ready_output.stdout),
        String::from_utf8_lossy(&ready_output.stderr)
    );
    assert_eq!(ready_output.stdout, b"agent-ready");
    evidence.ready_queue_hits += 1;
    ready_trace.record(SandboxEventName::CliFirstUsefulStdout);
    ready_trace.record(SandboxEventName::DatabaseReady);
    ready_trace.record(SandboxEventName::BrowserReady);
    ready_trace.record(SandboxEventName::AgentComputerReady);
    let ready_event_trace = ready_trace.finish();
    evidence
        .samples
        .extend(derive_live_product_pod_scorecard_samples(
            ready_event_trace.clone(),
        ));
    evidence.traces.push(ready_event_trace);

    let resume_slot = "agent-computer-scorecard-resume";
    backend
        .add_pod_container(
            &pod_id,
            ready_deck_prestarted_agent_slot_container(resume_slot),
        )
        .await
        .expect("add agent-computer scorecard resume slot");
    validate_prestarted_agent_slots(&mut backend, &pod_id, &[resume_slot.to_owned()]).await;

    let mut resume_trace = agent_computer_event_trace(LifecycleClass::Resumed);
    resume_trace.record(SandboxEventName::AgentComputerResumed);
    dispatch_ready_deck_prestarted_agent_slot(
        &control_driver,
        &pod_id,
        resume_slot,
        &mut resume_trace,
    )
    .await;
    evidence.ready_queue_hits += 1;
    let resume_event_trace = resume_trace.finish();
    evidence
        .samples
        .extend(derive_live_product_pod_scorecard_samples(
            resume_event_trace.clone(),
        ));
    evidence.traces.push(resume_event_trace);

    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete agent-computer scorecard product pod");
    let density_levels = live_runtime_density_levels(
        std::env::var_os("FIRKIN_LIVE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS").as_deref(),
        &[1, 2, 4],
    );
    let (density_samples, density_traces) =
        collect_live_product_pod_ready_deck_density_samples(&density_levels).await;
    evidence.samples.extend(density_samples);
    evidence.traces.extend(density_traces);
    evidence
        .samples
        .extend(collect_live_reliability_benchmark_samples().await);
    evidence
        .samples
        .extend(collect_live_cleanup_leftover_benchmark_samples().await);

    evidence
}

fn agent_computer_event_trace(lifecycle: LifecycleClass) -> EventTraceRecorder {
    EventTraceRecorder::new(
        lifecycle,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    )
}

fn agent_computer_density_event_trace(lifecycle: LifecycleClass) -> EventTraceRecorder {
    EventTraceRecorder::new(
        lifecycle,
        WorkloadClass::ConcurrentCreate,
        RuntimeProfile::BrowserDbCli,
    )
}

fn derive_live_agent_computer_proxy_database_samples(
    trace: SandboxEventTrace,
) -> impl Iterator<Item = BenchmarkSample> {
    firkin_evidence::derive_available_product_autoscale_metric_samples([trace])
        .into_iter()
        .map(mark_proxy_database_boundary)
}

fn derive_live_product_pod_scorecard_samples(
    trace: SandboxEventTrace,
) -> impl Iterator<Item = BenchmarkSample> {
    firkin_evidence::derive_available_product_autoscale_metric_samples([trace])
        .into_iter()
        .map(|sample| {
            sample
                .with_static_tag("probe_surface", "browser_db_cli_readiness")
                .with_static_tag("measurement_boundary", "product_path")
                .with_static_tag("cli_boundary", "real_cli")
                .with_static_tag("browser_boundary", "real_browser_sidecar")
                .with_static_tag("database_boundary", "real_db_sidecar")
                .with_static_tag("pod_surface", "product_pod_ready_deck")
        })
}

fn mark_proxy_database_boundary(sample: BenchmarkSample) -> BenchmarkSample {
    match sample.metric() {
        "product.agent_computer_ready_ms" | "product.agent_computer_resume_ms" => sample
            .with_static_tag("cli_boundary", "code_interpreter_exec")
            .with_static_tag("browser_boundary", "code_interpreter_health")
            .with_static_tag("database_boundary", "sqlite_proxy_not_db_sidecar"),
        "product.database_ready_ms" => sample
            .with_static_tag("database_boundary", "sqlite_proxy_not_db_sidecar")
            .with_static_tag("database_probe_surface", "code_interpreter_sqlite"),
        _ => sample,
    }
}

async fn collect_live_db_sidecar_readiness_sample() -> (BenchmarkSample, SandboxEventTrace) {
    let temp = tempfile::tempdir().expect("DB sidecar tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!("live-db-sidecar-{}", uuid::Uuid::new_v4().simple());
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let metrics_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([("benchmark".to_owned(), "db-sidecar-readiness".to_owned())]),
            empty_dirs: vec![PodEmptyDir {
                name: "db".to_owned(),
            }],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: vec![PodContainerCreateRequest {
                name: "agent".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    "sleep 300".to_owned(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "db".to_owned(),
                    path: "/db".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            }],
        })
        .await
        .expect("create DB sidecar product pod");
    trace.record(SandboxEventName::AgentComputerSandboxCreated);
    let before_ready_bytes = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read pod-store usage before DB sidecar");
    backend
        .add_pod_container(
            &pod_id,
            PodContainerCreateRequest {
                name: "db".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    db_sidecar_readiness_command(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "db".to_owned(),
                    path: "/db".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            },
        )
        .await
        .expect("add DB sidecar container");
    wait_for_db_sidecar_ready(&metrics_driver, &pod_id, before_ready_bytes).await;
    trace.record(SandboxEventName::DatabaseReady);
    let event_trace = trace.finish();
    let sample = real_db_sidecar_database_sample(&event_trace);
    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete DB sidecar pod");
    (sample, event_trace)
}

fn db_sidecar_readiness_command() -> String {
    r#"python3 - <<'PY'
import os, sqlite3, time
conn = sqlite3.connect('/db/ready.db')
conn.execute('create table if not exists readiness(value integer not null)')
conn.execute('insert into readiness(value) values (1)')
conn.execute('select 1').fetchone()
conn.commit()
conn.close()
heartbeat = open('/db/heartbeat', 'wb')
heartbeat.write(b'db-ready')
heartbeat.flush()
os.fsync(heartbeat.fileno())
heartbeat.close()
with open('/db/ready-proof.bin', 'wb') as proof:
    proof.write(b'1' * (4 * 1024 * 1024))
    proof.flush()
    os.fsync(proof.fileno())
time.sleep(300)
PY"#
    .to_owned()
}

async fn wait_for_db_sidecar_ready(
    driver: &firkin_single_node::AppleVzLocalRuntimeDriver,
    pod_id: &str,
    before_ready_bytes: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let used = driver
            .pod_store_used_bytes(pod_id)
            .await
            .expect("read pod-store usage during DB sidecar readiness");
        if used > before_ready_bytes {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "DB sidecar did not write readiness proof: before={before_ready_bytes} current={used}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn real_db_sidecar_database_sample(trace: &SandboxEventTrace) -> BenchmarkSample {
    firkin_evidence::derive_product_autoscale_metric_sample(
        trace,
        ProductAutoscaleDurationMetric::AgentComputerDatabaseReady,
    )
    .expect("derive real DB sidecar sample")
    .into_benchmark_sample()
    .with_static_tag("probe_surface", "db_sidecar_health")
    .with_static_tag("measurement_boundary", "db_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod")
}

async fn collect_live_browser_sidecar_readiness_sample() -> (BenchmarkSample, SandboxEventTrace) {
    let temp = tempfile::tempdir().expect("browser sidecar tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!("live-browser-sidecar-{}", uuid::Uuid::new_v4().simple());
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let metrics_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([(
                "benchmark".to_owned(),
                "browser-sidecar-readiness".to_owned(),
            )]),
            empty_dirs: vec![PodEmptyDir {
                name: "browser".to_owned(),
            }],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: vec![PodContainerCreateRequest {
                name: "agent".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    "sleep 300".to_owned(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "browser".to_owned(),
                    path: "/browser".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            }],
        })
        .await
        .expect("create browser sidecar product pod");
    trace.record(SandboxEventName::AgentComputerSandboxCreated);
    let before_ready_bytes = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read pod-store usage before browser sidecar");
    backend
        .add_pod_container(
            &pod_id,
            PodContainerCreateRequest {
                name: "browser".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    browser_sidecar_readiness_command(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "browser".to_owned(),
                    path: "/browser".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            },
        )
        .await
        .expect("add browser sidecar container");
    wait_for_browser_sidecar_ready(&metrics_driver, &pod_id, before_ready_bytes).await;
    trace.record(SandboxEventName::BrowserReady);
    let event_trace = trace.finish();
    let sample = real_browser_sidecar_browser_sample(&event_trace);
    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete browser sidecar pod");
    (sample, event_trace)
}

fn browser_sidecar_readiness_command() -> String {
    r#"python3 - <<'PY'
import http.server, os, socketserver, threading, time, urllib.request
class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'browser-ready')
    def log_message(self, fmt, *args):
        pass
server = socketserver.TCPServer(('127.0.0.1', 9222), Handler)
threading.Thread(target=server.serve_forever, daemon=True).start()
body = urllib.request.urlopen('http://127.0.0.1:9222/', timeout=5).read()
assert body == b'browser-ready'
heartbeat = open('/browser/heartbeat', 'wb')
heartbeat.write(b'browser-ready')
heartbeat.flush()
os.fsync(heartbeat.fileno())
heartbeat.close()
with open('/browser/ready-proof.bin', 'wb') as proof:
    proof.write(b'1' * (1024 * 1024))
    proof.flush()
    os.fsync(proof.fileno())
time.sleep(300)
PY"#
    .to_owned()
}

async fn wait_for_browser_sidecar_ready(
    driver: &firkin_single_node::AppleVzLocalRuntimeDriver,
    pod_id: &str,
    before_ready_bytes: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let used = driver
            .pod_store_used_bytes(pod_id)
            .await
            .expect("read pod-store usage during browser sidecar readiness");
        if used > before_ready_bytes {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "browser sidecar did not write readiness proof: before={before_ready_bytes} current={used}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn real_browser_sidecar_browser_sample(trace: &SandboxEventTrace) -> BenchmarkSample {
    firkin_evidence::derive_product_autoscale_metric_sample(
        trace,
        ProductAutoscaleDurationMetric::AgentComputerBrowserReady,
    )
    .expect("derive real browser sidecar sample")
    .into_benchmark_sample()
    .with_static_tag("probe_surface", "browser_sidecar_health")
    .with_static_tag("measurement_boundary", "browser_sidecar")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("pod_surface", "product_pod")
}

async fn collect_live_product_pod_readiness_sample() -> (BenchmarkSample, SandboxEventTrace) {
    let temp = tempfile::tempdir().expect("product pod tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!("live-product-pod-{}", uuid::Uuid::new_v4().simple());
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let metrics_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    let mut trace = agent_computer_event_trace(LifecycleClass::Hot);
    trace.record(SandboxEventName::AgentComputerRequestStart);
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([(
                "benchmark".to_owned(),
                "product-pod-readiness".to_owned(),
            )]),
            empty_dirs: vec![
                PodEmptyDir {
                    name: "db".to_owned(),
                },
                PodEmptyDir {
                    name: "browser".to_owned(),
                },
            ],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: vec![PodContainerCreateRequest {
                name: "agent".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    "printf agent-ready".to_owned(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: Vec::new(),
                capture_output: true,
            }],
        })
        .await
        .expect("create integrated product pod");
    trace.record(SandboxEventName::AgentComputerSandboxCreated);
    let cli_output = backend
        .wait_pod_container(&pod_id, "agent")
        .await
        .expect("wait product-pod agent CLI container");
    assert_eq!(cli_output.exit_code, 0);
    assert_eq!(cli_output.stdout, b"agent-ready");
    trace.record(SandboxEventName::CliFirstUsefulStdout);

    let before_db_ready_bytes = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read pod-store usage before integrated DB sidecar");
    backend
        .add_pod_container(
            &pod_id,
            PodContainerCreateRequest {
                name: "db".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    db_sidecar_readiness_command(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "db".to_owned(),
                    path: "/db".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            },
        )
        .await
        .expect("add integrated DB sidecar container");
    wait_for_db_sidecar_ready(&metrics_driver, &pod_id, before_db_ready_bytes).await;
    trace.record(SandboxEventName::DatabaseReady);

    let before_browser_ready_bytes = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read pod-store usage before integrated browser sidecar");
    backend
        .add_pod_container(
            &pod_id,
            PodContainerCreateRequest {
                name: "browser".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    browser_sidecar_readiness_command(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "browser".to_owned(),
                    path: "/browser".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            },
        )
        .await
        .expect("add integrated browser sidecar container");
    wait_for_browser_sidecar_ready(&metrics_driver, &pod_id, before_browser_ready_bytes).await;
    trace.record(SandboxEventName::BrowserReady);
    trace.record(SandboxEventName::AgentComputerReady);
    let event_trace = trace.finish();
    let sample = real_product_pod_readiness_sample(&event_trace);
    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete integrated product pod");
    (sample, event_trace)
}

fn real_product_pod_readiness_sample(trace: &SandboxEventTrace) -> BenchmarkSample {
    firkin_evidence::derive_product_autoscale_metric_sample(
        trace,
        ProductAutoscaleDurationMetric::AgentComputerReady,
    )
    .expect("derive real product-pod readiness sample")
    .into_benchmark_sample()
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "product_path")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod")
}

async fn collect_live_product_pod_ready_deck_samples(
    repeats: usize,
) -> (Vec<BenchmarkSample>, Vec<SandboxEventTrace>) {
    let temp = tempfile::tempdir().expect("ready-deck product pod tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!("live-ready-deck-{}", uuid::Uuid::new_v4().simple());
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let control_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([(
                "benchmark".to_owned(),
                "product-pod-ready-deck".to_owned(),
            )]),
            empty_dirs: vec![
                PodEmptyDir {
                    name: "db".to_owned(),
                },
                PodEmptyDir {
                    name: "browser".to_owned(),
                },
            ],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: ready_deck_sidecar_containers(),
        })
        .await
        .expect("create ready-deck product pod");

    validate_ready_deck(&mut backend, &pod_id).await;
    let slot_names = (0..repeats)
        .map(|repeat| format!("agent-{repeat}"))
        .collect::<Vec<_>>();
    for slot_name in &slot_names {
        backend
            .add_pod_container(
                &pod_id,
                ready_deck_prestarted_agent_slot_container(slot_name),
            )
            .await
            .expect("add ready-deck prestarted agent slot");
    }
    validate_prestarted_agent_slots(&mut backend, &pod_id, &slot_names).await;

    let mut samples = Vec::with_capacity(repeats);
    let mut traces = Vec::with_capacity(repeats);
    for slot_name in slot_names {
        let mut trace = agent_computer_event_trace(LifecycleClass::Resumed);
        trace.record(SandboxEventName::AgentComputerResumed);
        dispatch_ready_deck_prestarted_agent_slot(&control_driver, &pod_id, &slot_name, &mut trace)
            .await;
        let event_trace = trace.finish();
        samples.push(real_product_pod_ready_deck_sample(&event_trace));
        traces.push(event_trace);
    }

    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete ready-deck product pod");
    (samples, traces)
}

async fn collect_live_product_pod_ready_deck_density_samples(
    concurrency_levels: &[usize],
) -> (Vec<BenchmarkSample>, Vec<SandboxEventTrace>) {
    assert!(
        concurrency_levels.contains(&1),
        "ready-deck density requires a single-agent baseline"
    );
    let temp = tempfile::tempdir().expect("ready-deck density product pod tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!("live-ready-deck-density-{}", uuid::Uuid::new_v4().simple());
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let metrics_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([(
                "benchmark".to_owned(),
                "product-pod-ready-deck-density".to_owned(),
            )]),
            empty_dirs: vec![
                PodEmptyDir {
                    name: "db".to_owned(),
                },
                PodEmptyDir {
                    name: "browser".to_owned(),
                },
            ],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: ready_deck_sidecar_containers(),
        })
        .await
        .expect("create ready-deck density product pod");
    validate_ready_deck(&mut backend, &pod_id).await;

    let mut points = Vec::with_capacity(concurrency_levels.len());
    let mut samples = Vec::with_capacity((concurrency_levels.len() * 3) + 2);
    let mut traces = Vec::new();
    let level_tag = density_level_tag(concurrency_levels);
    for &concurrency in concurrency_levels {
        assert!(
            concurrency > 0,
            "ready-deck density concurrency must be positive"
        );
        let phase_sample_offset = metrics_driver
            .pod_container_add_benchmark_samples()
            .await
            .len();
        let mut futures = Vec::with_capacity(concurrency);
        for index in 0..concurrency {
            let mut backend = backend.clone();
            let pod_id = pod_id.clone();
            let agent_name = format!("density-c{concurrency}-{index}");
            futures.push(async move {
                let mut trace = agent_computer_density_event_trace(LifecycleClass::Resumed);
                trace.record(SandboxEventName::AgentComputerResumed);
                backend
                    .add_pod_container(&pod_id, ready_deck_agent_container(&agent_name))
                    .await
                    .expect("add ready-deck density agent");
                trace.record(SandboxEventName::AgentComputerSandboxCreated);
                trace.record(SandboxEventName::AgentComputerProbeStart);
                let agent_output = backend
                    .wait_pod_container(&pod_id, &agent_name)
                    .await
                    .expect("wait ready-deck density agent");
                assert_eq!(
                    agent_output.exit_code,
                    0,
                    "ready-deck density agent failed: stdout={} stderr={}",
                    String::from_utf8_lossy(&agent_output.stdout),
                    String::from_utf8_lossy(&agent_output.stderr)
                );
                assert_eq!(agent_output.stdout, b"agent-ready");
                trace.record(SandboxEventName::CliFirstUsefulStdout);
                trace.record(SandboxEventName::DatabaseReady);
                trace.record(SandboxEventName::BrowserReady);
                trace.record(SandboxEventName::AgentComputerReady);
                trace.finish()
            });
        }
        let level_traces = join_all(futures).await;
        let p95_latency_ms = level_traces
            .iter()
            .map(|trace| {
                trace
                    .duration_between(
                        SandboxEventName::AgentComputerResumed,
                        SandboxEventName::AgentComputerReady,
                    )
                    .expect("ready-deck density trace duration")
                    .as_secs_f64()
                    * 1000.0
            })
            .max_by(f64::total_cmp)
            .expect("density level has traces");
        let container_add_p95_latency_ms = level_traces
            .iter()
            .map(|trace| {
                trace
                    .duration_between(
                        SandboxEventName::AgentComputerResumed,
                        SandboxEventName::AgentComputerSandboxCreated,
                    )
                    .expect("ready-deck density container-add trace duration")
                    .as_secs_f64()
                    * 1000.0
            })
            .max_by(f64::total_cmp)
            .expect("density level has container-add traces");
        let output_wait_p95_latency_ms = level_traces
            .iter()
            .map(|trace| {
                trace
                    .duration_between(
                        SandboxEventName::AgentComputerSandboxCreated,
                        SandboxEventName::AgentComputerReady,
                    )
                    .expect("ready-deck density output-wait trace duration")
                    .as_secs_f64()
                    * 1000.0
            })
            .max_by(f64::total_cmp)
            .expect("density level has output-wait traces");
        points.push(DensityP95Point::new(concurrency as u64, p95_latency_ms));
        samples.push(product_pod_ready_deck_density_level_sample(
            concurrency,
            &level_tag,
            p95_latency_ms,
        ));
        samples.push(product_pod_ready_deck_container_add_level_sample(
            concurrency,
            &level_tag,
            container_add_p95_latency_ms,
        ));
        samples.push(product_pod_ready_deck_output_wait_level_sample(
            concurrency,
            &level_tag,
            output_wait_p95_latency_ms,
        ));
        let phase_samples = metrics_driver.pod_container_add_benchmark_samples().await;
        samples.extend(product_pod_ready_deck_add_phase_level_samples(
            concurrency,
            &level_tag,
            &phase_samples[phase_sample_offset..],
        ));
        traces.extend(level_traces);
    }
    samples.extend(product_pod_ready_deck_degradation_samples(
        &points, &level_tag,
    ));

    let limit = max_active_before_p95_doubles(points.iter().copied())
        .expect("ready-deck density breakpoint");
    let sample = limit
        .into_agent_computer_sample()
        .with_static_tag("probe_surface", "browser_db_cli_readiness")
        .with_static_tag("measurement_boundary", "product_path")
        .with_static_tag("cli_boundary", "real_cli")
        .with_static_tag("browser_boundary", "real_browser_sidecar")
        .with_static_tag("database_boundary", "real_db_sidecar")
        .with_static_tag("pod_surface", "product_pod_ready_deck")
        .with_static_tag("excludes_container_add", "false")
        .with_static_tag("ready_signal", "agent_computer_ready_after_container_add")
        .with_dynamic_tag("concurrency_levels", level_tag)
        .with_dynamic_tag(
            "baseline_p95_ms",
            format!("{:.6}", limit.baseline_p95_latency_ms()),
        )
        .with_dynamic_tag(
            "threshold_p95_ms",
            format!("{:.6}", limit.threshold_p95_latency_ms()),
        );
    samples.push(sample);

    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete ready-deck density product pod");
    (samples, traces)
}

async fn collect_live_product_pod_prestarted_agent_slot_density_samples(
    concurrency_levels: &[usize],
) -> (Vec<BenchmarkSample>, Vec<SandboxEventTrace>) {
    assert!(
        concurrency_levels.contains(&1),
        "prestarted agent-slot density requires a single-slot baseline"
    );
    let temp = tempfile::tempdir().expect("prestarted agent-slot density product pod tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!(
        "live-prestarted-slot-density-{}",
        uuid::Uuid::new_v4().simple()
    );
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let control_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([(
                "benchmark".to_owned(),
                "product-pod-prestarted-agent-slot-density".to_owned(),
            )]),
            empty_dirs: vec![
                PodEmptyDir {
                    name: "db".to_owned(),
                },
                PodEmptyDir {
                    name: "browser".to_owned(),
                },
            ],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: ready_deck_sidecar_containers(),
        })
        .await
        .expect("create prestarted agent-slot density product pod");
    validate_ready_deck(&mut backend, &pod_id).await;

    let mut points = Vec::with_capacity(concurrency_levels.len());
    let mut samples = Vec::with_capacity(concurrency_levels.len() + 1);
    let mut traces = Vec::new();
    let mut prestarted_slots = 0_usize;
    let level_tag = density_level_tag(concurrency_levels);
    for &concurrency in concurrency_levels {
        assert!(
            concurrency > 0,
            "prestarted agent-slot density concurrency must be positive"
        );
        let slot_names = (0..concurrency)
            .map(|index| format!("slot-c{concurrency}-{index}"))
            .collect::<Vec<_>>();
        for slot_name in &slot_names {
            backend
                .add_pod_container(
                    &pod_id,
                    ready_deck_prestarted_agent_slot_container(slot_name),
                )
                .await
                .expect("add prestarted agent slot");
            prestarted_slots += 1;
        }
        validate_prestarted_agent_slots(&mut backend, &pod_id, &slot_names).await;

        let mut trace_recorders = slot_names
            .iter()
            .map(|_| {
                let mut trace = agent_computer_density_event_trace(LifecycleClass::Resumed);
                trace.record(SandboxEventName::AgentComputerResumed);
                trace
            })
            .collect::<Vec<_>>();
        signal_ready_deck_prestarted_agent_slots(&control_driver, &pod_id, concurrency).await;
        for trace in &mut trace_recorders {
            trace.record(SandboxEventName::AgentComputerSandboxCreated);
            trace.record(SandboxEventName::AgentComputerProbeStart);
        }
        let mut futures = Vec::with_capacity(concurrency);
        for (slot_name, mut trace) in slot_names.into_iter().zip(trace_recorders) {
            let driver = control_driver.clone();
            let pod_id = pod_id.clone();
            futures.push(async move {
                wait_ready_deck_prestarted_agent_slot(&driver, &pod_id, &slot_name, &mut trace)
                    .await;
                trace.finish()
            });
        }
        let level_traces = join_all(futures).await;
        let p95_latency_ms = level_traces
            .iter()
            .map(|trace| {
                trace
                    .duration_between(
                        SandboxEventName::AgentComputerResumed,
                        SandboxEventName::AgentComputerSandboxCreated,
                    )
                    .expect("prestarted agent-slot density trace duration")
                    .as_secs_f64()
                    * 1000.0
            })
            .max_by(f64::total_cmp)
            .expect("prestarted agent-slot density level has traces");
        let output_wait_p95_latency_ms = level_traces
            .iter()
            .map(|trace| {
                trace
                    .duration_between(
                        SandboxEventName::AgentComputerSandboxCreated,
                        SandboxEventName::AgentComputerReady,
                    )
                    .expect("prestarted agent-slot output wait trace duration")
                    .as_secs_f64()
                    * 1000.0
            })
            .max_by(f64::total_cmp)
            .expect("prestarted agent-slot output wait level has traces");
        points.push(DensityP95Point::new(concurrency as u64, p95_latency_ms));
        samples.push(prestarted_agent_slot_density_level_sample(
            concurrency,
            &level_tag,
            p95_latency_ms,
        ));
        samples.push(prestarted_agent_slot_output_wait_level_sample(
            concurrency,
            &level_tag,
            output_wait_p95_latency_ms,
        ));
        traces.extend(level_traces);
    }

    let snappy_guard_sample = prestarted_agent_slot_fifo_acceptance_p95_sample(points.clone())
        .expect("prestarted agent-slot FIFO acceptance p95 sample")
        .with_static_tag("probe_surface", "browser_db_cli_readiness")
        .with_static_tag("cli_boundary", "real_cli")
        .with_static_tag("browser_boundary", "real_browser_sidecar")
        .with_static_tag("database_boundary", "real_db_sidecar")
        .with_static_tag("pod_surface", "product_pod_ready_deck")
        .with_static_tag("output_wait_preserved", "true")
        .with_dynamic_tag("prestarted_slots", prestarted_slots.to_string())
        .with_dynamic_tag("concurrency_levels", level_tag.clone());
    let limit =
        max_active_before_p95_doubles(points).expect("prestarted agent-slot density breakpoint");
    let sample = limit
        .into_prestarted_agent_slot_sample()
        .with_static_tag("probe_surface", "browser_db_cli_readiness")
        .with_static_tag("measurement_boundary", "prestarted_slot_checkout")
        .with_static_tag("cli_boundary", "real_cli")
        .with_static_tag("browser_boundary", "real_browser_sidecar")
        .with_static_tag("database_boundary", "real_db_sidecar")
        .with_static_tag("pod_surface", "product_pod_ready_deck")
        .with_static_tag("slot_surface", "prestarted_agent_slot")
        .with_static_tag("excludes_container_add", "true")
        .with_static_tag("ready_signal", "request_fifo_acceptance")
        .with_static_tag("output_wait_preserved", "true")
        .with_dynamic_tag("prestarted_slots", prestarted_slots.to_string())
        .with_dynamic_tag("concurrency_levels", level_tag)
        .with_dynamic_tag(
            "baseline_p95_ms",
            format!("{:.6}", limit.baseline_p95_latency_ms()),
        )
        .with_dynamic_tag(
            "threshold_p95_ms",
            format!("{:.6}", limit.threshold_p95_latency_ms()),
        );
    samples.push(sample);
    samples.push(snappy_guard_sample);

    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete prestarted agent-slot density product pod");
    (samples, traces)
}

async fn collect_live_autoscale_ready_queue_capacity(target_slots: usize) -> u64 {
    assert!(
        target_slots > 0,
        "autoscale ready queue capacity requires at least one prestarted slot"
    );
    let temp = tempfile::tempdir().expect("autoscale ready queue capacity tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!(
        "live-autoscale-ready-queue-{}",
        uuid::Uuid::new_v4().simple()
    );
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([(
                "benchmark".to_owned(),
                "autoscale-ready-queue-capacity".to_owned(),
            )]),
            empty_dirs: vec![
                PodEmptyDir {
                    name: "db".to_owned(),
                },
                PodEmptyDir {
                    name: "browser".to_owned(),
                },
            ],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: ready_deck_sidecar_containers(),
        })
        .await
        .expect("create autoscale ready queue capacity product pod");
    validate_ready_deck(&mut backend, &pod_id).await;

    let slot_names = (0..target_slots)
        .map(|index| format!("autoscale-ready-slot-{index}"))
        .collect::<Vec<_>>();
    for slot_name in &slot_names {
        backend
            .add_pod_container(
                &pod_id,
                ready_deck_prestarted_agent_slot_container(slot_name),
            )
            .await
            .expect("add autoscale ready queue prestarted slot");
    }
    validate_prestarted_agent_slots(&mut backend, &pod_id, &slot_names).await;

    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete autoscale ready queue capacity pod");
    slot_names.len().try_into().unwrap_or(u64::MAX)
}

fn product_pod_ready_deck_density_level_sample(
    concurrency: usize,
    concurrency_levels: &str,
    p95_latency_ms: f64,
) -> BenchmarkSample {
    let mut sample = BenchmarkSample::new(
        format!("debug.product.agent_computer_ready_deck_c{concurrency}_ms"),
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        p95_latency_ms,
    )
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "product_path_density_level")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod_ready_deck")
    .with_static_tag("excludes_container_add", "false")
    .with_static_tag("ready_signal", "agent_computer_ready_after_container_add")
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("concurrency_levels", concurrency_levels);
    if let Some((tier, max_ready_ms)) = product_density_capacity_tier(concurrency as u64) {
        sample = sample
            .with_static_tag("density_tier", tier)
            .with_dynamic_tag("capacity_max_ready_ms", format!("{max_ready_ms:.6}"))
            .with_static_tag(
                "capacity_status",
                if p95_latency_ms <= max_ready_ms {
                    "pass"
                } else {
                    "miss"
                },
            );
    } else {
        sample = sample.with_static_tag("capacity_status", "observed_no_tier");
    }
    sample
}

fn product_pod_ready_deck_degradation_samples(
    points: &[DensityP95Point],
    concurrency_levels: &str,
) -> Vec<BenchmarkSample> {
    let Some(baseline) = points
        .iter()
        .find(|point| point.concurrency() == 1)
        .map(|point| point.p95_latency_ms())
    else {
        return Vec::new();
    };
    points
        .iter()
        .map(|point| {
            let concurrency = point.concurrency();
            let ratio = point.p95_latency_ms() / baseline;
            BenchmarkSample::new(
                format!(
                    "debug.product.agent_computer_ready_deck_p95_degradation_c{concurrency}_ratio"
                ),
                BenchmarkMetricKind::WorkloadResource,
                BenchmarkUnit::Ratio,
                ratio,
            )
            .with_static_tag("probe_surface", "browser_db_cli_readiness")
            .with_static_tag("measurement_boundary", "product_path_density_degradation")
            .with_static_tag("cli_boundary", "real_cli")
            .with_static_tag("browser_boundary", "real_browser_sidecar")
            .with_static_tag("database_boundary", "real_db_sidecar")
            .with_static_tag("pod_surface", "product_pod_ready_deck")
            .with_static_tag("excludes_container_add", "false")
            .with_static_tag("ready_signal", "agent_computer_ready_after_container_add")
            .with_dynamic_tag("concurrency_level", concurrency.to_string())
            .with_dynamic_tag("concurrency_levels", concurrency_levels)
            .with_dynamic_tag("baseline_p95_ms", format!("{baseline:.6}"))
            .with_dynamic_tag("level_p95_ms", format!("{:.6}", point.p95_latency_ms()))
            .with_static_tag("degradation_status", "diagnostic_only")
        })
        .collect()
}

fn product_density_capacity_tier(concurrency: u64) -> Option<(&'static str, f64)> {
    match concurrency {
        4 => Some(("snappy_4", 125.0)),
        8 => Some(("snappy_8", 250.0)),
        16 => Some(("degraded_16", 500.0)),
        _ => None,
    }
}

fn product_pod_ready_deck_container_add_level_sample(
    concurrency: usize,
    concurrency_levels: &str,
    p95_latency_ms: f64,
) -> BenchmarkSample {
    BenchmarkSample::new(
        format!("debug.product.agent_computer_container_add_c{concurrency}_ms"),
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        p95_latency_ms,
    )
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "product_path_container_add")
    .with_static_tag("phase", "pod_container_add")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod_ready_deck")
    .with_static_tag("excludes_container_add", "false")
    .with_static_tag("ready_signal", "container_added_before_agent_output")
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("concurrency_levels", concurrency_levels)
}

fn product_pod_ready_deck_output_wait_level_sample(
    concurrency: usize,
    concurrency_levels: &str,
    p95_latency_ms: f64,
) -> BenchmarkSample {
    BenchmarkSample::new(
        format!("debug.product.agent_computer_output_wait_c{concurrency}_ms"),
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        p95_latency_ms,
    )
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "product_path_output_wait")
    .with_static_tag("phase", "agent_output_after_container_add")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod_ready_deck")
    .with_static_tag("excludes_container_add", "false")
    .with_static_tag("ready_signal", "agent_computer_ready_after_container_add")
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("concurrency_levels", concurrency_levels)
}

fn product_pod_ready_deck_add_phase_level_samples(
    concurrency: usize,
    concurrency_levels: &str,
    raw_samples: &[BenchmarkSample],
) -> Vec<BenchmarkSample> {
    const PHASES: [(&str, &str); 17] = [
        ("template_lookup", "template_lookup"),
        ("spec_build", "spec_build"),
        ("begin", "begin_container_add"),
        ("prepare", "prepare_overlay"),
        ("start", "start_container"),
        ("start_spec_build", "start_spec_build"),
        ("start_vminitd_connect", "start_vminitd_connect"),
        ("start_socket_relays", "start_socket_relays"),
        ("start_stdio_prepare", "start_stdio_prepare"),
        ("start_config_write_rpc", "start_config_write_rpc"),
        ("start_request_encode", "start_request_encode"),
        ("start_create_process_rpc", "start_create_process_rpc"),
        ("start_gate_wait", "start_gate_wait"),
        ("start_process_rpc", "start_process_rpc"),
        ("start_total", "start_total"),
        ("commit", "commit_container_add"),
        ("total", "total"),
    ];
    PHASES
        .into_iter()
        .filter_map(|(raw_phase, metric_phase)| {
            let p95_latency_ms = raw_samples
                .iter()
                .filter(|sample| sample.tag_value("phase") == Some(raw_phase))
                .map(BenchmarkSample::value)
                .max_by(f64::total_cmp)?;
            Some(
                BenchmarkSample::new(
                    format!(
                        "debug.product.agent_computer_pod_add_{metric_phase}_c{concurrency}_ms"
                    ),
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    p95_latency_ms,
                )
                .with_static_tag("probe_surface", "browser_db_cli_readiness")
                .with_static_tag("measurement_boundary", "product_path_pod_add_phase")
                .with_static_tag("phase", metric_phase)
                .with_static_tag("cli_boundary", "real_cli")
                .with_static_tag("browser_boundary", "real_browser_sidecar")
                .with_static_tag("database_boundary", "real_db_sidecar")
                .with_static_tag("pod_surface", "product_pod_ready_deck")
                .with_static_tag("excludes_container_add", "false")
                .with_static_tag("source_boundary", "single_node_pod_container_add")
                .with_dynamic_tag("concurrency_level", concurrency.to_string())
                .with_dynamic_tag("concurrency_levels", concurrency_levels),
            )
        })
        .collect()
}

fn prestarted_agent_slot_density_level_sample(
    concurrency: usize,
    concurrency_levels: &str,
    p95_latency_ms: f64,
) -> BenchmarkSample {
    BenchmarkSample::new(
        format!("debug.product.prestarted_agent_slot_checkout_c{concurrency}_ms"),
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        p95_latency_ms,
    )
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "prestarted_slot_density_level")
    .with_static_tag("phase", "host_control_file_write")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod_ready_deck")
    .with_static_tag("slot_surface", "prestarted_agent_slot")
    .with_static_tag("excludes_container_add", "true")
    .with_static_tag("ready_signal", "request_fifo_acceptance")
    .with_static_tag("output_wait_preserved", "true")
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("concurrency_levels", concurrency_levels)
}

fn prestarted_agent_slot_output_wait_level_sample(
    concurrency: usize,
    concurrency_levels: &str,
    p95_latency_ms: f64,
) -> BenchmarkSample {
    BenchmarkSample::new(
        format!("debug.product.prestarted_agent_slot_output_wait_c{concurrency}_ms"),
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        p95_latency_ms,
    )
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "prestarted_slot_output_wait")
    .with_static_tag("phase", "slot_process_completion_after_acceptance")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod_ready_deck")
    .with_static_tag("slot_surface", "prestarted_agent_slot")
    .with_static_tag("excludes_container_add", "true")
    .with_static_tag("ready_signal", "agent_slot_ready_after_fifo_acceptance")
    .with_static_tag("checkout_wait_preserved", "true")
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("concurrency_levels", concurrency_levels)
}

async fn validate_ready_deck(
    backend: &mut LocalRuntimeBackend<firkin_single_node::AppleVzLocalRuntimeDriver>,
    pod_id: &str,
) {
    backend
        .add_pod_container(
            pod_id,
            PodContainerCreateRequest {
                name: "deck-validator".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    ready_deck_probe_command("deck-ready"),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: ready_deck_probe_mounts(),
                capture_output: true,
            },
        )
        .await
        .expect("add ready-deck validator");
    let validator_output = backend
        .wait_pod_container(pod_id, "deck-validator")
        .await
        .expect("wait ready-deck validator");
    assert_eq!(
        validator_output.exit_code,
        0,
        "ready-deck validator failed: stdout={} stderr={}",
        String::from_utf8_lossy(&validator_output.stdout),
        String::from_utf8_lossy(&validator_output.stderr)
    );
    assert_eq!(validator_output.stdout, b"deck-ready");
}

async fn validate_prestarted_agent_slots(
    backend: &mut LocalRuntimeBackend<firkin_single_node::AppleVzLocalRuntimeDriver>,
    pod_id: &str,
    slot_names: &[String],
) {
    let validator_name = format!("slot-validator-{}", uuid::Uuid::new_v4().simple());
    backend
        .add_pod_container(
            pod_id,
            PodContainerCreateRequest {
                name: validator_name.clone(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    ready_deck_prestarted_slot_validator_command(slot_names),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: ready_deck_probe_mounts(),
                capture_output: true,
            },
        )
        .await
        .expect("add prestarted slot validator");
    let validator_output = backend
        .wait_pod_container(pod_id, &validator_name)
        .await
        .expect("wait prestarted slot validator");
    assert_eq!(
        validator_output.exit_code,
        0,
        "prestarted slot validator failed: stdout={} stderr={}",
        String::from_utf8_lossy(&validator_output.stdout),
        String::from_utf8_lossy(&validator_output.stderr)
    );
    assert_eq!(validator_output.stdout, b"slots-ready");
}

async fn dispatch_ready_deck_prestarted_agent_slot(
    driver: &firkin_single_node::AppleVzLocalRuntimeDriver,
    pod_id: &str,
    slot_name: &str,
    trace: &mut EventTraceRecorder,
) {
    signal_ready_deck_prestarted_agent_slot(driver, pod_id, slot_name).await;
    trace.record(SandboxEventName::AgentComputerSandboxCreated);
    trace.record(SandboxEventName::AgentComputerProbeStart);
    wait_ready_deck_prestarted_agent_slot(driver, pod_id, slot_name, trace).await;
}

async fn signal_ready_deck_prestarted_agent_slot(
    driver: &firkin_single_node::AppleVzLocalRuntimeDriver,
    pod_id: &str,
    slot_name: &str,
) {
    driver
        .write_pod_empty_dir_file(
            pod_id,
            "db",
            &format!("requests/{slot_name}"),
            b"go\n".to_vec(),
        )
        .await
        .expect("dispatch named prestarted agent slot");
}

async fn signal_ready_deck_prestarted_agent_slots(
    driver: &firkin_single_node::AppleVzLocalRuntimeDriver,
    pod_id: &str,
    request_count: usize,
) {
    let request_payload = "go\n".repeat(request_count).into_bytes();
    driver
        .write_pod_empty_dir_file(pod_id, "db", "requests/agent-slot-queue", request_payload)
        .await
        .expect("dispatch prestarted agent slot");
}

async fn wait_ready_deck_prestarted_agent_slot(
    driver: &firkin_single_node::AppleVzLocalRuntimeDriver,
    pod_id: &str,
    slot_name: &str,
    trace: &mut EventTraceRecorder,
) {
    let output = driver
        .wait_pod_container(pod_id, slot_name)
        .await
        .expect("wait prestarted agent slot");
    assert_eq!(
        output.exit_code,
        0,
        "prestarted agent slot failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"agent-slot-ready");
    trace.record(SandboxEventName::CliFirstUsefulStdout);
    trace.record(SandboxEventName::DatabaseReady);
    trace.record(SandboxEventName::BrowserReady);
    trace.record(SandboxEventName::AgentComputerReady);
}

fn ready_deck_sidecar_containers() -> Vec<PodContainerCreateRequest> {
    vec![
        PodContainerCreateRequest {
            name: "keeper".to_owned(),
            template_id: "base".to_owned(),
            command: vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                "sleep 300".to_owned(),
            ],
            env_vars: BTreeMap::new(),
            empty_dir_mounts: ready_deck_writable_mounts(),
            capture_output: false,
        },
        PodContainerCreateRequest {
            name: "db".to_owned(),
            template_id: "base".to_owned(),
            command: vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                db_sidecar_readiness_command(),
            ],
            env_vars: BTreeMap::new(),
            empty_dir_mounts: vec![PodVolumeMountRequest {
                name: "db".to_owned(),
                path: "/db".to_owned(),
                read_only: false,
            }],
            capture_output: false,
        },
        PodContainerCreateRequest {
            name: "browser".to_owned(),
            template_id: "base".to_owned(),
            command: vec![
                "/bin/sh".to_owned(),
                "-c".to_owned(),
                browser_sidecar_readiness_command(),
            ],
            env_vars: BTreeMap::new(),
            empty_dir_mounts: vec![PodVolumeMountRequest {
                name: "browser".to_owned(),
                path: "/browser".to_owned(),
                read_only: false,
            }],
            capture_output: false,
        },
    ]
}

fn ready_deck_agent_container(name: &str) -> PodContainerCreateRequest {
    PodContainerCreateRequest {
        name: name.to_owned(),
        template_id: "base".to_owned(),
        command: vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            ready_deck_agent_probe_command(),
        ],
        env_vars: BTreeMap::new(),
        empty_dir_mounts: ready_deck_probe_mounts(),
        capture_output: true,
    }
}

fn ready_deck_prestarted_agent_slot_container(name: &str) -> PodContainerCreateRequest {
    PodContainerCreateRequest {
        name: name.to_owned(),
        template_id: "base".to_owned(),
        command: vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            ready_deck_prestarted_agent_slot_command(name),
        ],
        env_vars: BTreeMap::new(),
        empty_dir_mounts: ready_deck_writable_mounts(),
        capture_output: true,
    }
}

fn ready_deck_writable_mounts() -> Vec<PodVolumeMountRequest> {
    vec![
        PodVolumeMountRequest {
            name: "db".to_owned(),
            path: "/db".to_owned(),
            read_only: false,
        },
        PodVolumeMountRequest {
            name: "browser".to_owned(),
            path: "/browser".to_owned(),
            read_only: false,
        },
    ]
}

fn ready_deck_probe_mounts() -> Vec<PodVolumeMountRequest> {
    ready_deck_writable_mounts()
        .into_iter()
        .map(|mut mount| {
            mount.read_only = true;
            mount
        })
        .collect()
}

fn ready_deck_agent_probe_command() -> String {
    ready_deck_probe_command("agent-ready")
}

fn ready_deck_prestarted_agent_slot_command(slot_name: &str) -> String {
    format!(
        r#"slot={slot_name:?}
queue=/db/requests/agent-slot-queue
private="/db/requests/$slot"
signal="/db/slots/$slot.go"
i=0
while [ "$i" -lt 600 ]; do
  browser="$(cat /browser/heartbeat 2>/dev/null || true)"
  db="$(cat /db/heartbeat 2>/dev/null || true)"
  if [ "$browser" = "browser-ready" ] && [ "$db" = "db-ready" ]; then
    mkdir -p /db/slots
    mkdir -p /db/requests
    if [ ! -p "$queue" ]; then
      mkfifo "$queue" 2>/dev/null || [ -p "$queue" ] || exit 1
    fi
    rm -f "$private"
    mkfifo "$private"
    printf ready > "/db/slots/$slot.ready"
    break
  fi
  i=$((i + 1))
  sleep 0.005
done
if [ ! -f "/db/slots/$slot.ready" ]; then
  printf 'slot never reached ready-deck readiness' >&2
  exit 1
fi
(
  IFS= read -r _ < "$private"
  printf go > "$signal"
) &
private_pid=$!
(
  IFS= read -r _ < "$queue"
  printf go > "$signal"
) &
queue_pid=$!
while [ ! -f "$signal" ]; do
  sleep 0.001
done
kill "$private_pid" "$queue_pid" 2>/dev/null || true
wait "$private_pid" "$queue_pid" 2>/dev/null || true
printf 'agent-slot-ready'"#
    )
}

fn ready_deck_prestarted_slot_validator_command(slot_names: &[String]) -> String {
    let quoted_slots = slot_names
        .iter()
        .map(|slot| format!("{slot:?}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        r#"i=0
while [ "$i" -lt 600 ]; do
  missing=0
  for slot in {quoted_slots}; do
    if [ ! -f "/db/slots/$slot.ready" ]; then
      missing=1
    fi
  done
  if [ "$missing" = "0" ]; then
    printf 'slots-ready'
    exit 0
  fi
  i=$((i + 1))
  sleep 0.005
done
printf 'missing prestarted slot readiness: ' >&2
ls -A /db/slots 2>/dev/null >&2 || true
exit 1"#
    )
}

fn ready_deck_probe_command(success_text: &str) -> String {
    format!(
        r#"i=0
while [ "$i" -lt 600 ]; do
  browser="$(cat /browser/heartbeat 2>/dev/null || true)"
  db="$(cat /db/heartbeat 2>/dev/null || true)"
  if [ "$browser" = "browser-ready" ] && [ "$db" = "db-ready" ]; then
    printf {success_text:?}
    exit 0
  fi
  i=$((i + 1))
  sleep 0.005
done
printf 'missing ready-deck heartbeat: browser_entries=' >&2
ls -A /browser 2>/dev/null >&2 || true
printf ' db_entries=' >&2
ls -A /db 2>/dev/null >&2 || true
exit 1"#
    )
}

fn real_product_pod_ready_deck_sample(trace: &SandboxEventTrace) -> BenchmarkSample {
    firkin_evidence::derive_product_autoscale_metric_sample(
        trace,
        ProductAutoscaleDurationMetric::AgentComputerResume,
    )
    .expect("derive real ready-deck product-pod sample")
    .into_benchmark_sample()
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "product_path")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar")
    .with_static_tag("pod_surface", "product_pod_ready_deck")
    .with_static_tag("slot_surface", "prestarted_agent_slot")
    .with_static_tag("excludes_container_add", "true")
    .with_static_tag("ready_signal", "request_fifo_acceptance")
    .with_static_tag("output_wait_preserved", "true")
}

fn retained_shell_density_sample(
    metric: impl Into<String>,
    concurrency: usize,
    repeat: usize,
    repeat_count: usize,
    observation: RetainedShellDispatchObservation,
) -> BenchmarkSample {
    let mut sample = BenchmarkSample::new(
        metric,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        observation.first_stdout.as_secs_f64() * 1000.0,
    )
    .with_static_tag("measurement_boundary", "retained_shell_cli_density")
    .with_static_tag("shell_mode", "prestarted_stdin_reused")
    .with_static_tag("warmup_dispatch", "excluded")
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("repeat_index", repeat.to_string())
    .with_dynamic_tag("repeat_count", repeat_count.to_string())
    .with_dynamic_tag("connect_polls_max", observation.connect_polls.to_string())
    .with_dynamic_tag("dispatch_transport", observation.dispatch_transport)
    .with_dynamic_tag(
        "send_stdin_ms",
        format!("{:.6}", observation.send_stdin.as_secs_f64() * 1000.0),
    )
    .with_dynamic_tag(
        "output_wait_ms",
        format!("{:.6}", observation.output_wait.as_secs_f64() * 1000.0),
    );
    if let Some(runtime_stdin_write_max) = observation.runtime_stdin_write_max {
        sample = sample.with_dynamic_tag(
            "runtime_stdin_write_max_ms",
            format!("{:.6}", runtime_stdin_write_max.as_secs_f64() * 1000.0),
        );
    }
    sample
}

async fn collect_retained_shell_density_reused_samples(
    adapter: &firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher>,
    sandbox: &e2b_sdk::Sandbox,
    density_levels: &[usize],
    repeats: usize,
) -> Vec<BenchmarkSample> {
    let density_level_tag = density_level_tag(density_levels);
    let mut samples = Vec::with_capacity(density_levels.len() * repeats);
    for concurrency in density_levels {
        let mut shells = Vec::with_capacity(*concurrency);
        for _ in 0..*concurrency {
            shells.push(start_retained_eval_shell(sandbox).await);
        }
        let warmups = join_all(shells.iter_mut().enumerate().map(|(index, shell)| {
            let command = format!("printf retained-density-warmup-c{concurrency}-{index}");
            async move { dispatch_retained_shell_first_stdout(sandbox, shell, &command).await }
        }))
        .await;
        assert_eq!(warmups.len(), *concurrency);
        for repeat in 0..repeats {
            let runtime_sample_offset = adapter.benchmark_samples().await.len();
            let dispatches =
                join_all(
                    shells.iter_mut().enumerate().map(|(index, shell)| {
                        let command =
                            format!("printf retained-density-c{concurrency}-r{repeat}-{index}");
                        async move {
                            dispatch_retained_shell_first_stdout(sandbox, shell, &command).await
                        }
                    }),
                )
                .await;
            let mut observation = dispatches
                .into_iter()
                .max_by_key(|observation| observation.first_stdout)
                .unwrap_or_else(|| panic!("retained shell density c{concurrency} had no shells"));
            observation.runtime_stdin_write_max =
                runtime_stdin_write_max_since(adapter, runtime_sample_offset).await;
            samples.push(
                retained_shell_density_sample(
                    format!("debug.exec.retained_shell_first_stdout_c{concurrency}_ms"),
                    *concurrency,
                    repeat,
                    repeats,
                    observation,
                )
                .with_dynamic_tag("concurrency_levels", density_level_tag.clone()),
            );
        }
        for shell in shells {
            assert!(
                sandbox
                    .commands()
                    .kill(shell.pid, e2b_sdk::CommandRequestOpts::default())
                    .await
                    .unwrap()
            );
        }
    }
    samples
}

async fn collect_retained_shell_send_path_samples(
    sandbox: &e2b_sdk::Sandbox,
    client: &reqwest::Client,
    proxy_url: &str,
    envd_url: &str,
    repeats: usize,
    density_levels: &[usize],
) -> Vec<BenchmarkSample> {
    let mut shell = start_retained_eval_shell(sandbox).await;
    let warmup = format!("printf retained-send-path-warmup-{}", uuid::Uuid::new_v4());
    let _ = dispatch_retained_shell_first_stdout(sandbox, &mut shell, &warmup).await;
    let mut samples = Vec::with_capacity(repeats * 6 * (density_levels.len() + 1));
    for repeat in 0..repeats {
        let direct_command = format!("printf retained-send-direct-r{repeat}");
        let direct = dispatch_retained_shell_first_stdout_with_send(
            &mut shell,
            &direct_command,
            |pid, data| {
                let client = client.clone();
                let envd_url = envd_url.to_owned();
                async move {
                    timed_live_raw_envd_direct_send_input(&client, &envd_url, pid, data).await;
                }
            },
        )
        .await;
        samples.extend(retained_shell_send_path_samples(
            "direct_envd",
            repeat,
            repeats,
            direct,
        ));

        let proxy_command = format!("printf retained-send-proxy-r{repeat}");
        let proxy = dispatch_retained_shell_first_stdout_with_send(
            &mut shell,
            &proxy_command,
            |pid, data| {
                let client = client.clone();
                let proxy_url = proxy_url.to_owned();
                let sandbox_id = sandbox.sandbox_id().to_owned();
                async move {
                    timed_live_raw_envd_proxy_send_input(
                        &client,
                        &proxy_url,
                        &sandbox_id,
                        pid,
                        data,
                    )
                    .await;
                }
            },
        )
        .await;
        samples.extend(retained_shell_send_path_samples(
            "domain_proxy",
            repeat,
            repeats,
            proxy,
        ));
    }
    assert!(
        sandbox
            .commands()
            .kill(shell.pid, e2b_sdk::CommandRequestOpts::default())
            .await
            .unwrap()
    );
    samples.extend(
        collect_retained_shell_send_path_fanout_samples(
            sandbox,
            client,
            proxy_url,
            envd_url,
            repeats,
            density_levels,
        )
        .await,
    );
    samples
}

async fn collect_retained_shell_send_path_fanout_samples(
    sandbox: &e2b_sdk::Sandbox,
    client: &reqwest::Client,
    proxy_url: &str,
    envd_url: &str,
    repeats: usize,
    density_levels: &[usize],
) -> Vec<BenchmarkSample> {
    let mut samples = Vec::with_capacity(repeats * density_levels.len() * 6);
    for path in ["direct_envd", "domain_proxy"] {
        for concurrency in density_levels {
            let mut shells = Vec::with_capacity(*concurrency);
            for _ in 0..*concurrency {
                shells.push(start_retained_eval_shell(sandbox).await);
            }
            let warmups =
                join_all(
                    shells.iter_mut().enumerate().map(|(index, shell)| {
                        let command =
                            format!("printf retained-send-path-fanout-warmup-{path}-{index}");
                        async move {
                            dispatch_retained_shell_first_stdout(sandbox, shell, &command).await
                        }
                    }),
                )
                .await;
            assert_eq!(warmups.len(), *concurrency);
            for repeat in 0..repeats {
                let dispatches = join_all(shells.iter_mut().enumerate().map(|(index, shell)| {
                    let command = format!(
                        "printf retained-send-path-fanout-{path}-c{concurrency}-r{repeat}-{index}"
                    );
                    async move {
                        dispatch_retained_shell_first_stdout_for_raw_send_path(
                            sandbox, client, proxy_url, envd_url, shell, &command, path,
                        )
                        .await
                    }
                }))
                .await;
                let observation = dispatches
                    .into_iter()
                    .max_by_key(|observation| observation.first_stdout)
                    .unwrap_or_else(|| {
                        panic!("retained shell send path c{concurrency} had no dispatches")
                    });
                samples.extend(retained_shell_send_path_fanout_samples(
                    path,
                    *concurrency,
                    repeat,
                    repeats,
                    density_levels,
                    observation,
                ));
            }
            for shell in shells {
                assert!(
                    sandbox
                        .commands()
                        .kill(shell.pid, e2b_sdk::CommandRequestOpts::default())
                        .await
                        .unwrap()
                );
            }
        }
    }
    samples
}

async fn dispatch_retained_shell_first_stdout_for_raw_send_path(
    sandbox: &e2b_sdk::Sandbox,
    client: &reqwest::Client,
    proxy_url: &str,
    envd_url: &str,
    shell: &mut RetainedEvalShell,
    command: &str,
    path: &str,
) -> RetainedShellSendPathObservation {
    match path {
        "direct_envd" => {
            dispatch_retained_shell_first_stdout_with_send(shell, command, |pid, data| {
                let client = client.clone();
                let envd_url = envd_url.to_owned();
                async move {
                    timed_live_raw_envd_direct_send_input(&client, &envd_url, pid, data).await;
                }
            })
            .await
        }
        "domain_proxy" => {
            dispatch_retained_shell_first_stdout_with_send(shell, command, |pid, data| {
                let client = client.clone();
                let proxy_url = proxy_url.to_owned();
                let sandbox_id = sandbox.sandbox_id().to_owned();
                async move {
                    timed_live_raw_envd_proxy_send_input(
                        &client,
                        &proxy_url,
                        &sandbox_id,
                        pid,
                        data,
                    )
                    .await;
                }
            })
            .await
        }
        _ => unreachable!("known retained shell raw send path"),
    }
}

fn retained_shell_send_path_samples(
    path: &'static str,
    repeat: usize,
    repeats: usize,
    observation: RetainedShellSendPathObservation,
) -> [BenchmarkSample; 3] {
    [
        retained_shell_send_path_sample(
            format!("debug.exec.retained_shell_send_input_{path}_ms"),
            path,
            repeat,
            repeats,
            observation.send_input,
        ),
        retained_shell_send_path_sample(
            format!("debug.exec.retained_shell_first_stdout_{path}_ms"),
            path,
            repeat,
            repeats,
            observation.first_stdout,
        ),
        retained_shell_send_path_sample(
            format!("debug.exec.retained_shell_output_wait_{path}_ms"),
            path,
            repeat,
            repeats,
            observation.output_wait,
        ),
    ]
}

fn retained_shell_send_path_fanout_samples(
    path: &str,
    concurrency: usize,
    repeat: usize,
    repeats: usize,
    density_levels: &[usize],
    observation: RetainedShellSendPathObservation,
) -> [BenchmarkSample; 3] {
    let levels = density_level_tag(density_levels);
    [
        retained_shell_send_path_sample(
            format!("debug.exec.retained_shell_send_input_{path}_c{concurrency}_ms"),
            path,
            repeat,
            repeats,
            observation.send_input,
        )
        .with_dynamic_tag("concurrency_level", concurrency.to_string())
        .with_dynamic_tag("concurrency_levels", levels.clone()),
        retained_shell_send_path_sample(
            format!("debug.exec.retained_shell_first_stdout_{path}_c{concurrency}_ms"),
            path,
            repeat,
            repeats,
            observation.first_stdout,
        )
        .with_dynamic_tag("concurrency_level", concurrency.to_string())
        .with_dynamic_tag("concurrency_levels", levels.clone()),
        retained_shell_send_path_sample(
            format!("debug.exec.retained_shell_output_wait_{path}_c{concurrency}_ms"),
            path,
            repeat,
            repeats,
            observation.output_wait,
        )
        .with_dynamic_tag("concurrency_level", concurrency.to_string())
        .with_dynamic_tag("concurrency_levels", levels),
    ]
}

fn retained_shell_send_path_sample(
    metric: String,
    path: impl Into<String>,
    repeat: usize,
    repeats: usize,
    duration: Duration,
) -> BenchmarkSample {
    BenchmarkSample::new(
        metric,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        duration.as_secs_f64() * 1000.0,
    )
    .with_static_tag(
        "measurement_boundary",
        "retained_shell_send_path_attribution",
    )
    .with_static_tag("shell_mode", "prestarted_stdin_reused")
    .with_dynamic_tag("send_path", path.into())
    .with_dynamic_tag("repeat_index", repeat.to_string())
    .with_dynamic_tag("repeat_count", repeats.to_string())
}

async fn collect_direct_exec_first_stdout_samples(
    adapter: &firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher>,
    repeats: usize,
) -> Vec<BenchmarkSample> {
    for repeat in 0..repeats {
        let payload = format!("direct-exec-{repeat}");
        let output = adapter
            .start_process(EnvdProcessStartRequest {
                cmd: "/usr/bin/printf".to_owned(),
                args: vec![payload.clone()],
                ..EnvdProcessStartRequest::default()
            })
            .await
            .expect("direct exec proof command starts");
        assert_eq!(output.exit_code, 0);
        assert_eq!(output.stdout, payload.as_bytes());
    }
    let mut samples = adapter
        .benchmark_samples()
        .await
        .into_iter()
        .filter(|sample| {
            matches!(
                sample.metric(),
                "exec.direct_command_start_ms" | "exec.direct_first_stdout_byte_ms"
            )
        })
        .filter(|sample| {
            sample
                .tag_value("args")
                .is_some_and(|args| args.starts_with("direct-exec-"))
        })
        .map(|sample| {
            sample
                .with_static_tag("measurement_boundary", "direct_envd_adapter")
                .with_static_tag("cmd", "/usr/bin/printf")
                .with_dynamic_tag("repeat_count", repeats.to_string())
        })
        .collect::<Vec<_>>();
    samples.sort_by_key(|sample| sample.tag_value("args").map(str::to_owned));
    samples
        .into_iter()
        .enumerate()
        .map(|(repeat, sample)| sample.with_dynamic_tag("repeat_index", repeat.to_string()))
        .collect()
}

async fn collect_live_hot_to_first_stdout_samples(repeats: usize) -> Vec<BenchmarkSample> {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-hot-to-first-stdout";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let ready_templates = backend.templates().latest_prepared_templates();

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let mut samples = Vec::with_capacity(repeats);
    for repeat in 0..repeats {
        let warm_targets = warm_template_targets_for_depth(ready_templates.clone(), 1);
        FirkinWarmTemplateMaintainer::new(adapter.clone(), warm_targets, Duration::from_secs(1))
            .run_cycle()
            .await
            .expect("prewarm hot-to-first-stdout target");

        let trace_offset = adapter.benchmark_event_traces().await.len();
        let sandbox_id = format!("sbx_firkin_hot_{repeat}");
        let sandbox =
            create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, &sandbox_id)).await;
        let payload = format!("hot-proof-{repeat}");
        let result = sandbox
            .commands()
            .run(
                format!("printf {payload}"),
                e2b_sdk::CommandRunOpts::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, payload);
        assert!(sandbox.kill().await.unwrap());

        let traces = adapter.benchmark_event_traces().await;
        let mut repeat_samples = firkin_evidence::derive_available_contract_metric_samples(
            traces.into_iter().skip(trace_offset),
        )
        .into_iter()
        .filter(|sample| sample.metric() == "start.hot_to_first_stdout_ms")
        .map(|sample| {
            sample
                .with_dynamic_tag("repeat_index", repeat.to_string())
                .with_dynamic_tag("repeat_count", repeats.to_string())
        })
        .collect::<Vec<_>>();
        assert_eq!(
            repeat_samples.len(),
            1,
            "expected one hot-to-first-stdout sample for repeat {repeat}"
        );
        samples.push(repeat_samples.remove(0));
    }

    proxy_task.abort();
    control_task.abort();
    samples
}

async fn collect_retained_shell_batch_100_sample(
    sandbox: &e2b_sdk::Sandbox,
    repeat: usize,
) -> (BenchmarkSample, SandboxEventTrace) {
    let mut trace = hot_batch_100_exec_trace();
    trace.record(SandboxEventName::ExecRequestSent);
    let pid = start_retained_stdin_command(
        sandbox,
        "while IFS= read -r line; do printf '%s\n' \"$line\"; done",
    )
    .await;
    trace.record(SandboxEventName::ProcessStarted);
    let mut input = String::new();
    let mut expected = String::new();
    for index in 0..100 {
        let line = format!("batch-{repeat}-{index}\n");
        input.push_str(&line);
        expected.push_str(&line);
    }
    finish_retained_stdin_command(sandbox, pid, input.as_bytes(), &expected).await;
    trace.record(SandboxEventName::ProcessExited);
    let event_trace = trace.finish();
    let sample = firkin_evidence::derive_available_contract_metric_samples([event_trace.clone()])
        .into_iter()
        .find(|sample| sample.metric() == "exec.batch_100_small_commands_ms")
        .expect("derive retained-shell batch sample")
        .with_static_tag("batch_mode", "retained_stdin_shell");
    (sample, event_trace)
}

fn write_live_db_sidecar_readiness_artifact(
    artifact: &Path,
    sample: BenchmarkSample,
    trace: SandboxEventTrace,
) {
    let file = std::fs::File::create(artifact).expect("create DB sidecar readiness artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_db_sidecar_readiness",
            "samples": [sample],
            "traces": [trace],
        }),
    )
    .expect("write DB sidecar readiness artifact");
}

fn write_live_browser_sidecar_readiness_artifact(
    artifact: &Path,
    sample: BenchmarkSample,
    trace: SandboxEventTrace,
) {
    let file = std::fs::File::create(artifact).expect("create browser sidecar readiness artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_browser_sidecar_readiness",
            "samples": [sample],
            "traces": [trace],
        }),
    )
    .expect("write browser sidecar readiness artifact");
}

fn write_live_product_pod_readiness_artifact(
    artifact: &Path,
    sample: BenchmarkSample,
    trace: SandboxEventTrace,
) {
    let file = std::fs::File::create(artifact).expect("create product-pod readiness artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_product_pod_readiness",
            "samples": [sample],
            "traces": [trace],
        }),
    )
    .expect("write product-pod readiness artifact");
}

fn write_live_product_pod_ready_deck_artifact(
    artifact: &Path,
    samples: Vec<BenchmarkSample>,
    traces: Vec<SandboxEventTrace>,
) {
    let confidence = PercentileAvailability::for_sample_count(samples.len()).as_str();
    let samples = samples
        .into_iter()
        .map(|sample| sample.with_static_tag("confidence", confidence))
        .collect::<Vec<_>>();
    let file = std::fs::File::create(artifact).expect("create ready-deck product-pod artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_product_pod_ready_deck",
            "samples": samples,
            "traces": traces,
        }),
    )
    .expect("write ready-deck product-pod artifact");
}

fn write_live_product_pod_ready_deck_density_artifact(
    artifact: &Path,
    samples: Vec<BenchmarkSample>,
    traces: Vec<SandboxEventTrace>,
) {
    let confidence = PercentileAvailability::for_sample_count(samples.len()).as_str();
    let samples = samples
        .into_iter()
        .map(|sample| sample.with_static_tag("confidence", confidence))
        .collect::<Vec<_>>();
    let file = std::fs::File::create(artifact).expect("create ready-deck density artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_product_pod_ready_deck_density",
            "samples": samples,
            "traces": traces,
        }),
    )
    .expect("write ready-deck density artifact");
}

fn write_live_product_pod_prestarted_agent_slot_density_artifact(
    artifact: &Path,
    samples: Vec<BenchmarkSample>,
    traces: Vec<SandboxEventTrace>,
) {
    let confidence = PercentileAvailability::for_sample_count(samples.len()).as_str();
    let samples = samples
        .into_iter()
        .map(|sample| sample.with_static_tag("confidence", confidence))
        .collect::<Vec<_>>();
    let file =
        std::fs::File::create(artifact).expect("create prestarted agent-slot density artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_product_pod_prestarted_agent_slot_density",
            "samples": samples,
            "traces": traces,
        }),
    )
    .expect("write prestarted agent-slot density artifact");
}

fn write_live_retained_batch_artifact(
    artifact: &Path,
    samples: Vec<BenchmarkSample>,
    traces: Vec<SandboxEventTrace>,
) {
    let confidence = PercentileAvailability::for_sample_count(samples.len()).as_str();
    let samples = samples
        .into_iter()
        .map(|sample| sample.with_static_tag("confidence", confidence))
        .collect::<Vec<_>>();
    let file = std::fs::File::create(artifact).expect("create retained batch artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_retained_shell_batch_100",
            "samples": samples,
            "traces": traces,
        }),
    )
    .expect("write retained batch artifact");
}

fn write_live_raw_sample_artifact(artifact: &Path, kind: &str, samples: Vec<BenchmarkSample>) {
    let mut metric_counts = HashMap::<String, usize>::new();
    for sample in &samples {
        *metric_counts.entry(sample.metric().to_owned()).or_default() += 1;
    }
    let samples = samples
        .into_iter()
        .map(|sample| {
            let confidence =
                PercentileAvailability::for_sample_count(metric_counts[sample.metric()]).as_str();
            sample.with_static_tag("confidence", confidence)
        })
        .collect::<Vec<_>>();
    let file = std::fs::File::create(artifact).expect("create raw live sample artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": kind,
            "samples": samples,
        }),
    )
    .expect("write raw live sample artifact");
}

fn tag_live_repeat_samples(
    samples: Vec<BenchmarkSample>,
    repeat: usize,
    repeats: usize,
) -> impl Iterator<Item = BenchmarkSample> {
    samples.into_iter().map(move |sample| {
        sample
            .with_dynamic_tag("repeat_index", repeat.to_string())
            .with_dynamic_tag("repeat_count", repeats.to_string())
    })
}

fn write_live_retained_shell_density_artifact(
    artifact: &Path,
    mut samples: Vec<BenchmarkSample>,
    traces: Vec<SandboxEventTrace>,
) {
    if let Some(sample) = retained_shell_density_breakpoint_sample(&samples) {
        samples.push(sample);
    }
    let mut metric_counts = HashMap::<String, usize>::new();
    for sample in &samples {
        *metric_counts.entry(sample.metric().to_owned()).or_default() += 1;
    }
    let samples = samples
        .into_iter()
        .map(|sample| {
            let confidence =
                PercentileAvailability::for_sample_count(metric_counts[sample.metric()]).as_str();
            sample.with_static_tag("confidence", confidence)
        })
        .collect::<Vec<_>>();
    let file = std::fs::File::create(artifact).expect("create retained shell density artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_retained_shell_density",
            "samples": samples,
            "traces": traces,
        }),
    )
    .expect("write retained shell density artifact");
}

fn retained_shell_density_breakpoint_sample(
    samples: &[BenchmarkSample],
) -> Option<BenchmarkSample> {
    let mut samples_by_metric = BTreeMap::<&str, Vec<BenchmarkSample>>::new();
    let mut concurrency_levels = None;
    for sample in samples {
        if !sample
            .metric()
            .starts_with("debug.exec.retained_shell_first_stdout_c")
        {
            continue;
        }
        concurrency_levels = concurrency_levels.or_else(|| {
            sample
                .tag_value("concurrency_levels")
                .map(ToOwned::to_owned)
        });
        samples_by_metric
            .entry(sample.metric())
            .or_default()
            .push(sample.clone());
    }

    let mut points = Vec::with_capacity(samples_by_metric.len());
    let mut min_samples_per_level = usize::MAX;
    for (metric, metric_samples) in samples_by_metric {
        min_samples_per_level = min_samples_per_level.min(metric_samples.len());
        let concurrency = metric_samples
            .first()
            .and_then(|sample| sample.tag_value("concurrency_level"))
            .and_then(|value| value.parse::<u64>().ok())?;
        let summary = BenchmarkSummary::from_samples(metric, metric_samples).ok()?;
        points.push(DensityP95Point::new(concurrency, summary.p95()));
    }

    let fallback_concurrency_levels = points
        .iter()
        .map(|point| point.concurrency().to_string())
        .collect::<Vec<_>>()
        .join(",");
    let density_limit = max_active_before_p95_doubles(points).ok()?;
    let baseline_p95_latency_ms = density_limit.baseline_p95_latency_ms();
    let threshold_p95_latency_ms = density_limit.threshold_p95_latency_ms();
    let sample = density_limit
        .into_retained_shell_sample()
        .with_static_tag("measurement_boundary", "retained_shell_cli_density")
        .with_static_tag("shell_mode", "prestarted_stdin_reused")
        .with_dynamic_tag(
            "concurrency_levels",
            concurrency_levels.unwrap_or(fallback_concurrency_levels),
        )
        .with_dynamic_tag(
            "underlying_samples_per_level_min",
            min_samples_per_level.to_string(),
        )
        .with_dynamic_tag("baseline_p95_ms", format!("{baseline_p95_latency_ms:.6}"))
        .with_dynamic_tag("threshold_p95_ms", format!("{threshold_p95_latency_ms:.6}"));
    Some(sample)
}

fn write_live_direct_exec_first_stdout_artifact(artifact: &Path, samples: Vec<BenchmarkSample>) {
    let mut metric_counts = HashMap::<String, usize>::new();
    for sample in &samples {
        *metric_counts.entry(sample.metric().to_owned()).or_default() += 1;
    }
    let samples = samples
        .into_iter()
        .map(|sample| {
            let confidence =
                PercentileAvailability::for_sample_count(metric_counts[sample.metric()]).as_str();
            sample.with_static_tag("confidence", confidence)
        })
        .collect::<Vec<_>>();
    let file = std::fs::File::create(artifact).expect("create direct exec artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_direct_exec_first_stdout",
            "samples": samples,
        }),
    )
    .expect("write direct exec artifact");
}

fn write_live_product_pod_disk_reclaim_artifact(artifact: &Path, samples: Vec<BenchmarkSample>) {
    let mut metric_counts = HashMap::<String, usize>::new();
    for sample in &samples {
        *metric_counts.entry(sample.metric().to_owned()).or_default() += 1;
    }
    let samples = samples
        .into_iter()
        .map(|sample| {
            let confidence =
                PercentileAvailability::for_sample_count(metric_counts[sample.metric()]).as_str();
            sample.with_static_tag("confidence", confidence)
        })
        .collect::<Vec<_>>();
    let file = std::fs::File::create(artifact).expect("create disk reclaim artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_product_pod_disk_reclaim",
            "samples": samples,
        }),
    )
    .expect("write disk reclaim artifact");
}

fn live_autoscale_harness_samples(
    observation: LiveAutoscaleHarnessObservation,
) -> Vec<BenchmarkSample> {
    let mut samples = Vec::new();
    samples.push(live_autoscale_safe_spare_sample(observation));
    samples.push(live_autoscale_ready_queue_sample(
        observation.ready_queue_outcomes(),
    ));
    samples
}

fn live_autoscale_safe_spare_sample(
    observation: LiveAutoscaleHarnessObservation,
) -> BenchmarkSample {
    let root = std::env::current_dir().expect("current dir for host disk accounting");
    let total = AutoscaleResourceBudget::new(
        host_logical_cpu_count(),
        Size::bytes(host_memory_bytes()),
        Size::bytes(host_disk_total_bytes(&root)),
    );
    let active = observation.active_budget();
    let reserved_floor = AutoscaleResourceBudget::new(0, Size::bytes(0), Size::gib(10));
    let ready_queue = observation.ready_queue_budget();
    SafeSpareResourceSnapshot::new(total, active, reserved_floor, ready_queue)
        .limiting_utilization()
        .expect("signed-live safe-spare utilization")
        .into_sample()
        .with_static_tag("measurement_boundary", "signed_live_resource_accounting")
        .with_static_tag("total_resource_source", "host_capacity_probe")
        .with_static_tag(
            "active_resource_source",
            "runtime_active_pod_registry_budget",
        )
        .with_static_tag("reserved_floor_source", "runtime_reserve_floor_config")
        .with_static_tag(
            "ready_queue_resource_source",
            "observed_ready_queue_capacity_budget",
        )
        .with_static_tag(
            "resource_accounting_scope",
            "agent_computer_scorecard_harness_observation",
        )
        .with_dynamic_tag(
            "ready_queue_capacity",
            observation.ready_queue_capacity.to_string(),
        )
        .with_dynamic_tag(
            "ready_hits",
            observation.ready_queue_outcomes.ready_hits().to_string(),
        )
        .with_dynamic_tag("total_cpu_slots", total.cpus().to_string())
        .with_dynamic_tag("active_cpu_slots", active.cpus().to_string())
        .with_dynamic_tag("reserved_cpu_slots", reserved_floor.cpus().to_string())
        .with_dynamic_tag("ready_queue_cpu_slots", ready_queue.cpus().to_string())
        .with_dynamic_tag("total_memory_bytes", total.memory().as_bytes().to_string())
        .with_dynamic_tag(
            "ready_queue_memory_bytes",
            ready_queue.memory().as_bytes().to_string(),
        )
}

fn host_logical_cpu_count() -> u32 {
    StdCommand::new("sysctl")
        .args(["-n", "hw.logicalcpu"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|stdout| stdout.trim().parse::<u32>().ok())
        .filter(|cpus| *cpus > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1)
                .try_into()
                .unwrap_or(u32::MAX)
        })
}

fn host_memory_bytes() -> u64 {
    if let Some(bytes) = command_stdout_u64("sysctl", &["-n", "hw.memsize"]) {
        return bytes;
    }
    let pages = command_stdout_u64("getconf", &["_PHYS_PAGES"])
        .expect("host memory accounting requires hw.memsize or _PHYS_PAGES");
    let page_size = command_stdout_u64("getconf", &["PAGE_SIZE"])
        .or_else(|| command_stdout_u64("getconf", &["PAGESIZE"]))
        .expect("host memory accounting requires PAGE_SIZE");
    pages
        .checked_mul(page_size)
        .expect("host memory accounting overflow")
}

fn host_disk_total_bytes(path: &Path) -> u64 {
    let output = StdCommand::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .expect("run host disk capacity probe");
    assert!(
        output.status.success(),
        "host disk capacity probe failed: {}",
        output.status
    );
    let stdout = String::from_utf8(output.stdout).expect("df output utf8");
    stdout
        .lines()
        .skip(1)
        .find_map(|line| line.split_whitespace().nth(1))
        .expect("df total-kib field")
        .parse::<u64>()
        .expect("df total-kib parse")
        .saturating_mul(1024)
}

fn command_stdout_u64(program: &str, args: &[&str]) -> Option<u64> {
    StdCommand::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|stdout| stdout.trim().parse::<u64>().ok())
}

async fn live_autoscale_pressure_scenario_samples() -> (Vec<BenchmarkSample>, Vec<SandboxEventTrace>)
{
    let root = std::env::current_dir().expect("current dir for pressure scenario");
    let refill_scenario = prepare_live_autoscale_ready_queue_refill().await;
    let pressure_temp = tempfile::tempdir().expect("autoscale pressure tempdir");
    let pressure_file = pressure_temp.path().join("reclaim-work.bin");
    let reclaim_target_bytes = Size::mib(64).as_bytes();
    write_reclaim_work_file(&pressure_file, reclaim_target_bytes);
    let mut probe = HostDiskPressureProbe::new();
    let available_under_pressure = probe
        .available_disk(&root)
        .expect("runtime pressure scenario requires host disk probe");
    let pressure_floor = available_under_pressure + Size::mib(32);
    let pressure_guard = RuntimeDiskPressureGuard::new(&root, pressure_floor);
    let pressure = pressure_guard
        .check(&mut probe)
        .expect_err("elevated pressure floor must detect pressure");
    assert!(
        matches!(pressure, DiskPressureError::BelowMinimum { .. }),
        "pressure scenario must be caused by host free-space floor"
    );

    let mut trace = EventTraceRecorder::new(
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    );
    trace.record(SandboxEventName::PressureDetected);
    trace.record(SandboxEventName::AutoscaleDecisionMade);
    trace.record(SandboxEventName::AutoscaleActionStarted);
    std::fs::remove_file(&pressure_file).expect("remove reclaim work file");
    let restored = pressure_guard
        .check(&mut probe)
        .expect("reclaimed capacity must restore runtime pressure floor");
    assert!(
        restored.available_free() >= restored.minimum_free(),
        "reserve-floor probe must satisfy runtime floor after reclaim work"
    );
    trace.record(SandboxEventName::SafeFloorRestored);
    let ready_outcomes =
        collect_live_autoscale_ready_queue_refill(refill_scenario, &mut trace).await;
    trace.record(SandboxEventName::ReadyTargetRestored);

    let trace = trace.finish();
    let shrink =
        pressure_scenario_sample(&trace, ProductAutoscaleDurationMetric::PressureToSafeFloor)
            .with_dynamic_tag(
                "pressure_floor_bytes",
                pressure_floor.as_bytes().to_string(),
            )
            .with_static_tag("pressure_transition", "violated_to_satisfied")
            .with_static_tag("autoscale_work_observed", "capacity_reclaimed")
            .with_dynamic_tag("reclaim_work_bytes", reclaim_target_bytes.to_string())
            .with_dynamic_tag(
                "available_under_pressure_bytes",
                available_under_pressure.as_bytes().to_string(),
            )
            .with_dynamic_tag(
                "available_after_reclaim_bytes",
                restored.available_free().as_bytes().to_string(),
            )
            .with_dynamic_tag(
                "restored_floor_bytes",
                restored.minimum_free().as_bytes().to_string(),
            );
    let refill = pressure_scenario_sample(
        &trace,
        ProductAutoscaleDurationMetric::PressureClearToReadyTarget,
    )
    .with_dynamic_tag("ready_target_hits", ready_outcomes.ready_hits().to_string())
    .with_dynamic_tag("ready_target_misses", ready_outcomes.misses().to_string())
    .with_static_tag("ready_queue_transition", "drained_to_target")
    .with_static_tag("autoscale_work_observed", "ready_capacity_refilled")
    .with_static_tag("refill_setup_excluded", "true")
    .with_static_tag("ready_queue_prepared_before_pressure_clear", "true")
    .with_dynamic_tag(
        "ready_target_drained_slots",
        ready_outcomes.misses().to_string(),
    )
    .with_dynamic_tag(
        "ready_target_refilled_slots",
        ready_outcomes.ready_hits().to_string(),
    );
    let protection =
        live_autoscale_pressure_stress_protection_samples(AutoscaleProtectionCounts::new(0, 0));
    let mut samples = vec![shrink, refill];
    samples.extend(protection);
    (samples, vec![trace])
}

fn write_reclaim_work_file(path: &Path, bytes: u64) {
    let mut file = std::fs::File::create(path).expect("create reclaim work file");
    let chunk = vec![0xA5_u8; 1024 * 1024];
    let mut remaining = bytes;
    while remaining > 0 {
        let write_len = usize::try_from(remaining.min(chunk.len() as u64))
            .expect("reclaim work chunk length fits usize");
        file.write_all(&chunk[..write_len])
            .expect("write reclaim work bytes");
        remaining -= write_len as u64;
    }
    file.sync_all().expect("sync reclaim work file");
}

struct LiveAutoscaleReadyQueueRefillScenario {
    backend: LocalRuntimeBackend<firkin_single_node::AppleVzLocalRuntimeDriver>,
    control_driver: firkin_single_node::AppleVzLocalRuntimeDriver,
    pod_id: String,
    drain_slots: Vec<String>,
    _temp: tempfile::TempDir,
}

async fn prepare_live_autoscale_ready_queue_refill() -> LiveAutoscaleReadyQueueRefillScenario {
    const READY_TARGET: usize = 2;
    let temp = tempfile::tempdir().expect("autoscale refill tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!("live-autoscale-refill-{}", uuid::Uuid::new_v4().simple());
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "python:3.12-slim",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let control_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([(
                "benchmark".to_owned(),
                "autoscale-ready-queue-refill".to_owned(),
            )]),
            empty_dirs: vec![
                PodEmptyDir {
                    name: "db".to_owned(),
                },
                PodEmptyDir {
                    name: "browser".to_owned(),
                },
            ],
            pod_store: PodStoreOptions {
                size_bytes: PYTHON_PRODUCT_POD_STORE_BYTES,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: ready_deck_sidecar_containers(),
        })
        .await
        .expect("create autoscale refill product pod");
    validate_ready_deck(&mut backend, &pod_id).await;

    let drain_slots = (0..READY_TARGET)
        .map(|index| format!("autoscale-drain-slot-{index}"))
        .collect::<Vec<_>>();
    for slot_name in &drain_slots {
        backend
            .add_pod_container(
                &pod_id,
                ready_deck_prestarted_agent_slot_container(slot_name),
            )
            .await
            .expect("add drain-ready slot");
    }
    validate_prestarted_agent_slots(&mut backend, &pod_id, &drain_slots).await;

    LiveAutoscaleReadyQueueRefillScenario {
        backend,
        control_driver,
        pod_id,
        drain_slots,
        _temp: temp,
    }
}

async fn collect_live_autoscale_ready_queue_refill(
    mut scenario: LiveAutoscaleReadyQueueRefillScenario,
    trace: &mut EventTraceRecorder,
) -> ReadyQueueOutcomes {
    const READY_TARGET: usize = 2;
    for slot_name in &scenario.drain_slots {
        dispatch_ready_deck_prestarted_agent_slot(
            &scenario.control_driver,
            &scenario.pod_id,
            slot_name,
            trace,
        )
        .await;
    }

    let refill_slots = (0..READY_TARGET)
        .map(|index| format!("autoscale-refill-slot-{index}"))
        .collect::<Vec<_>>();
    for slot_name in &refill_slots {
        scenario
            .backend
            .add_pod_container(
                &scenario.pod_id,
                ready_deck_prestarted_agent_slot_container(slot_name),
            )
            .await
            .expect("add refill-ready slot");
    }
    validate_prestarted_agent_slots(&mut scenario.backend, &scenario.pod_id, &refill_slots).await;

    scenario
        .backend
        .delete_pod(&scenario.pod_id)
        .await
        .expect("delete autoscale refill product pod");
    ReadyQueueOutcomes::new(refill_slots.len() as u64, scenario.drain_slots.len() as u64)
}

fn pressure_scenario_sample(
    trace: &SandboxEventTrace,
    metric: ProductAutoscaleDurationMetric,
) -> BenchmarkSample {
    let sample = firkin_evidence::derive_product_autoscale_metric_sample(trace, metric)
        .expect("derive pressure scenario sample")
        .into_benchmark_sample()
        .with_static_tag("measurement_boundary", "signed_live_autoscale_scenario")
        .with_static_tag("scenario_scope", "host_disk_pressure_ready_queue_probe");
    match metric {
        ProductAutoscaleDurationMetric::PressureToSafeFloor => sample
            .with_static_tag("pressure_source", "runtime_pressure_signal")
            .with_static_tag("safe_floor_source", "runtime_reserve_floor_probe"),
        ProductAutoscaleDurationMetric::PressureClearToReadyTarget => sample
            .with_static_tag("pressure_clear_source", "runtime_pressure_signal")
            .with_static_tag("ready_target_source", "runtime_ready_queue_probe"),
        _ => unreachable!("pressure scenario only derives pressure metrics"),
    }
}

fn live_autoscale_pressure_stress_protection_samples(
    counts: AutoscaleProtectionCounts,
) -> [BenchmarkSample; 2] {
    counts.into_samples().map(|sample| {
        sample
            .with_static_tag("measurement_boundary", "signed_live_product_path")
            .with_static_tag("eviction_scope", "active_session_protection")
            .with_static_tag("reserve_scope", "configured_runtime_floor")
            .with_static_tag("pressure_policy", "no_pool_comfort_eviction")
            .with_static_tag("pressure_stress_observed", "true")
            .with_static_tag("protection_evidence_scope", "pressure_stress")
            .with_static_tag(
                "protection_count_source",
                "observed_pressure_scenario_completion",
            )
    })
}

fn live_autoscale_ready_queue_sample(outcomes: ReadyQueueOutcomes) -> BenchmarkSample {
    let ready_hits = outcomes.ready_hits();
    let misses = outcomes.misses();
    outcomes
        .into_sample()
        .expect("ready queue signed-live product-path sample")
        .with_static_tag("measurement_boundary", "signed_live_product_path")
        .with_static_tag("request_classification", "hot_or_resumed_ready_capacity")
        .with_static_tag("demand_source", "agent_computer_scorecard_harness")
        .with_static_tag("outcome_source", "observed_product_request_results")
        .with_dynamic_tag("ready_hits", ready_hits.to_string())
        .with_dynamic_tag("misses", misses.to_string())
}

async fn run_live_agent_computer_probes(
    client: &reqwest::Client,
    proxy_url: &str,
    sandbox_id: &str,
    cli_payload: &str,
    trace: &mut EventTraceRecorder,
) {
    trace.record(SandboxEventName::AgentComputerProbeStart);
    let probe_epoch = Instant::now();
    let trace_elapsed_at_probe_epoch = trace.elapsed();
    let cli_command = format!("printf {cli_payload}");
    let cli = async {
        let cli_body = execute_live_code_interpreter(
            client,
            proxy_url,
            sandbox_id,
            &cli_command,
            Some("bash"),
        )
        .await;
        assert!(
            cli_body.contains(&format!(r#""text":"{cli_payload}""#)),
            "{cli_body}"
        );
        trace_elapsed_at_probe_epoch + probe_epoch.elapsed()
    };
    let browser = async {
        assert_live_code_interpreter_health(client, proxy_url, sandbox_id).await;
        trace_elapsed_at_probe_epoch + probe_epoch.elapsed()
    };
    let database = async {
        let db_body = execute_live_code_interpreter(
            client,
            proxy_url,
            sandbox_id,
            "import sqlite3\nconn = sqlite3.connect('/tmp/firkin-agent-computer.db')\nconn.execute('select 1')\nprint('database-ready')",
            None,
        )
        .await;
        assert!(
            db_body.contains(r#""text":"database-ready\n""#),
            "{db_body}"
        );
        trace_elapsed_at_probe_epoch + probe_epoch.elapsed()
    };
    let (cli_elapsed, browser_elapsed, database_elapsed) = tokio::join!(cli, browser, database);
    trace.record_at_elapsed(SandboxEventName::CliFirstUsefulStdout, cli_elapsed);
    trace.record_at_elapsed(SandboxEventName::BrowserReady, browser_elapsed);
    trace.record_at_elapsed(SandboxEventName::DatabaseReady, database_elapsed);
    trace.record_at_elapsed(
        SandboxEventName::AgentComputerReady,
        cli_elapsed.max(browser_elapsed).max(database_elapsed),
    );
}

async fn assert_live_code_interpreter_health(
    client: &reqwest::Client,
    proxy_url: &str,
    sandbox_id: &str,
) {
    let response = client
        .get(format!("{proxy_url}/health"))
        .header(
            "host",
            format!("{DEFAULT_CODE_INTERPRETER_PORT}-{sandbox_id}.cube.localhost"),
        )
        .send()
        .await
        .expect("code interpreter health response");
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.expect("health json");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "code-interpreter");
    assert_eq!(body["sandboxID"], sandbox_id);
}

async fn execute_live_code_interpreter(
    client: &reqwest::Client,
    proxy_url: &str,
    sandbox_id: &str,
    code: &str,
    language: Option<&str>,
) -> String {
    let mut body = serde_json::Map::new();
    body.insert(
        "code".to_owned(),
        serde_json::Value::String(code.to_owned()),
    );
    if let Some(language) = language {
        body.insert(
            "language".to_owned(),
            serde_json::Value::String(language.to_owned()),
        );
    }
    let response = client
        .post(format!("{proxy_url}/execute"))
        .header(
            "host",
            format!("{DEFAULT_CODE_INTERPRETER_PORT}-{sandbox_id}.cube.localhost"),
        )
        .json(&serde_json::Value::Object(body))
        .send()
        .await
        .expect("code interpreter execute response");
    assert_eq!(response.status(), 200);
    response.text().await.expect("execute response body")
}

async fn create_live_agent_computer_snapshot(
    client: &reqwest::Client,
    control_url: &str,
    sandbox_id: &str,
    snapshot_name: &str,
) -> SnapshotInfo {
    let response = client
        .post(format!(
            "{control_url}{}",
            SandboxRoutes::create_snapshot(sandbox_id)
        ))
        .json(&CreateSnapshotRequest {
            name: Some(snapshot_name.to_owned()),
        })
        .send()
        .await
        .expect("create agent-computer snapshot response");
    assert_eq!(response.status(), 200);
    response.json().await.expect("snapshot info")
}

fn cleanup_live_agent_computer_snapshot_files(snapshot_id: &str) {
    let snapshot_path = live_runtime_continuation_path(snapshot_id);
    let _ = std::fs::remove_file(snapshot_path.with_extension("state.json"));
    let _ = std::fs::remove_file(
        firkin_artifacts::SnapshotArtifactManifest::sidecar_path_for_artifact(&snapshot_path),
    );
    let _ = std::fs::remove_file(
        firkin_artifacts::SnapshotArtifactIntegrity::sidecar_path_for_artifact(&snapshot_path),
    );
    let _ = std::fs::remove_file(snapshot_path);
}

async fn create_live_agent_computer_followup(
    client: &reqwest::Client,
    control_url: &str,
    snapshot_id: &str,
    sandbox_id: &str,
) -> ConnectedSandbox {
    let mut create_request = SandboxCreateRequest::default();
    create_request
        .metadata
        .insert("requested_sandbox_id".to_owned(), sandbox_id.to_owned());
    let response = client
        .post(format!("{control_url}{}", SandboxRoutes::followup()))
        .json(&FollowupSandboxCreateRequest {
            snapshot_id: snapshot_id.to_owned(),
            create_request,
        })
        .send()
        .await
        .expect("create follow-up response");
    assert_eq!(response.status(), 200);
    response.json().await.expect("connected follow-up sandbox")
}

async fn delete_live_sandbox(client: &reqwest::Client, control_url: &str, sandbox_id: &str) {
    let response = client
        .delete(format!(
            "{control_url}{}",
            SandboxRoutes::delete(sandbox_id)
        ))
        .send()
        .await
        .expect("delete sandbox response");
    assert_eq!(response.status(), 204);
}

async fn collect_live_guest_disk_core_benchmark_samples() -> Vec<BenchmarkSample> {
    let rootfs = live_arm64_python_rootfs().await;
    let builder_id = "live-benchmark-disk-core";
    let (_snapshot_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let mut backend = live_backend_with_template(adapter.clone(), &snapshot_path);
    let request = SandboxCreateRequest {
        template_id: "repo-main".to_owned(),
        ..SandboxCreateRequest::default()
    };
    let sandbox = backend.create(request).await.expect("create disk sandbox");
    let output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                guest_disk_core_benchmark_script("/tmp/firkin-disk-core-benchmark", 512, 32),
            ],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("run guest disk core benchmark");
    assert_eq!(
        output.exit_code,
        0,
        "disk benchmark failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pressure_output = adapter
        .start_process(EnvdProcessStartRequest {
            cmd: "sh".to_owned(),
            args: vec![
                "-c".to_owned(),
                signed_live_guest_io_pressure_script().to_owned(),
            ],
            ..EnvdProcessStartRequest::default()
        })
        .await
        .expect("read guest io pressure");
    backend
        .delete(&sandbox.sandbox_id)
        .await
        .expect("delete disk sandbox");

    let mut samples = GuestDiskCoreBenchmarkOutput::parse_json(output.stdout)
        .expect("parse guest disk core benchmark output")
        .into_samples();
    if pressure_output.exit_code == 0 {
        samples.extend(
            GuestIoPressure::from_emitted_json(pressure_output.stdout)
                .expect("parse signed-live guest io pressure")
                .into_samples(),
        );
    }
    samples
}

async fn collect_live_product_pod_disk_reclaim_samples() -> Vec<BenchmarkSample> {
    collect_live_product_pod_disk_reclaim_samples_for_format(PodStoreImageFormat::Raw).await
}

async fn collect_live_product_pod_disk_reclaim_samples_for_format(
    image_format: PodStoreImageFormat,
) -> Vec<BenchmarkSample> {
    let temp = tempfile::tempdir().expect("product pod disk tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!("live-benchmark-disk-{}", uuid::Uuid::new_v4().simple());
    let driver = firkin_single_node::AppleVzLocalRuntimeDriver::with_snapshot_dir(
        "busybox",
        firkin_single_node::PortRegistry::default(),
        firkin_single_node::LogStore::default(),
        &snapshot_dir,
    );
    let metrics_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(300),
            metadata: BTreeMap::from([("benchmark".to_owned(), "disk-reclaim".to_owned())]),
            empty_dirs: vec![PodEmptyDir {
                name: "work".to_owned(),
            }],
            pod_store: PodStoreOptions {
                size_bytes: 512 * 1024 * 1024,
                image_format,
                trim_policy: PodTrimPolicy::Manual,
                ..PodStoreOptions::default()
            },
            containers: vec![PodContainerCreateRequest {
                name: "writer".to_owned(),
                template_id: "base".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-lc".to_owned(),
                    "dd if=/dev/zero of=/work/reclaim.bin bs=1048576 count=64 2>/dev/null; sync; rm /work/reclaim.bin; sync"
                        .to_owned(),
                ],
                env_vars: BTreeMap::new(),
                empty_dir_mounts: vec![PodVolumeMountRequest {
                    name: "work".to_owned(),
                    path: "/work".to_owned(),
                    read_only: false,
                }],
                capture_output: false,
            }],
        })
        .await
        .expect("create product pod disk benchmark");

    let writer_output = backend
        .wait_pod_container(&pod_id, "writer")
        .await
        .expect("wait product pod disk benchmark writer");
    assert_eq!(
        writer_output.exit_code,
        0,
        "product pod disk benchmark writer failed: stdout={} stderr={}",
        String::from_utf8_lossy(&writer_output.stdout),
        String::from_utf8_lossy(&writer_output.stderr)
    );
    wait_for_pod_store_host_allocation(&metrics_driver, &pod_id).await;
    let host_allocated_before_trim_bytes = metrics_driver
        .pod_store_host_allocated_bytes(&pod_id)
        .await
        .expect("read pod-store host allocated bytes");
    let guest_used_bytes = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read pod-store guest used bytes");
    let trim_start = Instant::now();
    let trim_reclaimed_bytes = metrics_driver
        .trim_pod_store(&pod_id)
        .await
        .expect("trim product pod store");
    let trim_duration = trim_start.elapsed();
    let host_allocated_after_trim_bytes = metrics_driver
        .pod_store_host_allocated_bytes(&pod_id)
        .await
        .expect("read post-trim pod-store host allocated bytes");
    let guest_used_after_trim_bytes = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read post-trim pod-store guest used bytes");
    let host_reclaimed_bytes =
        host_allocated_before_trim_bytes.saturating_sub(host_allocated_after_trim_bytes);
    let image_format_tag = match image_format {
        PodStoreImageFormat::Raw => "raw",
        PodStoreImageFormat::Asif => "asif",
    };
    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete product pod");

    vec![
        HostGuestDiskUsageOutput::parse_json(host_guest_disk_usage_json(
            host_allocated_before_trim_bytes,
            guest_used_bytes,
        ))
        .expect("valid product pod post-delete host/guest disk usage")
        .into_sample_with_metric(SPARSE_BLOAT_AFTER_DELETE_METRIC)
        .with_static_tag("source", "product_pod_store")
        .with_static_tag("image_format", image_format_tag)
        .with_dynamic_tag(
            "host_allocated_after_delete_bytes",
            host_allocated_before_trim_bytes.to_string(),
        )
        .with_dynamic_tag(
            "guest_used_after_delete_bytes",
            guest_used_bytes.to_string(),
        ),
        HostGuestDiskUsageOutput::parse_json(host_guest_disk_usage_json(
            host_allocated_after_trim_bytes,
            guest_used_after_trim_bytes,
        ))
        .expect("valid product pod host/guest disk usage")
        .into_sample()
        .with_static_tag("source", "product_pod_store")
        .with_static_tag("image_format", image_format_tag)
        .with_dynamic_tag(
            "host_allocated_after_trim_bytes",
            host_allocated_after_trim_bytes.to_string(),
        )
        .with_dynamic_tag("guest_used_before_trim_bytes", guest_used_bytes.to_string())
        .with_dynamic_tag(
            "guest_used_after_trim_bytes",
            guest_used_after_trim_bytes.to_string(),
        ),
        BenchmarkSample::new(
            HOST_BYTES_RECLAIMED_AFTER_TRIM_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Bytes,
            host_reclaimed_bytes as f64,
        )
        .with_static_tag("source", "product_pod_store_host_delta_after_fstrim")
        .with_static_tag("image_format", image_format_tag)
        .with_dynamic_tag("host_reclaimed_bytes", host_reclaimed_bytes.to_string())
        .with_dynamic_tag("trim_duration_ms", trim_duration.as_millis().to_string())
        .with_dynamic_tag(
            "guest_reported_trim_reclaimed_bytes",
            trim_reclaimed_bytes.to_string(),
        ),
    ]
}

async fn wait_for_pod_store_host_allocation(
    driver: &firkin_single_node::AppleVzLocalRuntimeDriver,
    pod_id: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let allocated = driver
            .pod_store_host_allocated_bytes(pod_id)
            .await
            .expect("read pod-store host allocation during disk benchmark");
        if allocated >= 64 * 1024 * 1024 || Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

struct LiveSdkSandboxTiming {
    sandbox: e2b_sdk::Sandbox,
    total: Duration,
    create: Duration,
    command: Duration,
}

async fn timed_live_sdk_sandbox_first_stdout(
    config: e2b_sdk::Config,
    payload: &'static str,
) -> LiveSdkSandboxTiming {
    let started = Instant::now();
    let create_started = Instant::now();
    let sandbox = create_live_sdk_sandbox(config).await;
    let create = create_started.elapsed();
    let command_started = Instant::now();
    let result = sandbox
        .commands()
        .run(
            format!("printf {payload}"),
            e2b_sdk::CommandRunOpts::default(),
        )
        .await
        .unwrap();
    let command = command_started.elapsed();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, payload);
    LiveSdkSandboxTiming {
        sandbox,
        total: started.elapsed(),
        create,
        command,
    }
}

struct RawEnvdStartTiming {
    process_started: Duration,
    first_stdout: Duration,
}

async fn timed_live_raw_envd_proxy_start(
    client: &reqwest::Client,
    proxy_url: &str,
    sandbox_id: &str,
    payload: &'static str,
) -> RawEnvdStartTiming {
    let started = Instant::now();
    let response = client
        .post(format!("{proxy_url}/process.Process/Start"))
        .header(CONTENT_TYPE, "application/grpc-web+proto")
        .header(
            "host",
            format!("{DEFAULT_ENVD_PORT}-{sandbox_id}.cube.localhost"),
        )
        .body(grpc_web_frame(
            0,
            &encode_envd_start_request(
                "/bin/bash",
                &["-l", "-c", &format!("printf {payload}")],
                "raw-envd-proxy",
            ),
        ))
        .send()
        .await
        .expect("raw envd proxy start request");
    assert_eq!(response.status(), 200);
    read_envd_start_response_timing(response, payload.as_bytes(), started).await
}

async fn timed_live_envd_proxy_health(
    client: &reqwest::Client,
    proxy_url: &str,
    sandbox_id: &str,
) -> Duration {
    let started = Instant::now();
    let response = client
        .get(format!("{proxy_url}/health"))
        .header(
            "host",
            format!("{DEFAULT_ENVD_PORT}-{sandbox_id}.cube.localhost"),
        )
        .send()
        .await
        .expect("envd proxy health request");
    assert_eq!(response.status(), 200);
    let body = response.bytes().await.expect("envd proxy health body");
    assert_eq!(&body[..], b"ok");
    started.elapsed()
}

async fn timed_live_raw_envd_direct_start(
    client: &reqwest::Client,
    envd_url: &str,
    payload: &'static str,
) -> RawEnvdStartTiming {
    let started = Instant::now();
    let response = client
        .post(format!("{envd_url}/process.Process/Start"))
        .header(CONTENT_TYPE, "application/grpc-web+proto")
        .body(grpc_web_frame(
            0,
            &encode_envd_start_request(
                "/bin/bash",
                &["-l", "-c", &format!("printf {payload}")],
                "raw-envd-direct",
            ),
        ))
        .send()
        .await
        .expect("raw envd direct start request");
    assert_eq!(response.status(), 200);
    read_envd_start_response_timing(response, payload.as_bytes(), started).await
}

async fn timed_live_raw_envd_direct_send_input(
    client: &reqwest::Client,
    envd_url: &str,
    pid: u32,
    data: Vec<u8>,
) {
    let response = client
        .post(format!("{envd_url}/process.Process/SendInput"))
        .header(CONTENT_TYPE, "application/proto")
        .header("connect-protocol-version", "1")
        .body(encode_envd_send_input_request(pid, data))
        .send()
        .await
        .expect("raw envd direct send-input request");
    assert_eq!(response.status(), 200);
    let body = response.bytes().await.expect("raw envd send-input body");
    EnvdSendInputResponseProto::decode(body.as_ref()).expect("raw envd send-input response");
}

async fn timed_live_raw_envd_proxy_send_input(
    client: &reqwest::Client,
    proxy_url: &str,
    sandbox_id: &str,
    pid: u32,
    data: Vec<u8>,
) {
    let response = client
        .post(format!("{proxy_url}/process.Process/SendInput"))
        .header(CONTENT_TYPE, "application/proto")
        .header("connect-protocol-version", "1")
        .header(
            "host",
            format!("{DEFAULT_ENVD_PORT}-{sandbox_id}.cube.localhost"),
        )
        .body(encode_envd_send_input_request(pid, data))
        .send()
        .await
        .expect("raw envd proxy send-input request");
    assert_eq!(response.status(), 200);
    let body = response.bytes().await.expect("raw proxy send-input body");
    EnvdSendInputResponseProto::decode(body.as_ref()).expect("raw proxy send-input response");
}

async fn timed_live_envd_direct_health(client: &reqwest::Client, envd_url: &str) -> Duration {
    let started = Instant::now();
    let response = client
        .get(format!("{envd_url}/health"))
        .send()
        .await
        .expect("envd direct health request");
    assert_eq!(response.status(), 200);
    let body = response.bytes().await.expect("envd direct health body");
    assert_eq!(&body[..], b"ok");
    started.elapsed()
}

async fn read_envd_start_response_timing(
    response: reqwest::Response,
    expected_stdout: &[u8],
    started: Instant,
) -> RawEnvdStartTiming {
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut saw_start = false;
    let mut saw_end = false;
    let mut process_started = None;
    let mut first_stdout = None;
    let mut trailers = None;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("raw envd stream chunk");
        buffer.extend_from_slice(&chunk);
        loop {
            if buffer.len() < 5 {
                break;
            }
            let len = u32::from_be_bytes([buffer[1], buffer[2], buffer[3], buffer[4]]) as usize;
            if buffer.len() < 5 + len {
                break;
            }
            let flags = buffer[0];
            let frame = buffer[5..5 + len].to_vec();
            buffer.drain(..5 + len);
            if flags == 0x80 {
                trailers = Some(frame);
                continue;
            }
            assert_eq!(flags, 0);
            let event = EnvdStartResponseProto::decode(frame.as_slice())
                .expect("raw envd stream response frame")
                .event
                .and_then(|event| event.event)
                .expect("raw envd stream event");
            match event {
                envd_process_event_proto::Event::Start(EnvdStartEventProto { pid }) => {
                    assert!(pid > 0);
                    saw_start = true;
                    process_started = Some(started.elapsed());
                }
                envd_process_event_proto::Event::Data(EnvdDataEventProto {
                    output: Some(envd_data_event_proto::Output::Stdout(bytes)),
                }) => {
                    assert!(saw_start);
                    if first_stdout.is_none() {
                        first_stdout = Some(started.elapsed());
                    }
                    assert_eq!(bytes, expected_stdout);
                }
                envd_process_event_proto::Event::End(EnvdEndEventProto {
                    exit_code,
                    exited,
                    ..
                }) => {
                    assert_eq!(exit_code, 0);
                    assert!(exited);
                    saw_end = true;
                }
                _ => {}
            }
        }
    }
    assert!(buffer.is_empty());
    assert!(saw_start);
    assert!(saw_end);
    let trailer = trailers.expect("raw envd stream trailers");
    assert!(
        String::from_utf8(trailer)
            .unwrap()
            .contains("grpc-status: 0")
    );
    RawEnvdStartTiming {
        process_started: process_started.expect("raw envd stream start frame"),
        first_stdout: first_stdout.expect("raw envd stream stdout frame"),
    }
}

async fn live_envd_url_for_sandbox(
    adapter: &FirkinRuntimeAdapter<ReadyLiveLauncher>,
    sandbox_id: &str,
) -> String {
    match adapter
        .port_target(sandbox_id, DEFAULT_ENVD_PORT)
        .await
        .expect("active sandbox has envd target")
    {
        PortTarget::Tcp { host, port } => format!("http://{host}:{port}"),
        target => panic!("expected tcp envd target, got {target:?}"),
    }
}

fn warm_template_targets_for_depth(
    templates: Vec<PreparedTemplate>,
    depth: usize,
) -> Vec<PreparedTemplate> {
    templates
        .into_iter()
        .flat_map(|template| std::iter::repeat_n(template, depth))
        .collect()
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy retained stdin smoke; requires signed test harness"]
async fn live_vendored_sdk_retains_interactive_stdin_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-stdin";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let config = e2b_sdk::Config::builder()
        .api_key("local-test")
        .api_url(format!("{control_url}/"))
        .sandbox_url(format!("{proxy_url}/"))
        .sandbox_header(
            "host",
            format!("{DEFAULT_ENVD_PORT}-sbx_firkin_1.cube.localhost"),
        )
        .request_timeout(std::time::Duration::from_mins(1))
        .build()
        .unwrap();
    let sandbox = e2b_sdk::Sandbox::create_with_config(
        config,
        e2b_sdk::SandboxCreateOpts::builder()
            .template("repo-main")
            .build(),
    )
    .await
    .unwrap();

    let handle = sandbox
        .commands()
        .run_background(
            "read line; printf 'stdin:%s' \"$line\"",
            e2b_sdk::CommandRunOpts::builder()
                .stdin(true)
                .request_timeout(std::time::Duration::from_mins(1))
                .build(),
        )
        .await
        .unwrap();
    let pid = handle.pid();
    sandbox
        .commands()
        .send_stdin(
            pid,
            b"sdk-live\n".to_vec(),
            e2b_sdk::CommandRequestOpts::default(),
        )
        .await
        .unwrap();
    sandbox
        .commands()
        .close_stdin(pid, e2b_sdk::CommandRequestOpts::default())
        .await
        .unwrap();

    let mut result = None;
    for _ in 0..20 {
        let connected = sandbox
            .commands()
            .connect(pid, e2b_sdk::CommandRequestOpts::default())
            .await
            .unwrap()
            .wait()
            .await
            .unwrap();
        if connected.stdout == "stdin:sdk-live" {
            result = Some(connected);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let connected = result.expect("retained stdin output is captured");
    assert_eq!(connected.exit_code, 0);
    assert_eq!(connected.stdout, "stdin:sdk-live");
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

fn live_sdk_config(control_url: &str, proxy_url: &str, _sandbox_id: &str) -> e2b_sdk::Config {
    e2b_sdk::Config::builder()
        .api_key("local-test")
        .api_url(format!("{control_url}/"))
        .sandbox_url(format!("{proxy_url}/"))
        .request_timeout(std::time::Duration::from_mins(1))
        .build()
        .unwrap()
}

async fn create_live_sdk_sandbox(config: e2b_sdk::Config) -> e2b_sdk::Sandbox {
    e2b_sdk::Sandbox::create_with_config(
        config,
        e2b_sdk::SandboxCreateOpts::builder()
            .template("repo-main")
            .build(),
    )
    .await
    .unwrap()
}

async fn start_retained_stdin_command(sandbox: &e2b_sdk::Sandbox, command: &str) -> u32 {
    sandbox
        .commands()
        .run_background(
            command,
            e2b_sdk::CommandRunOpts::builder()
                .stdin(true)
                .request_timeout(std::time::Duration::from_mins(1))
                .build(),
        )
        .await
        .unwrap()
        .pid()
}

type RetainedShellEventStream =
    Pin<Box<dyn Stream<Item = e2b_sdk::Result<e2b_sdk::CommandEvent>> + Send + 'static>>;

struct RetainedEvalShell {
    pid: u32,
    events: RetainedShellEventStream,
}

async fn start_retained_eval_shell(sandbox: &e2b_sdk::Sandbox) -> RetainedEvalShell {
    let handle = sandbox
        .commands()
        .run_background(
            "while IFS= read -r line; do eval \"$line\"; done",
            e2b_sdk::CommandRunOpts::builder()
                .stdin(true)
                .request_timeout(std::time::Duration::from_mins(1))
                .build(),
        )
        .await
        .unwrap();
    let pid = handle.pid();
    RetainedEvalShell {
        pid,
        events: handle.into_events(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RetainedShellDispatchObservation {
    first_stdout: Duration,
    connect_polls: usize,
    send_stdin: Duration,
    output_wait: Duration,
    dispatch_transport: &'static str,
    runtime_stdin_write_max: Option<Duration>,
}

impl RetainedShellDispatchObservation {
    #[cfg(test)]
    fn from_millis_for_test(first_stdout_ms: f64, connect_polls: usize) -> Self {
        let half = Duration::from_secs_f64(first_stdout_ms / 2000.0);
        Self {
            first_stdout: Duration::from_secs_f64(first_stdout_ms / 1000.0),
            connect_polls,
            send_stdin: half,
            output_wait: half,
            dispatch_transport: "connect_snapshot",
            runtime_stdin_write_max: Some(half),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RetainedShellSendPathObservation {
    first_stdout: Duration,
    send_input: Duration,
    output_wait: Duration,
}

async fn timed_retained_shell_dispatch_first_stdout(
    sandbox: &e2b_sdk::Sandbox,
    command: &str,
) -> RetainedShellDispatchObservation {
    let mut shell = start_retained_eval_shell(sandbox).await;
    let warmup = format!("printf retained-shell-warmup-{}", uuid::Uuid::new_v4());
    let _ = dispatch_retained_shell_first_stdout(sandbox, &mut shell, &warmup).await;
    let observed = dispatch_retained_shell_first_stdout(sandbox, &mut shell, command).await;
    assert!(
        sandbox
            .commands()
            .kill(shell.pid, e2b_sdk::CommandRequestOpts::default())
            .await
            .unwrap()
    );
    observed
}

async fn dispatch_retained_shell_first_stdout_with_send<F, Fut>(
    shell: &mut RetainedEvalShell,
    command: &str,
    send: F,
) -> RetainedShellSendPathObservation
where
    F: FnOnce(u32, Vec<u8>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let started = Instant::now();
    let send_started = Instant::now();
    send(shell.pid, format!("{command}\n").into_bytes()).await;
    let send_input = send_started.elapsed();
    let expected = command
        .strip_prefix("printf ")
        .expect("retained shell send-path benchmark uses printf payload");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let output_wait_started = Instant::now();
            let event = shell
                .events
                .next()
                .await
                .expect("retained shell stream ended")
                .expect("retained shell stream event");
            let output_wait = output_wait_started.elapsed();
            match event {
                e2b_sdk::CommandEvent::Stdout(stdout) if stdout.contains(expected) => {
                    return RetainedShellSendPathObservation {
                        first_stdout: started.elapsed(),
                        send_input,
                        output_wait,
                    };
                }
                e2b_sdk::CommandEvent::Exit(result) => {
                    panic!(
                        "retained shell exited before stdout `{expected}`: status={} stdout={} stderr={}",
                        result.exit_code, result.stdout, result.stderr
                    );
                }
                e2b_sdk::CommandEvent::Stdout(_)
                | e2b_sdk::CommandEvent::Stderr(_)
                | e2b_sdk::CommandEvent::Pty(_) => {}
            }
        }
    })
    .await
    .expect("retained shell send-path dispatch timed out")
}

async fn dispatch_retained_shell_first_stdout(
    sandbox: &e2b_sdk::Sandbox,
    shell: &mut RetainedEvalShell,
    command: &str,
) -> RetainedShellDispatchObservation {
    let started = Instant::now();
    let send_started = Instant::now();
    sandbox
        .commands()
        .send_stdin(
            shell.pid,
            format!("{command}\n").into_bytes(),
            e2b_sdk::CommandRequestOpts::default(),
        )
        .await
        .unwrap();
    let send_stdin = send_started.elapsed();
    let expected = command
        .strip_prefix("printf ")
        .expect("retained shell dispatch benchmark uses printf payload");
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let output_wait_started = Instant::now();
            let event = shell
                .events
                .next()
                .await
                .expect("retained shell stream ended")
                .expect("retained shell stream event");
            let output_wait = output_wait_started.elapsed();
            match event {
                e2b_sdk::CommandEvent::Stdout(stdout) if stdout.contains(expected) => {
                    return RetainedShellDispatchObservation {
                        first_stdout: started.elapsed(),
                        connect_polls: 0,
                        send_stdin,
                        output_wait,
                        dispatch_transport: "start_stream_events",
                        runtime_stdin_write_max: None,
                    };
                }
                e2b_sdk::CommandEvent::Exit(result) => {
                    panic!(
                        "retained shell exited before stdout `{expected}`: status={} stdout={} stderr={}",
                        result.exit_code, result.stdout, result.stderr
                    );
                }
                e2b_sdk::CommandEvent::Stdout(_)
                | e2b_sdk::CommandEvent::Stderr(_)
                | e2b_sdk::CommandEvent::Pty(_) => {}
            }
        }
    })
    .await
    .expect("retained shell dispatch timed out");
    observed
}

async fn retained_shell_dispatch_density_p95(
    sandboxes: &[LiveSdkSandboxTiming],
    concurrency: usize,
) -> RetainedShellDispatchObservation {
    let mut shells = Vec::with_capacity(sandboxes.len());
    for timing in sandboxes {
        shells.push(start_retained_eval_shell(&timing.sandbox).await);
    }
    let warmups =
        join_all(sandboxes.iter().zip(shells.iter_mut()).enumerate().map(
            |(index, (timing, shell))| {
                let command = format!("printf retained-density-warmup-{index}");
                async move {
                    dispatch_retained_shell_first_stdout(&timing.sandbox, shell, &command).await
                }
            },
        ))
        .await;
    assert_eq!(warmups.len(), sandboxes.len());
    let dispatches =
        join_all(sandboxes.iter().zip(shells.iter_mut()).enumerate().map(
            |(index, (timing, shell))| {
                let command = format!("printf retained-density-{index}");
                async move {
                    dispatch_retained_shell_first_stdout(&timing.sandbox, shell, &command).await
                }
            },
        ))
        .await;
    for (timing, shell) in sandboxes.iter().zip(shells) {
        assert!(
            timing
                .sandbox
                .commands()
                .kill(shell.pid, e2b_sdk::CommandRequestOpts::default())
                .await
                .unwrap()
        );
    }
    dispatches
        .into_iter()
        .max_by_key(|observation| observation.first_stdout)
        .unwrap_or_else(|| panic!("retained shell density c{concurrency} had no dispatches"))
}

async fn runtime_stdin_write_max_since(
    adapter: &firkin_runtime::FirkinRuntimeAdapter<ReadyLiveLauncher>,
    offset: usize,
) -> Option<Duration> {
    adapter
        .benchmark_samples()
        .await
        .into_iter()
        .skip(offset)
        .filter(|sample| sample.metric() == "sandbox.exec.stdin_write_latency_ms")
        .map(|sample| Duration::from_secs_f64(sample.value() / 1000.0))
        .max()
}

async fn finish_retained_stdin_command(
    sandbox: &e2b_sdk::Sandbox,
    pid: u32,
    input: &[u8],
    expected_stdout: &str,
) {
    sandbox
        .commands()
        .send_stdin(pid, input.to_vec(), e2b_sdk::CommandRequestOpts::default())
        .await
        .unwrap();
    sandbox
        .commands()
        .close_stdin(pid, e2b_sdk::CommandRequestOpts::default())
        .await
        .unwrap();

    for _ in 0..20 {
        let connected = sandbox
            .commands()
            .connect(pid, e2b_sdk::CommandRequestOpts::default())
            .await
            .unwrap()
            .wait()
            .await
            .unwrap();
        if connected.stdout == expected_stdout {
            assert_eq!(connected.exit_code, 0);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("retained stdout did not become `{expected_stdout}`");
}

async fn assert_live_filesystem_roundtrip(
    sandbox: &e2b_sdk::Sandbox,
    path: &str,
    data: &'static [u8],
) {
    let written = sandbox
        .files()
        .write(
            path,
            data.to_vec(),
            e2b_sdk::FilesystemWriteOpts::builder()
                .use_octet_stream(true)
                .build(),
        )
        .await
        .unwrap();
    assert_eq!(written.path, path);
    assert_eq!(
        sandbox
            .files()
            .read_bytes(path, e2b_sdk::FilesystemReadOpts::default())
            .await
            .unwrap(),
        data
    );
    let info = sandbox
        .files()
        .get_info(path, e2b_sdk::FilesystemRequestOpts::default())
        .await
        .unwrap();
    assert_eq!(info.size, i64::try_from(data.len()).unwrap());
    let entries = sandbox
        .files()
        .list("/tmp", e2b_sdk::FilesystemListOpts::default())
        .await
        .unwrap();
    assert!(entries.iter().any(|entry| entry.path == path));
    sandbox
        .files()
        .remove(path, e2b_sdk::FilesystemRequestOpts::default())
        .await
        .unwrap();
    assert!(
        !sandbox
            .files()
            .exists(path, e2b_sdk::FilesystemRequestOpts::default())
            .await
            .unwrap()
    );
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy concurrent retained stdin smoke; requires signed test harness"]
async fn live_vendored_sdk_retains_concurrent_stdin_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-concurrent-stdin";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let first =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let second =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_2")).await;
    let first_pid =
        start_retained_stdin_command(&first, "read line; printf 'first:%s' \"$line\"").await;
    let second_pid =
        start_retained_stdin_command(&second, "read line; printf 'second:%s' \"$line\"").await;

    finish_retained_stdin_command(&first, first_pid, b"one\n", "first:one").await;
    finish_retained_stdin_command(&second, second_pid, b"two\n", "second:two").await;
    assert!(first.kill().await.unwrap());
    assert!(second.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy retained PTY smoke; requires signed test harness"]
async fn live_vendored_sdk_retains_pty_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-pty";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let config = e2b_sdk::Config::builder()
        .api_key("local-test")
        .api_url(format!("{control_url}/"))
        .sandbox_url(format!("{proxy_url}/"))
        .sandbox_header(
            "host",
            format!("{DEFAULT_ENVD_PORT}-sbx_firkin_1.cube.localhost"),
        )
        .request_timeout(std::time::Duration::from_mins(1))
        .build()
        .unwrap();
    let sandbox = e2b_sdk::Sandbox::create_with_config(
        config,
        e2b_sdk::SandboxCreateOpts::builder()
            .template("repo-main")
            .build(),
    )
    .await
    .unwrap();

    let handle = sandbox
        .pty()
        .create(
            e2b_sdk::PtyCreateOpts::builder(80, 24)
                .request_timeout(std::time::Duration::from_mins(1))
                .build(),
        )
        .await
        .unwrap();
    let pid = handle.pid();
    sandbox
        .pty()
        .resize(
            pid,
            e2b_sdk::PtySize {
                cols: 100,
                rows: 32,
            },
            e2b_sdk::CommandRequestOpts::default(),
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    sandbox
        .pty()
        .send_input(
            pid,
            b"printf 'pty:sdk-live'\r".to_vec(),
            e2b_sdk::CommandRequestOpts::default(),
        )
        .await
        .unwrap();

    let mut output = Vec::new();
    for _ in 0..20 {
        let mut events = sandbox
            .pty()
            .connect(pid, e2b_sdk::CommandRequestOpts::default())
            .await
            .unwrap()
            .into_events();
        while let Some(event) = events.next().await {
            match event.unwrap() {
                e2b_sdk::CommandEvent::Pty(bytes) => output.extend(bytes),
                e2b_sdk::CommandEvent::Exit(_) => break,
                _ => {}
            }
        }
        if String::from_utf8_lossy(&output).contains("pty:sdk-live") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        String::from_utf8_lossy(&output).contains("pty:sdk-live"),
        "pty output: {}",
        String::from_utf8_lossy(&output)
    );
    assert!(
        sandbox
            .pty()
            .kill(pid, e2b_sdk::CommandRequestOpts::default())
            .await
            .unwrap()
    );
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy filesystem smoke; requires signed test harness"]
async fn live_vendored_sdk_uses_filesystem_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-files";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let config = e2b_sdk::Config::builder()
        .api_key("local-test")
        .api_url(format!("{control_url}/"))
        .sandbox_url(format!("{proxy_url}/"))
        .sandbox_header(
            "host",
            format!("{DEFAULT_ENVD_PORT}-sbx_firkin_1.cube.localhost"),
        )
        .request_timeout(std::time::Duration::from_mins(1))
        .build()
        .unwrap();
    let sandbox = e2b_sdk::Sandbox::create_with_config(
        config,
        e2b_sdk::SandboxCreateOpts::builder()
            .template("repo-main")
            .build(),
    )
    .await
    .unwrap();

    let path = "/tmp/firkin-sdk-files.txt";
    let written = sandbox
        .files()
        .write(
            path,
            b"filesystem-live".to_vec(),
            e2b_sdk::FilesystemWriteOpts::builder()
                .use_octet_stream(true)
                .build(),
        )
        .await
        .unwrap();
    assert_eq!(written.path, path);
    assert_eq!(
        sandbox
            .files()
            .read_bytes(path, e2b_sdk::FilesystemReadOpts::default())
            .await
            .unwrap(),
        b"filesystem-live"
    );
    let info = sandbox
        .files()
        .get_info(path, e2b_sdk::FilesystemRequestOpts::default())
        .await
        .unwrap();
    assert_eq!(info.name, "firkin-sdk-files.txt");
    assert_eq!(info.size, 15);
    let entries = sandbox
        .files()
        .list("/tmp", e2b_sdk::FilesystemListOpts::default())
        .await
        .unwrap();
    assert!(entries.iter().any(|entry| entry.path == path));
    sandbox
        .files()
        .remove(path, e2b_sdk::FilesystemRequestOpts::default())
        .await
        .unwrap();
    assert!(
        !sandbox
            .files()
            .exists(path, e2b_sdk::FilesystemRequestOpts::default())
            .await
            .unwrap()
    );
    assert!(sandbox.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}

#[tokio::test]
#[ignore = "live VM-backed SDK/domain-proxy concurrent filesystem smoke; requires signed test harness"]
async fn live_vendored_sdk_uses_concurrent_filesystems_through_firkin_domain_proxy() {
    let rootfs = live_arm64_bash_rootfs().await;
    let builder_id = "live-sdk-concurrent-files";
    let (_temp, snapshot_path) = save_live_snapshot(rootfs.clone(), builder_id).await;
    let adapter = live_envd_adapter(rootfs, builder_id);
    let backend = live_backend_with_template(adapter, &snapshot_path);

    let control_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control_url = format!("http://{}", control_listener.local_addr().unwrap());
    let control_plane = ControlPlaneHttpServer::new(backend);
    let proxy =
        DomainProxyHttpServer::from_control_plane(&control_plane, hostname!("cube.localhost"));
    let control_task = tokio::spawn(control_plane.serve(control_listener));

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy_listener.local_addr().unwrap());
    let proxy_task = tokio::spawn(proxy.serve(proxy_listener));

    let first =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_1")).await;
    let second =
        create_live_sdk_sandbox(live_sdk_config(&control_url, &proxy_url, "sbx_firkin_2")).await;
    assert_live_filesystem_roundtrip(&first, "/tmp/firkin-first.txt", b"first-live").await;
    assert_live_filesystem_roundtrip(&second, "/tmp/firkin-second.txt", b"second-live").await;
    assert!(first.kill().await.unwrap());
    assert!(second.kill().await.unwrap());

    proxy_task.abort();
    control_task.abort();
}
