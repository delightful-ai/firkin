//! Trace recorder behavior tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use firkin_trace::{
    BenchProfile, BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit, EventTraceRecorder,
    LifecycleClass, MAX_SAMPLE_TAGS, MAX_TAG_VALUE_BYTES, Recorder, RecorderConfig, RecorderError,
    RuntimeProfile, Sampler, SandboxEventName, SandboxEventRole, SandboxEventTrace,
    SandboxTraceEvent, Tags, TraceOutcome, WorkloadClass, phase,
};
use serde_json::json;

#[test]
fn sample_json_keeps_existing_shape_and_skips_empty_tags() {
    let sample = BenchmarkSample::new(
        "command_start",
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        12.5,
    );

    let value = serde_json::to_value(&sample).unwrap();
    assert_eq!(
        value,
        json!({
            "metric": "command_start",
            "kind": "LifecycleLatency",
            "unit": "Milliseconds",
            "value": 12.5
        })
    );

    let roundtrip: BenchmarkSample = serde_json::from_value(value).unwrap();
    assert_eq!(roundtrip.metric(), "command_start");
    assert_eq!(roundtrip.tags().len(), 0);
}

#[test]
fn sample_tags_serialize_as_a_small_map() {
    let sample = BenchmarkSample::from_static(
        phase::FIRST_EXEC,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        4.0,
    )
    .with_static_tag("outcome", "ok")
    .with_dynamic_tag("sandbox_id", "sandbox-1");

    assert_eq!(sample.tag_value("outcome"), Some("ok"));
    assert_eq!(sample.tag_value("sandbox_id"), Some("sandbox-1"));
    assert_eq!(
        serde_json::to_value(&sample).unwrap()["tags"],
        json!({
            "outcome": "ok",
            "sandbox_id": "sandbox-1"
        })
    );
}

#[test]
fn span_finish_ok_records_lifecycle_latency_with_outcome_and_variant() {
    let recorder = Recorder::enabled(
        BenchProfile::Default,
        Tags::new().with_static("machine_model", "m4").unwrap(),
    );

    recorder.span(phase::VM_START).cold().finish_ok();

    let trace = recorder.drain();
    assert_eq!(trace.shared_tags.value("machine_model"), Some("m4"));
    assert_eq!(trace.samples.len(), 1);

    let sample = &trace.samples[0];
    assert_eq!(sample.metric(), phase::VM_START);
    assert_eq!(sample.kind(), BenchmarkMetricKind::LifecycleLatency);
    assert_eq!(sample.unit(), BenchmarkUnit::Milliseconds);
    assert_eq!(sample.tag_value("outcome"), Some("ok"));
    assert_eq!(sample.tag_value("phase_variant"), Some("cold"));
}

#[test]
fn unfinished_span_drop_records_cancelled_sample() {
    let recorder = Recorder::enabled(BenchProfile::Default, Tags::new());

    {
        let _span = recorder.span(phase::FIRST_STDOUT).warm();
    }

    let samples = recorder.drain().into_samples();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].metric(), phase::FIRST_STDOUT);
    assert_eq!(samples[0].tag_value("outcome"), Some("cancelled"));
    assert_eq!(samples[0].tag_value("phase_variant"), Some("warm"));
}

#[test]
fn finish_error_records_failure_class() {
    let recorder = Recorder::enabled(BenchProfile::Default, Tags::new());

    recorder
        .span(phase::AGENT_HANDSHAKE)
        .finish_error(firkin_trace::FailureClass::static_code("vsock_connect"));

    let samples = recorder.drain().into_samples();
    assert_eq!(samples[0].tag_value("outcome"), Some("error"));
    assert_eq!(samples[0].tag_value("failure_class"), Some("vsock_connect"));
}

#[test]
fn disabled_recorder_is_a_noop() {
    let recorder = Recorder::disabled();

    recorder.sample(BenchmarkSample::from_static(
        phase::VM_STOP,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        1.0,
    ));
    recorder.span(phase::VM_START).finish_ok();

    let trace = recorder.drain();
    assert!(trace.samples.is_empty());
    assert_eq!(trace.overflowed, 0);
    assert!(trace.event_traces.is_empty());
}

#[test]
fn event_trace_json_preserves_exact_host_offsets() {
    let mut trace = SandboxEventTrace::with_event_cap(8);
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::PoolLeaseAcquired,
        10_000_000,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::FirstStdoutByte,
        83_000_000,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));

    assert_eq!(
        trace
            .duration_between(
                SandboxEventName::PoolLeaseAcquired,
                SandboxEventName::FirstStdoutByte
            )
            .unwrap(),
        Duration::from_millis(73)
    );

    let value = serde_json::to_value(&trace).unwrap();
    assert_eq!(value["events"][0]["host_monotonic_ns"], 10_000_000_u64);
    assert_eq!(value["events"][1]["host_monotonic_ns"], 83_000_000_u64);
    let roundtrip: SandboxEventTrace = serde_json::from_value(value).unwrap();
    assert_eq!(roundtrip.events()[0].host_monotonic_ns(), 10_000_000);
    assert_eq!(roundtrip.events()[1].host_monotonic_ns(), 83_000_000);
}

#[test]
fn product_event_trace_anchors_agent_computer_readiness() {
    let mut trace = SandboxEventTrace::with_event_cap(8);
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::AgentComputerRequestStart,
        1_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::AgentComputerSandboxCreated,
        11_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::AgentComputerProbeStart,
        16_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::CliFirstUsefulStdout,
        21_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::BrowserReady,
        31_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::DatabaseReady,
        41_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::AgentComputerReady,
        51_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));

    assert_eq!(
        trace
            .duration_between(
                SandboxEventName::AgentComputerRequestStart,
                SandboxEventName::AgentComputerReady,
            )
            .unwrap(),
        Duration::from_millis(50)
    );
    assert_eq!(
        trace
            .duration_between(
                SandboxEventName::AgentComputerRequestStart,
                SandboxEventName::AgentComputerSandboxCreated,
            )
            .unwrap(),
        Duration::from_millis(10)
    );
    assert_eq!(
        trace
            .duration_between(
                SandboxEventName::AgentComputerProbeStart,
                SandboxEventName::AgentComputerReady,
            )
            .unwrap(),
        Duration::from_millis(35)
    );
    assert_eq!(
        trace
            .headline_event(SandboxEventName::AgentComputerReady)
            .expect("agent computer ready")
            .workload(),
        WorkloadClass::AgentComputer
    );
    assert_eq!(
        trace
            .headline_event(SandboxEventName::AgentComputerReady)
            .expect("agent computer ready")
            .profile(),
        RuntimeProfile::BrowserDbCli
    );
}

#[test]
fn event_trace_recorder_can_record_explicit_elapsed_offsets() {
    let mut recorder = EventTraceRecorder::new(
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    );
    recorder.record_at_elapsed(
        SandboxEventName::AgentComputerRequestStart,
        Duration::from_millis(1),
    );
    recorder.record_at_elapsed(
        SandboxEventName::CliFirstUsefulStdout,
        Duration::from_millis(21),
    );

    let trace = recorder.finish();

    assert_eq!(
        trace
            .duration_between(
                SandboxEventName::AgentComputerRequestStart,
                SandboxEventName::CliFirstUsefulStdout,
            )
            .unwrap(),
        Duration::from_millis(20)
    );
}

#[test]
fn autoscale_event_trace_anchors_pressure_and_refill_spans() {
    let mut trace = SandboxEventTrace::with_event_cap(8);
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::PressureDetected,
        100_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::AutoscaleDecisionMade,
        125_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::AutoscaleActionStarted,
        150_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::SafeFloorRestored,
        600_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::ReadyTargetRestored,
        1_100_000_000,
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    ));

    assert_eq!(
        trace
            .duration_between(
                SandboxEventName::PressureDetected,
                SandboxEventName::SafeFloorRestored,
            )
            .unwrap(),
        Duration::from_millis(500)
    );
    assert_eq!(
        trace
            .duration_between(
                SandboxEventName::SafeFloorRestored,
                SandboxEventName::ReadyTargetRestored,
            )
            .unwrap(),
        Duration::from_millis(500)
    );
}

#[test]
fn duplicate_successful_endpoint_is_debug_not_headline() {
    let mut trace = SandboxEventTrace::with_event_cap(8);
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::ExecRequestSent,
        1,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::FirstStdoutByte,
        5,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::FirstStdoutByte,
        9,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));

    assert_eq!(trace.events()[1].role(), SandboxEventRole::Headline);
    assert_eq!(trace.events()[2].role(), SandboxEventRole::DebugDuplicate);
    assert_eq!(
        trace
            .duration_between(
                SandboxEventName::ExecRequestSent,
                SandboxEventName::FirstStdoutByte
            )
            .unwrap(),
        Duration::from_nanos(4)
    );
}

#[test]
fn event_trace_reports_missing_endpoint_and_reversed_order() {
    let mut trace = SandboxEventTrace::with_event_cap(8);
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::FirstStdoutByte,
        20,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));

    let missing = trace
        .duration_between(
            SandboxEventName::ExecRequestSent,
            SandboxEventName::FirstStdoutByte,
        )
        .unwrap_err();
    assert_eq!(
        missing.to_string(),
        "missing sandbox event ExecRequestSent in event trace"
    );

    trace.push(SandboxTraceEvent::new(
        SandboxEventName::ExecRequestSent,
        30,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));
    let reversed = trace
        .duration_between(
            SandboxEventName::ExecRequestSent,
            SandboxEventName::FirstStdoutByte,
        )
        .unwrap_err();
    assert_eq!(
        reversed.to_string(),
        "sandbox event FirstStdoutByte occurred before ExecRequestSent"
    );
}

#[test]
fn event_trace_overflow_counts_dropped_events() {
    let mut trace = SandboxEventTrace::with_event_cap(1);
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::RequestStart,
        1,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::CleanupDone,
        2,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    ));

    assert_eq!(trace.events().len(), 1);
    assert_eq!(trace.overflowed(), 1);
}

#[test]
fn recorder_drains_raw_event_traces_without_flattening_them_into_samples() {
    let recorder = Recorder::enabled(BenchProfile::Default, Tags::new());
    let mut trace = EventTraceRecorder::new(
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    );
    trace.record(SandboxEventName::RequestStart);
    trace.record_with_outcome(
        SandboxEventName::CleanupDone,
        TraceOutcome::Error,
        Some(firkin_trace::SandboxFailureClass::GuestAgentCrash),
    );
    recorder.record_event_trace(trace.finish());

    let drained_trace = recorder.drain();
    assert!(drained_trace.samples.is_empty());
    assert_eq!(drained_trace.event_traces.len(), 1);
    assert_eq!(
        drained_trace.event_traces[0].events()[0].name(),
        SandboxEventName::RequestStart
    );
    assert_eq!(
        drained_trace.event_traces[0].events()[1].failure_class(),
        Some(firkin_trace::SandboxFailureClass::GuestAgentCrash)
    );
}

#[test]
fn shared_tags_are_flattened_only_when_requested() {
    let recorder = Recorder::enabled(
        BenchProfile::Default,
        Tags::new().with_static("chip", "m4-max").unwrap(),
    );
    recorder.sample(BenchmarkSample::from_static(
        "sandbox.exec.count",
        BenchmarkMetricKind::FirkinOverhead,
        BenchmarkUnit::Bytes,
        1.0,
    ));

    let trace = recorder.drain();
    assert_eq!(trace.clone().into_samples()[0].tag_value("chip"), None);
    assert_eq!(
        trace.into_flat_samples()[0].tag_value("chip"),
        Some("m4-max")
    );
}

#[test]
fn checkpoint_records_host_guest_pair_and_elapsed_sample() {
    let recorder = Recorder::enabled(BenchProfile::Default, Tags::new());

    recorder
        .checkpoint("fstrim")
        .record_pair(100.0, 75.0, BenchmarkUnit::Bytes);

    let samples = recorder.drain().into_samples();
    assert_eq!(samples.len(), 3);
    assert!(samples.iter().any(|sample| {
        sample.tag_value("checkpoint") == Some("fstrim")
            && sample.tag_value("side") == Some("host")
            && (sample.value() - 100.0).abs() < f64::EPSILON
    }));
    assert!(samples.iter().any(|sample| {
        sample.tag_value("checkpoint") == Some("fstrim")
            && sample.tag_value("side") == Some("guest")
            && (sample.value() - 75.0).abs() < f64::EPSILON
    }));
    assert!(samples.iter().any(|sample| {
        sample.metric() == "checkpoint.fstrim.elapsed_ms"
            && sample.unit() == BenchmarkUnit::Milliseconds
            && sample.tag_value("checkpoint") == Some("fstrim")
    }));
}

#[test]
fn sample_cap_prefers_lifecycle_samples_over_gauges() {
    let recorder = Recorder::enabled_with_config(
        BenchProfile::Default,
        Tags::new(),
        RecorderConfig { sample_cap: 2 },
    );

    recorder.sample(BenchmarkSample::from_static(
        "sandbox.mem.host_footprint_bytes",
        BenchmarkMetricKind::FirkinOverhead,
        BenchmarkUnit::Bytes,
        1.0,
    ));
    recorder.sample(BenchmarkSample::from_static(
        phase::VM_START,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        2.0,
    ));
    recorder.sample(BenchmarkSample::from_static(
        phase::FIRST_EXEC,
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        3.0,
    ));

    let trace = recorder.drain();
    assert_eq!(trace.overflowed, 1);
    assert_eq!(trace.samples.len(), 2);
    assert!(
        trace
            .samples
            .iter()
            .all(|sample| { sample.kind() == BenchmarkMetricKind::LifecycleLatency })
    );
}

#[test]
fn tag_values_have_a_hard_limit() {
    let too_long = "x".repeat(MAX_TAG_VALUE_BYTES + 1);
    let error = Tags::new()
        .with_dynamic("sandbox_id", too_long)
        .unwrap_err();

    assert_eq!(error, RecorderError::TagLimitExceeded { key: "sandbox_id" });
}

#[test]
fn sample_tags_fit_product_promotion_evidence_and_still_have_a_cap() {
    let sample = BenchmarkSample::from_static(
        "product.agent_computer_ready_ms",
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        42.0,
    )
    .with_static_tag("trust", "exact_host_event_pair")
    .with_static_tag("confidence", "exact")
    .with_static_tag("start_event", "AgentComputerRequestStart")
    .with_static_tag("end_event", "AgentComputerReady")
    .with_static_tag("probe_surface", "browser_db_cli_readiness")
    .with_static_tag("measurement_boundary", "product_path")
    .with_static_tag("cli_boundary", "real_cli")
    .with_static_tag("browser_boundary", "real_browser_sidecar")
    .with_static_tag("database_boundary", "real_db_sidecar");

    assert_eq!(sample.tags().len(), 9);
    assert_eq!(
        sample.tag_value("database_boundary"),
        Some("real_db_sidecar")
    );

    let mut raw_tags = std::collections::BTreeMap::new();
    for index in 0..=MAX_SAMPLE_TAGS {
        raw_tags.insert(format!("k{index}"), "v".to_owned());
    }
    let error = serde_json::from_value::<BenchmarkSample>(json!({
        "metric": "product.agent_computer_ready_ms",
        "kind": "LifecycleLatency",
        "unit": "Milliseconds",
        "value": 42.0,
        "tags": raw_tags,
    }))
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("too many sample tags"),
        "unexpected error: {error}"
    );
}

#[test]
fn sample_tag_update_does_not_consume_cap() {
    let sample = BenchmarkSample::from_static(
        "product.agent_computer_ready_ms",
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        42.0,
    )
    .with_static_tag("k00", "v")
    .with_static_tag("k01", "v")
    .with_static_tag("k02", "v")
    .with_static_tag("k03", "v")
    .with_static_tag("k04", "v")
    .with_static_tag("k05", "v")
    .with_static_tag("k06", "v")
    .with_static_tag("k07", "v")
    .with_static_tag("k08", "v")
    .with_static_tag("k09", "v")
    .with_static_tag("k10", "v")
    .with_static_tag("k11", "v")
    .with_static_tag("k12", "v")
    .with_static_tag("k13", "v")
    .with_static_tag("k14", "v")
    .with_static_tag("k15", "old")
    .with_static_tag("k15", "new")
    .with_static_tag("over_cap", "dropped");

    assert_eq!(sample.tags().len(), MAX_SAMPLE_TAGS);
    assert_eq!(sample.tag_value("k15"), Some("new"));
    assert_eq!(sample.tag_value("over_cap"), None);
}

#[test]
fn sampler_attach_reports_no_runtime() {
    let recorder = Recorder::enabled(BenchProfile::Default, Tags::new());
    let error = recorder
        .attach_sampler(CountingSampler::default(), Duration::from_millis(10))
        .unwrap_err();

    assert_eq!(error, RecorderError::NoRuntime);
}

#[tokio::test(start_paused = true)]
async fn close_and_drain_aborts_sampler_and_drops_late_samples() {
    let snapshots = Arc::new(AtomicUsize::new(0));
    let recorder = Recorder::enabled(BenchProfile::Default, Tags::new());
    recorder
        .attach_sampler(
            CountingSampler {
                snapshots: Arc::clone(&snapshots),
            },
            Duration::from_millis(10),
        )
        .unwrap();

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(10)).await;
    tokio::task::yield_now().await;

    let trace = recorder.close_and_drain().await;
    assert!(snapshots.load(Ordering::SeqCst) >= 1);
    assert!(
        trace
            .samples
            .iter()
            .any(|sample| sample.metric() == "sampler.count")
    );

    recorder.sample(BenchmarkSample::from_static(
        "late.sample",
        BenchmarkMetricKind::FirkinOverhead,
        BenchmarkUnit::Bytes,
        1.0,
    ));
    assert_eq!(recorder.stats().closed_drops(), 1);
}

#[derive(Clone, Default, Debug)]
struct CountingSampler {
    snapshots: Arc<AtomicUsize>,
}

#[async_trait]
impl Sampler for CountingSampler {
    fn name(&self) -> &'static str {
        "counting"
    }

    async fn snapshot(&self) -> Vec<BenchmarkSample> {
        let value = self.snapshots.fetch_add(1, Ordering::SeqCst) + 1;
        vec![BenchmarkSample::from_static(
            "sampler.count",
            BenchmarkMetricKind::FirkinOverhead,
            BenchmarkUnit::Bytes,
            f64::from(u32::try_from(value).expect("test snapshot count fits in u32")),
        )]
    }
}
