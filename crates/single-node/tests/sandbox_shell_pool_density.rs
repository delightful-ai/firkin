#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use firkin_sandbox::{Command, DataPlaneSpec, Runtime, SandboxSpec, TemplateSpec};
use firkin_single_node::{AppleVzBackend, SingleNodeConfig};
use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use futures_util::future::join_all;

const SHELL_POOL_DENSITY_BREAKPOINT_METRIC: &str =
    "debug.density.max_active_before_sandbox_shell_pool_run_completion_p95_doubles";

#[tokio::test]
#[ignore = "live VM-backed shell pool density proof; requires signed test harness"]
async fn live_sandbox_shell_pool_density_writes_repeat_samples() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let runtime_temp = tempfile::tempdir().expect("runtime tempdir");
    let artifact = shell_pool_density_artifact_path(
        artifact_temp.path(),
        std::env::var_os("FIRKIN_LIVE_SANDBOX_SHELL_POOL_DENSITY_ARTIFACT").as_deref(),
    );
    let repeats = repeat_count(
        std::env::var_os("FIRKIN_LIVE_SANDBOX_SHELL_POOL_DENSITY_REPEATS").as_deref(),
        10,
    );
    let density_levels = density_levels(
        std::env::var_os("FIRKIN_LIVE_SANDBOX_SHELL_POOL_DENSITY_LEVELS").as_deref(),
        &[1, 2, 4, 8],
    );
    let image = std::env::var("FIRKIN_LIVE_SANDBOX_SHELL_POOL_IMAGE")
        .unwrap_or_else(|_| "debian:bookworm-slim".to_owned());

    let backend = AppleVzBackend::from_config(SingleNodeConfig::new(
        runtime_temp.path().join("state"),
        "cube.localhost",
    ));
    let runtime = Runtime::build(backend).await.expect("runtime");
    let template = runtime
        .templates()
        .prepare(TemplateSpec::oci(image).data_plane(DataPlaneSpec::none()))
        .await
        .expect("prepare template");
    let sandbox = runtime
        .sandboxes()
        .create(SandboxSpec::from_template(&template))
        .await
        .expect("create sandbox");

    let samples = collect_shell_pool_density_samples(&sandbox, &density_levels, repeats).await;
    assert_eq!(samples.len(), density_levels.len() * repeats);

    if let Some(parent) = artifact.parent() {
        std::fs::create_dir_all(parent).expect("create shell pool density artifact parent");
    }
    write_shell_pool_density_artifact(&artifact, samples);
    assert!(artifact.exists());

    let _ = sandbox.delete().await;
}

#[test]
fn shell_pool_density_artifact_tags_completion_boundary() {
    let artifact_temp = tempfile::tempdir().expect("artifact tempdir");
    let artifact = artifact_temp.path().join("shell-pool-density.json");
    write_shell_pool_density_artifact(
        &artifact,
        vec![
            shell_pool_density_sample(1, 0, 2, Duration::from_micros(500)),
            shell_pool_density_sample(1, 1, 2, Duration::from_micros(600)),
            shell_pool_density_sample(2, 0, 2, Duration::from_micros(700)),
            shell_pool_density_sample(2, 1, 2, Duration::from_micros(800)),
        ],
    );
    let value: serde_json::Value =
        serde_json::from_reader(std::fs::File::open(&artifact).expect("open artifact"))
            .expect("parse artifact");
    assert_eq!(value["kind"], "live_sandbox_shell_pool_density");
    let samples = value["samples"].as_array().expect("samples");
    assert!(samples.iter().any(|sample| {
        sample["metric"] == SHELL_POOL_DENSITY_BREAKPOINT_METRIC
            && sample["value"].as_f64() == Some(2.0)
    }));
    let sample = samples
        .iter()
        .find(|sample| sample["metric"] == "debug.exec.sandbox_shell_pool_run_completion_c1_ms")
        .expect("c1 sample");
    assert_eq!(
        sample["tags"]["measurement_boundary"],
        "sandbox_shell_pool_run_completion"
    );
    assert_eq!(sample["tags"]["shell_surface"], "firkin_sandbox_shell_pool");
    assert_eq!(sample["tags"]["output_boundary"], "command_completion");
    assert_eq!(sample["tags"]["warmup_dispatch"], "excluded");
}

async fn collect_shell_pool_density_samples(
    sandbox: &firkin_sandbox::Sandbox,
    density_levels: &[usize],
    repeats: usize,
) -> Vec<BenchmarkSample> {
    let density_level_tag = density_levels
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut samples = Vec::with_capacity(density_levels.len() * repeats);
    for &concurrency in density_levels {
        let pool = sandbox
            .shell_pool(concurrency)
            .await
            .unwrap_or_else(|error| panic!("open shell pool c{concurrency}: {error}"));
        let slots = pool.slots();
        let warmups = join_all(slots.iter().cloned().enumerate().map(
            |(index, shell)| async move {
                shell
                    .run(Command::shell(format!(
                        "printf shell-pool-warm-c{concurrency}-{index}"
                    )))
                    .await
                    .unwrap_or_else(|error| panic!("warm shell c{concurrency}/{index}: {error}"))
            },
        ))
        .await;
        assert_eq!(warmups.len(), concurrency);

        for repeat in 0..repeats {
            let dispatches = join_all(slots.iter().cloned().enumerate().map(
                |(index, shell)| async move {
                    let expected = format!("shell-pool-c{concurrency}-r{repeat}-{index}");
                    let started = Instant::now();
                    let output = shell
                        .run(Command::shell(format!("printf {expected}")))
                        .await
                        .unwrap_or_else(|error| {
                            panic!("run shell c{concurrency}/r{repeat}/{index}: {error}")
                        });
                    assert_eq!(output.stdout, expected.as_bytes());
                    started.elapsed()
                },
            ))
            .await;
            let slowest = dispatches
                .into_iter()
                .max()
                .unwrap_or_else(|| panic!("shell pool density c{concurrency} had no shells"));
            samples.push(
                shell_pool_density_sample(concurrency, repeat, repeats, slowest)
                    .with_dynamic_tag("concurrency_levels", density_level_tag.clone()),
            );
        }
        pool.close()
            .await
            .unwrap_or_else(|error| panic!("close shell pool c{concurrency}: {error}"));
    }
    samples
}

fn shell_pool_density_sample(
    concurrency: usize,
    repeat: usize,
    repeat_count: usize,
    elapsed: Duration,
) -> BenchmarkSample {
    BenchmarkSample::new(
        format!("debug.exec.sandbox_shell_pool_run_completion_c{concurrency}_ms"),
        BenchmarkMetricKind::LifecycleLatency,
        BenchmarkUnit::Milliseconds,
        elapsed.as_secs_f64() * 1000.0,
    )
    .with_static_tag("measurement_boundary", "sandbox_shell_pool_run_completion")
    .with_static_tag("shell_surface", "firkin_sandbox_shell_pool")
    .with_static_tag("output_boundary", "command_completion")
    .with_static_tag("command", "printf")
    .with_static_tag("warmup_dispatch", "excluded")
    .with_dynamic_tag("concurrency_level", concurrency.to_string())
    .with_dynamic_tag("repeat_index", repeat.to_string())
    .with_dynamic_tag("repeat_count", repeat_count.to_string())
}

fn write_shell_pool_density_artifact(artifact: &Path, mut samples: Vec<BenchmarkSample>) {
    if let Some(sample) = shell_pool_density_breakpoint_sample(&samples) {
        samples.push(sample);
    }
    let mut metric_counts = BTreeMap::<String, usize>::new();
    for sample in &samples {
        *metric_counts.entry(sample.metric().to_owned()).or_default() += 1;
    }
    let samples = samples
        .into_iter()
        .map(|sample| {
            let confidence = confidence_for_count(metric_counts[sample.metric()]);
            sample.with_dynamic_tag("confidence", confidence)
        })
        .collect::<Vec<_>>();
    let file = std::fs::File::create(artifact).expect("create shell pool density artifact");
    serde_json::to_writer_pretty(
        file,
        &serde_json::json!({
            "kind": "live_sandbox_shell_pool_density",
            "samples": samples,
            "traces": [],
        }),
    )
    .expect("write shell pool density artifact");
}

fn shell_pool_density_breakpoint_sample(samples: &[BenchmarkSample]) -> Option<BenchmarkSample> {
    let mut samples_by_level = BTreeMap::<usize, Vec<f64>>::new();
    let mut concurrency_levels = None;
    for sample in samples {
        if !sample
            .metric()
            .starts_with("debug.exec.sandbox_shell_pool_run_completion_c")
        {
            continue;
        }
        concurrency_levels = concurrency_levels
            .or_else(|| sample.tag_value("concurrency_levels").map(str::to_owned));
        let Some(level) = sample
            .tag_value("concurrency_level")
            .and_then(|value| value.parse::<usize>().ok())
        else {
            continue;
        };
        samples_by_level
            .entry(level)
            .or_default()
            .push(sample.value());
    }
    let baseline = samples_by_level
        .get(&1)
        .map(|values| nearest_rank(values, 95))?;
    let threshold = baseline * 2.0;
    let breakpoint = samples_by_level
        .iter()
        .filter_map(|(&level, values)| {
            let p95 = nearest_rank(values, 95);
            (p95 <= threshold).then_some(level)
        })
        .max()?;
    let mut sample = BenchmarkSample::from_static(
        SHELL_POOL_DENSITY_BREAKPOINT_METRIC,
        BenchmarkMetricKind::WorkloadResource,
        BenchmarkUnit::Count,
        breakpoint as f64,
    )
    .with_static_tag("source", "sandbox-shell-pool-run-completion-p95-threshold")
    .with_static_tag("measurement_boundary", "sandbox_shell_pool_run_completion")
    .with_static_tag("shell_surface", "firkin_sandbox_shell_pool")
    .with_static_tag("output_boundary", "command_completion")
    .with_dynamic_tag("baseline_p95_ms", format!("{baseline:.6}"))
    .with_dynamic_tag("threshold_p95_ms", format!("{threshold:.6}"));
    if let Some(levels) = concurrency_levels {
        sample = sample.with_dynamic_tag("concurrency_levels", levels);
    }
    Some(sample)
}

fn nearest_rank(values: &[f64], percentile: usize) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = sorted.len().saturating_mul(percentile).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn shell_pool_density_artifact_path(temp: &Path, override_path: Option<&OsStr>) -> PathBuf {
    override_path.map_or_else(
        || temp.join("live-sandbox-shell-pool-density.json"),
        PathBuf::from,
    )
}

fn repeat_count(value: Option<&OsStr>, default: usize) -> usize {
    value
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn density_levels(value: Option<&OsStr>, default: &[usize]) -> Vec<usize> {
    value
        .and_then(|value| value.to_str())
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| item.trim().parse::<usize>().ok())
                .filter(|level| *level > 0)
                .collect::<Vec<_>>()
        })
        .filter(|levels| !levels.is_empty())
        .unwrap_or_else(|| default.to_vec())
}

fn confidence_for_count(count: usize) -> &'static str {
    match count {
        0 => "missing",
        1..=2 => "smoke_only",
        3..=4 => "superfast_iteration",
        5..=9 => "fast_iteration",
        10..=29 => "baseline_checkpoint",
        30..=99 => "p50_p90_decision_grade",
        100..=499 => "p95_decision_grade",
        _ => "p99_decision_grade",
    }
}
