//! Benchmark evidence writer tests.

use firkin_benchmark::{
    RuntimeAgentComputerScorecardEvidenceWriter, RuntimeAgentScorecardEvidenceWriter,
    RuntimeAutoscaleScorecardEvidenceWriter, RuntimeBenchmarkEvidenceError,
    RuntimeBenchmarkEvidenceWriter, RuntimeOverheadEvidenceWriter,
};
use firkin_evidence::{
    AgentBenchmarkScorecardArtifact, AgentBenchmarkScorecardError, AgentComputerScorecardArtifact,
    AgentComputerScorecardError, AutoscaleEfficiencyScorecardArtifact,
    AutoscaleEfficiencyScorecardError, BenchmarkEvidenceArtifact, BenchmarkEvidenceError,
    BenchmarkOverheadEvidenceArtifact, REQUIRED_FIRKIN_OVERHEAD_METRICS,
    REQUIRED_LIFECYCLE_LATENCY_METRICS, required_agent_computer_metric_definitions,
    required_autoscale_efficiency_metric_definitions, required_scorecard_metric_definitions,
};
use firkin_trace::{
    BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit, LifecycleClass, RuntimeProfile,
    SandboxEventName, SandboxEventTrace, SandboxTraceEvent, WorkloadClass,
};

#[test]
fn runtime_benchmark_writer_validates_and_persists_required_lifecycle_samples() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("benchmark-evidence.json");
    let samples = REQUIRED_LIFECYCLE_LATENCY_METRICS
        .iter()
        .flat_map(|metric| {
            [10.0, 20.0].into_iter().map(move |value| {
                BenchmarkSample::new(
                    *metric,
                    BenchmarkMetricKind::LifecycleLatency,
                    BenchmarkUnit::Milliseconds,
                    value,
                )
            })
        })
        .collect::<Vec<_>>();

    let report = RuntimeBenchmarkEvidenceWriter::new(&artifact)
        .write_samples_with_traces(
            samples,
            [autoscale_trace(SandboxEventName::FirstStdoutByte)],
        )
        .expect("write benchmark artifact");
    let restored = BenchmarkEvidenceArtifact::read_json(&artifact).expect("read artifact");
    let raw_samples_path = temp.path().join("benchmark-evidence.samples.json");
    let raw_samples = std::fs::read(&raw_samples_path).expect("read raw samples sidecar");
    let raw_samples =
        serde_json::from_slice::<Vec<BenchmarkSample>>(&raw_samples).expect("raw sample json");
    let raw_traces_path = temp.path().join("benchmark-evidence.traces.json");
    let raw_traces = std::fs::read(&raw_traces_path).expect("read raw traces sidecar");
    let raw_traces =
        serde_json::from_slice::<Vec<SandboxEventTrace>>(&raw_traces).expect("raw trace json");

    assert_eq!(
        report.required_metrics(),
        REQUIRED_LIFECYCLE_LATENCY_METRICS
    );
    assert_eq!(
        restored.required_metrics(),
        REQUIRED_LIFECYCLE_LATENCY_METRICS
    );
    assert_eq!(
        restored
            .summary_for("start.resume_to_first_stdout_ms")
            .expect("resume-to-stdout summary")
            .count(),
        2
    );
    assert_eq!(
        raw_samples.len(),
        REQUIRED_LIFECYCLE_LATENCY_METRICS.len() * 2
    );
    assert_eq!(raw_traces.len(), 1);
    assert_eq!(
        raw_traces[0]
            .headline_event(SandboxEventName::FirstStdoutByte)
            .map(SandboxTraceEvent::name),
        Some(SandboxEventName::FirstStdoutByte)
    );
}

#[test]
fn runtime_benchmark_writer_rejects_incomplete_lifecycle_samples() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("benchmark-evidence.json");
    let samples = REQUIRED_LIFECYCLE_LATENCY_METRICS
        .iter()
        .filter(|metric| **metric != "start.hot_to_ready_ms")
        .map(|metric| {
            BenchmarkSample::new(
                *metric,
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                10.0,
            )
        })
        .collect::<Vec<_>>();

    let error = RuntimeBenchmarkEvidenceWriter::new(&artifact)
        .write_samples(samples)
        .expect_err("missing metric rejects");

    assert!(matches!(
        error,
        RuntimeBenchmarkEvidenceError::Evidence(
            BenchmarkEvidenceError::MissingLifecycleLatency { metric }
        ) if metric == "start.hot_to_ready_ms"
    ));
    assert!(!artifact.exists());
    assert!(!temp.path().join("benchmark-evidence.samples.json").exists());
    assert!(!temp.path().join("benchmark-evidence.traces.json").exists());
}

#[test]
fn runtime_overhead_writer_validates_and_persists_required_overhead_samples() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("overhead-evidence.json");
    let samples = REQUIRED_FIRKIN_OVERHEAD_METRICS
        .iter()
        .map(|metric| {
            BenchmarkSample::new(
                metric.name,
                BenchmarkMetricKind::FirkinOverhead,
                metric.unit,
                1.0,
            )
        })
        .collect::<Vec<_>>();

    let report = RuntimeOverheadEvidenceWriter::new(&artifact)
        .write_samples(samples)
        .expect("write overhead artifact");
    let restored = BenchmarkOverheadEvidenceArtifact::read_json(&artifact).expect("read artifact");

    assert_eq!(
        report.required_metrics(),
        REQUIRED_FIRKIN_OVERHEAD_METRICS
            .iter()
            .map(|metric| metric.name)
            .collect::<Vec<_>>()
    );
    assert_eq!(restored.required_metrics(), report.required_metrics());
}

#[test]
fn runtime_agent_scorecard_writer_validates_and_persists_required_samples() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("agent-scorecard.json");
    let samples = required_scorecard_metric_definitions()
        .into_iter()
        .flat_map(|metric| {
            [1.0, 2.0].into_iter().map(move |value| {
                BenchmarkSample::new(metric.name, metric.kind, metric.unit, value)
            })
        })
        .collect::<Vec<_>>();

    let report = RuntimeAgentScorecardEvidenceWriter::new(&artifact)
        .with_min_samples(2)
        .write_samples(samples)
        .expect("write scorecard artifact");
    let restored = AgentBenchmarkScorecardArtifact::read_json(&artifact).expect("read artifact");

    assert_eq!(restored.required_metrics(), report.required_metrics());
    assert_eq!(
        restored
            .summary_for("start.agent_task_ready_ms")
            .expect("agent ready")
            .count(),
        2
    );
}

#[test]
fn runtime_agent_scorecard_writer_rejects_missing_metric() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("agent-scorecard.json");
    let samples = required_scorecard_metric_definitions()
        .into_iter()
        .filter(|metric| metric.name != "start.agent_task_ready_ms")
        .map(|metric| BenchmarkSample::new(metric.name, metric.kind, metric.unit, 1.0))
        .collect::<Vec<_>>();

    let error = RuntimeAgentScorecardEvidenceWriter::new(&artifact)
        .write_samples(samples)
        .expect_err("missing metric rejects");

    assert!(matches!(
        error,
        RuntimeBenchmarkEvidenceError::Scorecard(
            AgentBenchmarkScorecardError::MissingRequiredMetric { metric }
        ) if metric == "start.agent_task_ready_ms"
    ));
    assert!(!artifact.exists());
}

#[test]
fn runtime_autoscale_scorecard_writer_validates_and_persists_required_samples() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("autoscale-scorecard.json");
    let samples = required_autoscale_efficiency_metric_definitions()
        .into_iter()
        .flat_map(|metric| {
            [1.0, 2.0].into_iter().map(move |value| {
                BenchmarkSample::new(metric.name, metric.kind, metric.unit, value)
            })
        })
        .collect::<Vec<_>>();

    let report = RuntimeAutoscaleScorecardEvidenceWriter::new(&artifact)
        .with_min_samples(2)
        .write_samples_with_traces(
            samples,
            [autoscale_trace(SandboxEventName::ReadyTargetRestored)],
        )
        .expect("write autoscale scorecard artifact");
    let restored =
        AutoscaleEfficiencyScorecardArtifact::read_json(&artifact).expect("read artifact");
    let raw_samples_path = temp.path().join("autoscale-scorecard.samples.json");
    let raw_samples = std::fs::read(&raw_samples_path).expect("read raw samples sidecar");
    let raw_samples =
        serde_json::from_slice::<Vec<BenchmarkSample>>(&raw_samples).expect("raw sample json");
    let raw_traces_path = temp.path().join("autoscale-scorecard.traces.json");
    let raw_traces = std::fs::read(&raw_traces_path).expect("read raw traces sidecar");
    let raw_traces =
        serde_json::from_slice::<Vec<SandboxEventTrace>>(&raw_traces).expect("raw trace json");

    assert_eq!(restored.required_metrics(), report.required_metrics());
    assert_eq!(
        restored
            .summary_for("autoscale.pressure_to_safe_floor_ms")
            .expect("pressure shrink")
            .count(),
        2
    );
    assert_eq!(raw_samples.len(), report.required_metrics().len() * 2);
    assert_eq!(raw_traces.len(), 1);
    assert_eq!(
        raw_traces[0]
            .headline_event(SandboxEventName::ReadyTargetRestored)
            .map(SandboxTraceEvent::name),
        Some(SandboxEventName::ReadyTargetRestored)
    );
}

#[test]
fn runtime_autoscale_scorecard_writer_rejects_missing_metric() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("autoscale-scorecard.json");
    let samples = required_autoscale_efficiency_metric_definitions()
        .into_iter()
        .filter(|metric| metric.name != "autoscale.ready_queue_hit_rate_pct")
        .map(|metric| BenchmarkSample::new(metric.name, metric.kind, metric.unit, 1.0))
        .collect::<Vec<_>>();

    let error = RuntimeAutoscaleScorecardEvidenceWriter::new(&artifact)
        .write_samples(samples)
        .expect_err("missing autoscale metric rejects");

    assert!(matches!(
        error,
        RuntimeBenchmarkEvidenceError::AutoscaleScorecard(
            AutoscaleEfficiencyScorecardError::MissingRequiredMetric { metric }
        ) if metric == "autoscale.ready_queue_hit_rate_pct"
    ));
    assert!(!artifact.exists());
    assert!(
        !temp
            .path()
            .join("autoscale-scorecard.samples.json")
            .exists()
    );
    assert!(!temp.path().join("autoscale-scorecard.traces.json").exists());
}

#[test]
fn runtime_agent_computer_scorecard_writer_validates_and_persists_required_samples() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("agent-computer-scorecard.json");
    let samples = required_agent_computer_metric_definitions()
        .into_iter()
        .flat_map(|metric| {
            [1.0, 2.0].into_iter().map(move |value| {
                BenchmarkSample::new(metric.name, metric.kind, metric.unit, value)
            })
        })
        .collect::<Vec<_>>();

    let report = RuntimeAgentComputerScorecardEvidenceWriter::new(&artifact)
        .with_min_samples(2)
        .write_samples_with_traces(
            samples,
            [agent_computer_trace(SandboxEventName::AgentComputerReady)],
        )
        .expect("write agent-computer scorecard artifact");
    let restored = AgentComputerScorecardArtifact::read_json(&artifact).expect("read artifact");
    let raw_samples_path = temp.path().join("agent-computer-scorecard.samples.json");
    let raw_samples = std::fs::read(&raw_samples_path).expect("read raw samples sidecar");
    let raw_samples =
        serde_json::from_slice::<Vec<BenchmarkSample>>(&raw_samples).expect("raw sample json");
    let raw_traces_path = temp.path().join("agent-computer-scorecard.traces.json");
    let raw_traces = std::fs::read(&raw_traces_path).expect("read raw traces sidecar");
    let raw_traces =
        serde_json::from_slice::<Vec<SandboxEventTrace>>(&raw_traces).expect("raw trace json");

    assert_eq!(restored.required_metrics(), report.required_metrics());
    assert_eq!(
        restored
            .summary_for("product.agent_computer_ready_ms")
            .expect("agent-computer ready")
            .count(),
        2
    );
    assert_eq!(raw_samples.len(), report.required_metrics().len() * 2);
    assert_eq!(raw_traces.len(), 1);
    assert_eq!(
        raw_traces[0]
            .headline_event(SandboxEventName::AgentComputerReady)
            .map(SandboxTraceEvent::name),
        Some(SandboxEventName::AgentComputerReady)
    );
}

#[test]
fn runtime_agent_computer_scorecard_writer_rejects_missing_metric() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact = temp.path().join("agent-computer-scorecard.json");
    let samples = required_agent_computer_metric_definitions()
        .into_iter()
        .filter(|metric| metric.name != "product.agent_computer_ready_ms")
        .map(|metric| BenchmarkSample::new(metric.name, metric.kind, metric.unit, 1.0))
        .collect::<Vec<_>>();

    let error = RuntimeAgentComputerScorecardEvidenceWriter::new(&artifact)
        .write_samples(samples)
        .expect_err("missing agent-computer metric rejects");

    assert!(matches!(
        error,
        RuntimeBenchmarkEvidenceError::AgentComputerScorecard(
            AgentComputerScorecardError::MissingRequiredMetric { metric }
        ) if metric == "product.agent_computer_ready_ms"
    ));
    assert!(!artifact.exists());
    assert!(
        !temp
            .path()
            .join("agent-computer-scorecard.samples.json")
            .exists()
    );
    assert!(
        !temp
            .path()
            .join("agent-computer-scorecard.traces.json")
            .exists()
    );
}

fn agent_computer_trace(end: SandboxEventName) -> SandboxEventTrace {
    let mut trace = SandboxEventTrace::new();
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::AgentComputerRequestStart,
        1,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        end,
        2,
        LifecycleClass::Hot,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    ));
    trace
}

fn autoscale_trace(end: SandboxEventName) -> SandboxEventTrace {
    let mut trace = SandboxEventTrace::new();
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::PressureDetected,
        1,
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    ));
    trace.push(SandboxTraceEvent::new(
        end,
        2,
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    ));
    trace
}
