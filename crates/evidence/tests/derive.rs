//! Event-derived metric behavior tests.

use firkin_evidence::{
    DerivedMetricTrust, MetricDerivationError, ProductAutoscaleDurationMetric,
    decision_grade_metric_contract, derive_available_contract_metric_samples,
    derive_available_product_autoscale_metric_samples, derive_contract_metric_sample,
    derive_product_autoscale_metric_sample,
};
use firkin_trace::{
    LifecycleClass, RuntimeProfile, SandboxEventName, SandboxEventTrace, SandboxTraceEvent,
    WorkloadClass,
};

fn event(name: SandboxEventName, ns: u128) -> SandboxTraceEvent {
    SandboxTraceEvent::new(
        name,
        ns,
        LifecycleClass::Hot,
        WorkloadClass::TinyExec,
        RuntimeProfile::FastAgent,
    )
}

fn readiness_event(name: SandboxEventName, ns: u128) -> SandboxTraceEvent {
    SandboxTraceEvent::new(
        name,
        ns,
        LifecycleClass::Hot,
        WorkloadClass::ReadinessProbe,
        RuntimeProfile::FastAgent,
    )
}

fn agent_computer_event(
    name: SandboxEventName,
    ns: u128,
    lifecycle: LifecycleClass,
) -> SandboxTraceEvent {
    SandboxTraceEvent::new(
        name,
        ns,
        lifecycle,
        WorkloadClass::AgentComputer,
        RuntimeProfile::BrowserDbCli,
    )
}

fn autoscale_event(name: SandboxEventName, ns: u128) -> SandboxTraceEvent {
    SandboxTraceEvent::new(
        name,
        ns,
        LifecycleClass::Hot,
        WorkloadClass::AutoscaleScenario,
        RuntimeProfile::BrowserDbCli,
    )
}

fn contract(metric: &str) -> &'static firkin_evidence::MetricContract {
    decision_grade_metric_contract()
        .iter()
        .find(|contract| contract.metric() == metric)
        .expect("contract metric")
}

#[test]
fn derives_hot_to_first_stdout_from_canonical_event_pair() {
    let mut trace = SandboxEventTrace::new();
    trace.push(event(SandboxEventName::PoolLeaseAcquired, 10_000_000));
    trace.push(event(SandboxEventName::ExecRequestSent, 50_000_000));
    trace.push(event(SandboxEventName::FirstStdoutByte, 83_000_000));

    let sample =
        derive_contract_metric_sample(&trace, contract("start.hot_to_first_stdout_ms")).unwrap();

    assert_eq!(sample.metric(), "start.hot_to_first_stdout_ms");
    assert_eq!(sample.value(), 73.0);
    assert_eq!(sample.start_event(), SandboxEventName::PoolLeaseAcquired);
    assert_eq!(sample.end_event(), SandboxEventName::FirstStdoutByte);
    assert_eq!(sample.trust(), DerivedMetricTrust::ExactHostEventPair);
}

#[test]
fn derives_pool_lease_without_readiness_or_exec_time() {
    let mut trace = SandboxEventTrace::new();
    trace.push(event(SandboxEventName::PoolLeaseRequested, 1_000_000));
    trace.push(event(SandboxEventName::PoolLeaseAcquired, 10_000_000));
    trace.push(event(SandboxEventName::ReadyProbePassed, 40_000_000));
    trace.push(event(SandboxEventName::FirstStdoutByte, 70_000_000));

    let sample = derive_contract_metric_sample(&trace, contract("pool.lease_ms")).unwrap();

    assert_eq!(sample.metric(), "pool.lease_ms");
    assert_eq!(sample.value(), 9.0);
}

#[test]
fn induced_stdout_delay_moves_first_stdout_metric_by_same_amount() {
    let mut baseline = SandboxEventTrace::new();
    baseline.push(event(SandboxEventName::ExecRequestSent, 10_000_000));
    baseline.push(event(SandboxEventName::ProcessStarted, 20_000_000));
    baseline.push(event(SandboxEventName::FirstStdoutByte, 40_000_000));

    let mut delayed = SandboxEventTrace::new();
    delayed.push(event(SandboxEventName::ExecRequestSent, 10_000_000));
    delayed.push(event(SandboxEventName::ProcessStarted, 20_000_000));
    delayed.push(event(SandboxEventName::FirstStdoutByte, 140_000_000));

    let baseline_stdout =
        derive_contract_metric_sample(&baseline, contract("exec.first_stdout_byte_ms")).unwrap();
    let delayed_stdout =
        derive_contract_metric_sample(&delayed, contract("exec.first_stdout_byte_ms")).unwrap();
    let baseline_start =
        derive_contract_metric_sample(&baseline, contract("exec.command_start_ms")).unwrap();
    let delayed_start =
        derive_contract_metric_sample(&delayed, contract("exec.command_start_ms")).unwrap();

    assert_eq!(delayed_stdout.value() - baseline_stdout.value(), 100.0);
    assert_eq!(baseline_start.value(), delayed_start.value());
}

#[test]
fn induced_workspace_delay_moves_hot_ready_metric_by_same_amount() {
    let mut baseline = SandboxEventTrace::new();
    baseline.push(readiness_event(
        SandboxEventName::PoolLeaseAcquired,
        10_000_000,
    ));
    baseline.push(readiness_event(
        SandboxEventName::GuestAgentPingPassed,
        20_000_000,
    ));
    baseline.push(readiness_event(
        SandboxEventName::WorkspaceReady,
        30_000_000,
    ));
    baseline.push(readiness_event(
        SandboxEventName::ReadyProbePassed,
        40_000_000,
    ));

    let mut delayed = SandboxEventTrace::new();
    delayed.push(readiness_event(
        SandboxEventName::PoolLeaseAcquired,
        10_000_000,
    ));
    delayed.push(readiness_event(
        SandboxEventName::GuestAgentPingPassed,
        20_000_000,
    ));
    delayed.push(readiness_event(
        SandboxEventName::WorkspaceReady,
        130_000_000,
    ));
    delayed.push(readiness_event(
        SandboxEventName::ReadyProbePassed,
        140_000_000,
    ));

    let baseline_ready =
        derive_contract_metric_sample(&baseline, contract("start.hot_to_ready_ms")).unwrap();
    let delayed_ready =
        derive_contract_metric_sample(&delayed, contract("start.hot_to_ready_ms")).unwrap();

    assert_eq!(delayed_ready.value() - baseline_ready.value(), 100.0);
}

#[test]
fn missing_workspace_blocks_hot_ready_derivation() {
    let mut trace = SandboxEventTrace::new();
    trace.push(event(SandboxEventName::PoolLeaseAcquired, 10_000_000));
    trace.push(event(SandboxEventName::GuestAgentPingPassed, 20_000_000));
    trace.push(event(SandboxEventName::ReadyProbePassed, 40_000_000));

    let error =
        derive_contract_metric_sample(&trace, contract("start.hot_to_ready_ms")).unwrap_err();

    assert!(matches!(
        error,
        MetricDerivationError::MissingPrerequisite {
            metric: "start.hot_to_ready_ms",
            event: SandboxEventName::WorkspaceReady
        }
    ));
}

#[test]
fn wrong_workload_is_rejected_instead_of_crossing_trace_classes() {
    let mut trace = SandboxEventTrace::new();
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::PoolLeaseAcquired,
        10_000_000,
        LifecycleClass::Hot,
        WorkloadClass::ShellExec,
        RuntimeProfile::FastAgent,
    ));
    trace.push(SandboxTraceEvent::new(
        SandboxEventName::FirstStdoutByte,
        80_000_000,
        LifecycleClass::Hot,
        WorkloadClass::ShellExec,
        RuntimeProfile::FastAgent,
    ));

    let error = derive_contract_metric_sample(&trace, contract("start.hot_to_first_stdout_ms"))
        .unwrap_err();

    assert!(matches!(
        error,
        MetricDerivationError::WrongWorkload {
            metric: "start.hot_to_first_stdout_ms",
            expected: WorkloadClass::TinyExec,
            actual: WorkloadClass::ShellExec
        }
    ));
}

#[test]
fn non_duration_dashboard_metrics_are_blocked_until_stage_values_exist() {
    let mut trace = SandboxEventTrace::new();
    trace.push(event(SandboxEventName::FstrimDone, 10_000_000));

    let error = derive_contract_metric_sample(&trace, contract("disk.sparse_bloat_after_trim"))
        .unwrap_err();

    assert!(matches!(
        error,
        MetricDerivationError::UnsupportedValueSource {
            metric: "disk.sparse_bloat_after_trim"
        }
    ));
}

#[test]
fn available_contract_metrics_derive_canonical_exec_samples_from_one_trace() {
    let mut trace = SandboxEventTrace::new();
    trace.push(event(SandboxEventName::ExecRequestSent, 10_000_000));
    trace.push(event(SandboxEventName::ProcessStarted, 32_000_000));
    trace.push(event(SandboxEventName::FirstStdoutByte, 45_000_000));
    trace.push(event(SandboxEventName::ProcessExited, 70_000_000));

    let samples = derive_available_contract_metric_samples([trace]);
    let by_metric = samples
        .iter()
        .map(|sample| (sample.metric(), sample))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(
        by_metric
            .get("exec.command_start_ms")
            .expect("command start")
            .value(),
        22.0
    );
    assert_eq!(
        by_metric
            .get("exec.first_stdout_byte_ms")
            .expect("first stdout")
            .value(),
        35.0
    );
    assert_eq!(
        by_metric
            .get("exec.command_start_ms")
            .expect("command start")
            .tag_value("trust"),
        Some("exact_host_event_pair")
    );
}

#[test]
fn derives_agent_computer_ready_from_product_trace() {
    let mut trace = SandboxEventTrace::new();
    trace.push(agent_computer_event(
        SandboxEventName::AgentComputerRequestStart,
        1_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::CliFirstUsefulStdout,
        50_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::BrowserReady,
        75_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::DatabaseReady,
        125_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::AgentComputerReady,
        201_000_000,
        LifecycleClass::Hot,
    ));

    let sample = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::AgentComputerReady,
    )
    .unwrap();

    assert_eq!(sample.metric(), "product.agent_computer_ready_ms");
    assert_eq!(sample.value(), 200.0);
    assert_eq!(
        sample.start_event(),
        SandboxEventName::AgentComputerRequestStart
    );
    assert_eq!(sample.end_event(), SandboxEventName::AgentComputerReady);
    assert_eq!(sample.workload(), WorkloadClass::AgentComputer);
    assert_eq!(sample.profile(), RuntimeProfile::BrowserDbCli);
}

#[test]
fn derives_agent_computer_product_drilldowns_from_hot_trace() {
    let mut trace = SandboxEventTrace::new();
    trace.push(agent_computer_event(
        SandboxEventName::AgentComputerRequestStart,
        1_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::CliFirstUsefulStdout,
        50_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::BrowserReady,
        75_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::DatabaseReady,
        125_000_000,
        LifecycleClass::Hot,
    ));

    let cli = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::AgentComputerCliReady,
    )
    .unwrap();
    let browser = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::AgentComputerBrowserReady,
    )
    .unwrap();
    let database = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::AgentComputerDatabaseReady,
    )
    .unwrap();

    assert_eq!(cli.metric(), "product.cli_ready_ms");
    assert_eq!(cli.value(), 49.0);
    assert_eq!(browser.metric(), "product.browser_ready_ms");
    assert_eq!(browser.value(), 74.0);
    assert_eq!(database.metric(), "product.database_ready_ms");
    assert_eq!(database.value(), 124.0);

    let cli_sample = cli.into_benchmark_sample();
    assert_eq!(
        cli_sample.tag_value("probe_surface"),
        Some("code_interpreter_exec")
    );
    assert_eq!(
        cli_sample.tag_value("measurement_boundary"),
        Some("cli_proxy")
    );

    let browser_sample = browser.into_benchmark_sample();
    assert_eq!(
        browser_sample.tag_value("probe_surface"),
        Some("code_interpreter_health")
    );
    assert_eq!(
        browser_sample.tag_value("measurement_boundary"),
        Some("browser_proxy")
    );

    let database_sample = database.into_benchmark_sample();
    assert_eq!(
        database_sample.tag_value("probe_surface"),
        Some("code_interpreter_exec")
    );
    assert_eq!(
        database_sample.tag_value("measurement_boundary"),
        Some("sqlite_proxy_not_db_sidecar")
    );
}

#[test]
fn missing_browser_probe_blocks_product_ready_derivation() {
    let mut trace = SandboxEventTrace::new();
    trace.push(agent_computer_event(
        SandboxEventName::AgentComputerRequestStart,
        1_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::CliFirstUsefulStdout,
        50_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::DatabaseReady,
        125_000_000,
        LifecycleClass::Hot,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::AgentComputerReady,
        201_000_000,
        LifecycleClass::Hot,
    ));

    let error = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::AgentComputerReady,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        MetricDerivationError::MissingPrerequisite {
            metric: "product.agent_computer_ready_ms",
            event: SandboxEventName::BrowserReady
        }
    ));
}

#[test]
fn derives_agent_computer_resume_from_resumed_trace() {
    let mut trace = SandboxEventTrace::new();
    trace.push(agent_computer_event(
        SandboxEventName::AgentComputerResumed,
        100_000_000,
        LifecycleClass::Resumed,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::CliFirstUsefulStdout,
        115_000_000,
        LifecycleClass::Resumed,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::BrowserReady,
        125_000_000,
        LifecycleClass::Resumed,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::DatabaseReady,
        135_000_000,
        LifecycleClass::Resumed,
    ));
    trace.push(agent_computer_event(
        SandboxEventName::AgentComputerReady,
        160_000_000,
        LifecycleClass::Resumed,
    ));

    let sample = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::AgentComputerResume,
    )
    .unwrap();

    assert_eq!(sample.metric(), "product.agent_computer_resume_ms");
    assert_eq!(sample.value(), 60.0);
    assert_eq!(sample.lifecycle(), LifecycleClass::Resumed);
}

#[test]
fn derives_autoscale_pressure_and_refill_spans() {
    let mut trace = SandboxEventTrace::new();
    trace.push(autoscale_event(
        SandboxEventName::PressureDetected,
        100_000_000,
    ));
    trace.push(autoscale_event(
        SandboxEventName::AutoscaleDecisionMade,
        120_000_000,
    ));
    trace.push(autoscale_event(
        SandboxEventName::AutoscaleActionStarted,
        150_000_000,
    ));
    trace.push(autoscale_event(
        SandboxEventName::SafeFloorRestored,
        600_000_000,
    ));
    trace.push(autoscale_event(
        SandboxEventName::ReadyTargetRestored,
        1_600_000_000,
    ));

    let shrink = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::PressureToSafeFloor,
    )
    .unwrap();
    let refill = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::PressureClearToReadyTarget,
    )
    .unwrap();

    assert_eq!(shrink.metric(), "autoscale.pressure_to_safe_floor_ms");
    assert_eq!(shrink.value(), 500.0);
    assert_eq!(
        refill.metric(),
        "autoscale.pressure_clear_to_ready_target_ms"
    );
    assert_eq!(refill.value(), 1000.0);
}

#[test]
fn missing_autoscale_action_blocks_pressure_shrink_derivation() {
    let mut trace = SandboxEventTrace::new();
    trace.push(autoscale_event(
        SandboxEventName::PressureDetected,
        100_000_000,
    ));
    trace.push(autoscale_event(
        SandboxEventName::AutoscaleDecisionMade,
        120_000_000,
    ));
    trace.push(autoscale_event(
        SandboxEventName::SafeFloorRestored,
        600_000_000,
    ));

    let error = derive_product_autoscale_metric_sample(
        &trace,
        ProductAutoscaleDurationMetric::PressureToSafeFloor,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        MetricDerivationError::MissingPrerequisite {
            metric: "autoscale.pressure_to_safe_floor_ms",
            event: SandboxEventName::AutoscaleActionStarted
        }
    ));
}

#[test]
fn available_product_autoscale_metrics_derive_from_mixed_traces() {
    let mut product = SandboxEventTrace::new();
    product.push(agent_computer_event(
        SandboxEventName::AgentComputerRequestStart,
        1_000_000,
        LifecycleClass::Hot,
    ));
    product.push(agent_computer_event(
        SandboxEventName::CliFirstUsefulStdout,
        50_000_000,
        LifecycleClass::Hot,
    ));
    product.push(agent_computer_event(
        SandboxEventName::BrowserReady,
        75_000_000,
        LifecycleClass::Hot,
    ));
    product.push(agent_computer_event(
        SandboxEventName::DatabaseReady,
        125_000_000,
        LifecycleClass::Hot,
    ));
    product.push(agent_computer_event(
        SandboxEventName::AgentComputerReady,
        201_000_000,
        LifecycleClass::Hot,
    ));

    let mut autoscale = SandboxEventTrace::new();
    autoscale.push(autoscale_event(
        SandboxEventName::PressureDetected,
        100_000_000,
    ));
    autoscale.push(autoscale_event(
        SandboxEventName::AutoscaleDecisionMade,
        120_000_000,
    ));
    autoscale.push(autoscale_event(
        SandboxEventName::AutoscaleActionStarted,
        150_000_000,
    ));
    autoscale.push(autoscale_event(
        SandboxEventName::SafeFloorRestored,
        600_000_000,
    ));
    autoscale.push(autoscale_event(
        SandboxEventName::ReadyTargetRestored,
        1_600_000_000,
    ));

    let samples = derive_available_product_autoscale_metric_samples([product, autoscale]);
    let by_metric = samples
        .iter()
        .map(|sample| (sample.metric(), sample))
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(
        by_metric
            .get("product.agent_computer_ready_ms")
            .expect("product ready")
            .value(),
        200.0
    );
    assert_eq!(
        by_metric
            .get("product.cli_ready_ms")
            .expect("cli drilldown")
            .value(),
        49.0
    );
    assert_eq!(
        by_metric
            .get("product.browser_ready_ms")
            .expect("browser drilldown")
            .value(),
        74.0
    );
    assert_eq!(
        by_metric
            .get("product.database_ready_ms")
            .expect("database drilldown")
            .value(),
        124.0
    );
    assert_eq!(
        by_metric
            .get("autoscale.pressure_to_safe_floor_ms")
            .expect("pressure shrink")
            .tag_value("trust"),
        Some("exact_host_event_pair")
    );
    assert_eq!(
        by_metric
            .get("autoscale.pressure_clear_to_ready_target_ms")
            .expect("refill")
            .value(),
        1000.0
    );
}
