use std::collections::BTreeSet;
use std::path::Path;

use firkin_evidence::{DecisionMetricLevel, decision_grade_metric_contract};

#[test]
fn decision_grade_contract_names_endpoints_and_sample_floors_are_canonical() {
    let contract = decision_grade_metric_contract();
    let names = contract
        .iter()
        .map(|metric| metric.metric())
        .collect::<BTreeSet<_>>();

    assert_eq!(contract.len(), 13);
    assert!(names.contains("start.hot_to_first_stdout_ms"));
    assert!(names.contains("pool.lease_ms"));
    assert!(names.contains("exec.direct_first_stdout_byte_ms"));
    assert!(names.contains("density.max_active_before_retained_shell_first_stdout_p95_doubles"));
    assert!(names.contains("disk.sparse_bloat_after_trim"));

    for legacy in [
        "sandbox.start.hot_pool_checkout_ms",
        "command_start",
        "first_stdout_byte",
        "ready_probe",
        "warm_pool_checkout",
        "sandbox.density.max_active_before_p95_doubles",
    ] {
        assert!(
            !names.contains(legacy),
            "legacy metric remained active: {legacy}"
        );
    }

    let hot_stdout = contract
        .iter()
        .find(|metric| metric.metric() == "start.hot_to_first_stdout_ms")
        .expect("hot stdout metric");
    assert_eq!(hot_stdout.start_event().as_str(), "PoolLeaseAcquired");
    assert_eq!(hot_stdout.end_event().as_str(), "FirstStdoutByte");
    assert_eq!(hot_stdout.lifecycle().as_str(), "hot");
    assert_eq!(hot_stdout.workload().as_str(), "tiny_exec");
    assert_eq!(hot_stdout.percentile_policy().p95_min_samples(), 100);
    assert_eq!(hot_stdout.percentile_policy().p99_min_samples(), 500);
    assert_eq!(hot_stdout.level(), DecisionMetricLevel::FocusedDashboard);
}

#[test]
fn docs_metric_contract_table_matches_code_metric_names() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/specs/firkin-decision-grade-metric-contract.md");
    let docs = std::fs::read_to_string(&docs_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", docs_path.display()));
    let doc_names = docs
        .lines()
        .filter(|line| line.starts_with("| `"))
        .filter(|line| line.matches('|').count() >= 11)
        .filter_map(|line| line.split('|').nth(1))
        .map(str::trim)
        .filter_map(|cell| {
            cell.strip_prefix('`')
                .and_then(|value| value.strip_suffix('`'))
        })
        .collect::<BTreeSet<_>>();
    let code_names = decision_grade_metric_contract()
        .iter()
        .map(|metric| metric.metric())
        .collect::<BTreeSet<_>>();

    assert_eq!(doc_names, code_names);
}
