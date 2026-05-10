#!/usr/bin/env bash
set -euo pipefail

suite="${1:-agent-core}"
sample_tier="${FIRKIN_BASELINE_SAMPLE_TIER:-superfast_iteration}"
case "$sample_tier" in
  superfast | superfast_iteration)
    default_sample_count=3
    ;;
  fast | fast_iteration)
    default_sample_count=5
    ;;
  p50_p90 | p50_p90_decision_grade)
    default_sample_count=30
    ;;
  p95 | p95_decision_grade)
    default_sample_count=100
    ;;
  *)
    echo "invalid_baseline_sample_tier=$sample_tier" >&2
    echo "valid_baseline_sample_tiers=superfast_iteration,fast_iteration,p50_p90_decision_grade,p95_decision_grade" >&2
    exit 1
    ;;
esac
duration="${FIRKIN_BASELINE_DURATION:-$((default_sample_count * 20))s}"
name="${FIRKIN_BASELINE_NAME:-local-${suite}-${duration}-release}"
out="${FIRKIN_BASELINE_OUT:-target/firkin-live-evidence/${name}.json}"
min_free_bytes="${FIRKIN_BASELINE_MIN_FREE_BYTES:-21474836480}"
fk="target/release/fk"
state_root="${FIRKIN_STATE_DIR:-$HOME/.firkin/state}"
benchmark_root="${FIRKIN_BENCHMARK_DIR:-$HOME/.firkin/benchmarks}"
restore_staging="$state_root/runtime/restore-staging"
product_pod_proof="${FIRKIN_BASELINE_PRODUCT_POD_PROOF:-auto}"
autoscale_proof="${FIRKIN_BASELINE_AUTOSCALE_PROOF:-auto}"
agent_computer_min_samples="${FIRKIN_BASELINE_AGENT_COMPUTER_MIN_SAMPLES:-$default_sample_count}"
ready_deck_repeats="${FIRKIN_BASELINE_PRODUCT_POD_READY_DECK_REPEATS:-$default_sample_count}"
autoscale_repeats="${FIRKIN_BASELINE_AUTOSCALE_REPEATS:-$default_sample_count}"
autoscale_min_samples="${FIRKIN_BASELINE_AUTOSCALE_MIN_SAMPLES:-$autoscale_repeats}"
shell_density_levels="${FIRKIN_BASELINE_SHELL_DENSITY_LEVELS:-1,2}"
ready_deck_density_levels="${FIRKIN_BASELINE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS:-1,2,4}"
prestarted_slot_density_levels="${FIRKIN_BASELINE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS:-1,2,4}"
retained_shell_density_repeats="${FIRKIN_BASELINE_RETAINED_SHELL_DENSITY_REPEATS:-1}"
retained_shell_density_proof="${FIRKIN_BASELINE_RETAINED_SHELL_DENSITY_PROOF:-auto}"
retained_shell_density_proof_levels="${FIRKIN_BASELINE_RETAINED_SHELL_DENSITY_PROOF_LEVELS:-1,2,4,8}"
retained_shell_density_proof_repeats="${FIRKIN_BASELINE_RETAINED_SHELL_DENSITY_PROOF_REPEATS:-10}"
direct_exec_first_stdout_proof="${FIRKIN_BASELINE_DIRECT_EXEC_FIRST_STDOUT_PROOF:-auto}"
direct_exec_first_stdout_proof_repeats="${FIRKIN_BASELINE_DIRECT_EXEC_FIRST_STDOUT_PROOF_REPEATS:-10}"
preflight_only="${FIRKIN_BASELINE_PREFLIGHT_ONLY:-0}"
contention_file="$(mktemp "${TMPDIR:-/tmp}/firkin-benchmark-processes.XXXXXX")"
cleanup_temp_files() {
  rm -f "$contention_file"
}
trap cleanup_temp_files EXIT

duration_to_repeats() {
  local value="$1"
  local seconds=""

  case "$value" in
    *s)
      seconds="${value%s}"
      ;;
    *m)
      seconds="$(( ${value%m} * 60 ))"
      ;;
    *)
      echo "unsupported_duration=$value" >&2
      echo "supported_duration_suffixes=s,m" >&2
      exit 1
      ;;
  esac

  if [[ "$seconds" =~ [^0-9] || "$seconds" -le 0 ]]; then
    echo "invalid_duration=$value" >&2
    exit 1
  fi

  echo "$(( (seconds + 19) / 20 ))"
}

duration_repeats="$(duration_to_repeats "$duration")"

emit_baseline_preflight() {
  cat <<EOF
baseline_preflight=signed-live-baseline
suite=$suite
sample_tier=$sample_tier
default_sample_count=$default_sample_count
duration=$duration
duration_repeats=$duration_repeats
baseline_name=$name
baseline_out=$out
state_root=$state_root
benchmark_root=$benchmark_root
min_free_bytes=$min_free_bytes
will_build_release=$([[ "${FIRKIN_BASELINE_NO_BUILD:-0}" == "1" ]] && echo false || echo true)
will_run_signed_live=true
product_pod_proof=$product_pod_proof
agent_computer_min_samples=$agent_computer_min_samples
product_pod_ready_deck_repeats=$ready_deck_repeats
autoscale_proof=$autoscale_proof
autoscale_repeats=$autoscale_repeats
autoscale_min_samples=$autoscale_min_samples
shell_density_levels=$shell_density_levels
product_pod_ready_deck_density_levels=$ready_deck_density_levels
product_pod_prestarted_agent_slot_density_levels=$prestarted_slot_density_levels
retained_shell_density_repeats=$retained_shell_density_repeats
retained_shell_density_proof=$retained_shell_density_proof
retained_shell_density_proof_levels=$retained_shell_density_proof_levels
retained_shell_density_proof_repeats=$retained_shell_density_proof_repeats
direct_exec_first_stdout_proof=$direct_exec_first_stdout_proof
direct_exec_first_stdout_proof_repeats=$direct_exec_first_stdout_proof_repeats
EOF
}

if [[ "$preflight_only" == "1" ]]; then
  emit_baseline_preflight
  exit 0
fi

if [[ "$suite" == "agent-computer" && -z "${FIRKIN_BASELINE_AGENT_COMPUTER_MIN_SAMPLES+x}" && "$duration_repeats" -lt "$agent_computer_min_samples" ]]; then
  echo "agent_computer_min_samples_unreachable=true" >&2
  echo "duration=$duration" >&2
  echo "duration_repeats=$duration_repeats" >&2
  echo "agent_computer_min_samples=$agent_computer_min_samples" >&2
  echo "next_action=increase FIRKIN_BASELINE_DURATION to at least $((agent_computer_min_samples * 20))s or explicitly set FIRKIN_BASELINE_AGENT_COMPUTER_MIN_SAMPLES=$duration_repeats for a wiring smoke" >&2
  exit 1
fi

emit_artifact_sidecars() {
  local prefix="$1"
  local artifact="$2"
  local sidecar_base="${artifact%.json}"
  local samples="${sidecar_base}.samples.json"
  local traces="${sidecar_base}.traces.json"

  if [[ -f "$samples" ]]; then
    echo "${prefix}_samples=$samples"
  fi
  if [[ -f "$traces" ]]; then
    echo "${prefix}_traces=$traces"
    echo "${prefix}_trace_report_command=$fk benchmark report-agent-computer-traces $traces"
  fi
}

trace_artifact_for() {
  local artifact="$1"
  local traces="${artifact%.json}.traces.json"

  if [[ -f "$traces" ]]; then
    printf '%s\n' "$traces"
  else
    printf '%s\n' "$artifact"
  fi
}

emit_density_status() {
  local prefix="$1"
  local actual="$2"
  local target="$3"

  python3 - "$prefix" "$actual" "$target" <<'PY'
import sys

prefix, actual, target = sys.argv[1], sys.argv[2], float(sys.argv[3])
print(f"{prefix}_target={target:g}")
try:
    value = float(actual)
except ValueError:
    print(f"{prefix}_target_status=missing")
else:
    print(f"{prefix}_target_status={'pass' if value >= target else 'miss'}")
PY
}

emit_autoscale_summary() {
  local artifact="$1"

  python3 - "$artifact" <<'PY'
import json
import sys

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

prefixes = {
    "autoscale.ready_queue_hit_rate_pct": "autoscale_ready_queue_hit_rate_pct",
    "autoscale.safe_spare_limiting_utilization_pct": "autoscale_safe_spare_limiting_utilization_pct",
    "autoscale.pressure_to_safe_floor_ms": "autoscale_pressure_to_safe_floor_ms",
    "autoscale.pressure_clear_to_ready_target_ms": "autoscale_pressure_clear_to_ready_target_ms",
    "autoscale.reserve_floor_violations": "autoscale_reserve_floor_violations",
    "autoscale.active_evictions_due_to_pool_pressure": "autoscale_active_evictions_due_to_pool_pressure",
    "density.max_agent_computers_before_ready_p95_doubles": "autoscale_agent_computer_density_breakpoint",
    "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles": "autoscale_prestarted_slot_density_breakpoint",
}

summaries = {
    summary.get("metric"): summary for summary in evidence.get("summaries", [])
}
for metric, prefix in prefixes.items():
    summary = summaries.get(metric)
    if summary is None:
        print(f"{prefix}=missing")
        continue
    print(f"{prefix}_count={summary.get('count', 'missing')}")
    print(f"{prefix}_confidence={summary.get('percentile_availability', 'missing')}")
    print(f"{prefix}_p95={summary.get('p95', 'missing')}")
    print(f"{prefix}_max={summary.get('max', 'missing')}")
PY
}

emit_storage_context() {
  python3 - "$PWD" "$state_root" "$benchmark_root" "$out" <<'PY'
import os
import subprocess
import sys

labels = {
    "repo": sys.argv[1],
    "state_root": sys.argv[2],
    "benchmark_root": sys.argv[3],
    "artifact": sys.argv[4],
}

def probe_path(path):
    if os.path.exists(path):
        return path
    parent = os.path.dirname(path)
    while parent and not os.path.exists(parent):
        next_parent = os.path.dirname(parent)
        if next_parent == parent:
            break
        parent = next_parent
    return parent or "."

def df(path):
    probe = probe_path(path)
    output = subprocess.check_output(["df", "-Pk", probe], text=True)
    fields = output.strip().splitlines()[-1].split()
    return {
        "filesystem": fields[0],
        "total_bytes": int(fields[1]) * 1024,
        "used_bytes": int(fields[2]) * 1024,
        "available_bytes": int(fields[3]) * 1024,
        "capacity": fields[4],
        "mount": fields[-1],
    }

infos = {}
for label, path in labels.items():
    try:
        infos[label] = df(path)
    except Exception as exc:
        print(f"host_storage_{label}_error={exc}")

for label, info in infos.items():
    prefix = f"host_storage_{label}"
    print(f"{prefix}_filesystem={info['filesystem']}")
    print(f"{prefix}_mount={info['mount']}")
    print(f"{prefix}_total_bytes={info['total_bytes']}")
    print(f"{prefix}_available_bytes={info['available_bytes']}")
    print(f"{prefix}_capacity={info['capacity']}")

def same_volume(left, right):
    return (
        left in infos
        and right in infos
        and infos[left]["filesystem"] == infos[right]["filesystem"]
        and infos[left]["mount"] == infos[right]["mount"]
    )

print(f"host_storage_repo_state_same_volume={str(same_volume('repo', 'state_root')).lower()}")
print(f"host_storage_state_benchmark_same_volume={str(same_volume('state_root', 'benchmark_root')).lower()}")
print(f"host_storage_state_artifact_same_volume={str(same_volume('state_root', 'artifact')).lower()}")
PY
}

emit_baseline_artifact_index() {
  local prefix="${out%.json}"
  local index="${prefix}.artifacts.txt"
  local artifact

  {
    echo "baseline_artifact_index=$index"
    echo "baseline_artifact=$out"
    echo "baseline_storage_context=${prefix}.storage.txt"
    echo "baseline_lifecycle_report=${prefix}.lifecycle.txt"
    echo "baseline_decision_report=${prefix}.decision.txt"
    echo "baseline_lifecycle_diagnostics=${prefix}.diagnostics.txt"
    echo "baseline_autoscale_contract=${prefix}.autoscale-contract.txt"
    echo "report_lifecycle_command=$fk benchmark report lifecycle $out"
    echo "report_decision_command=$fk benchmark report decision $out"
    for artifact in "${prefix}"*; do
      [[ -e "$artifact" ]] || continue
      echo "baseline_related_artifact=$artifact"
    done
  } >"$index"
}

emit_lifecycle_diagnostics() {
  local artifact="$1"

  python3 - "$artifact" <<'PY'
import json
import sys

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

summaries = {summary.get("metric"): summary for summary in evidence.get("summaries", [])}
metrics = [
    ("direct_exec_command_start", "debug.exec.direct_command_start_ms", 20.0),
    ("direct_exec_first_stdout", "debug.exec.direct_first_stdout_byte_ms", 25.0),
    ("envd_direct_health_rtt", "debug.envd.direct_health_rtt_ms", 5.0),
    ("envd_proxy_health_rtt", "debug.envd.proxy_health_rtt_ms", 5.0),
    ("raw_envd_direct_process_started", "debug.exec.raw_envd_direct_process_started_ms", 20.0),
    ("raw_envd_proxy_process_started", "debug.exec.raw_envd_proxy_process_started_ms", 20.0),
    ("raw_envd_direct_first_stdout", "debug.exec.raw_envd_direct_first_stdout_ms", 25.0),
    ("raw_envd_proxy_first_stdout", "debug.exec.raw_envd_proxy_first_stdout_ms", 25.0),
    ("core_stdio_prepare", "debug.exec.core_stdio_prepare_ms", 5.0),
    ("core_create_process_rpc", "debug.exec.core_create_process_rpc_ms", 15.0),
    ("core_start_process_rpc", "debug.exec.core_start_process_rpc_ms", 15.0),
    ("direct_core_start_process_rpc", "debug.exec.direct_core_start_process_rpc_ms", 15.0),
    ("shell_core_start_process_rpc", "debug.exec.shell_core_start_process_rpc_ms", 15.0),
    ("retained_shell_first_stdout", "debug.exec.retained_shell_first_stdout_ms", 25.0),
    ("retained_shell_first_stdout_c1", "debug.exec.retained_shell_first_stdout_c1_ms", 25.0),
    ("retained_shell_first_stdout_c2", "debug.exec.retained_shell_first_stdout_c2_ms", 25.0),
    ("retained_shell_first_stdout_c4", "debug.exec.retained_shell_first_stdout_c4_ms", 25.0),
    ("retained_shell_first_stdout_c8", "debug.exec.retained_shell_first_stdout_c8_ms", 25.0),
    ("aggregate_exec_command_start", "exec.command_start_ms", 20.0),
    ("aggregate_exec_first_stdout", "exec.first_stdout_byte_ms", 25.0),
    ("hot_to_first_stdout", "start.hot_to_first_stdout_ms", 75.0),
    ("shell_density_breakpoint", "density.max_active_before_hot_to_first_stdout_p95_doubles", 8.0),
    ("batch_100_small_commands", "exec.batch_100_small_commands_ms", 500.0),
    ("disk_sparse_bloat_after_trim", "disk.sparse_bloat_after_trim", 1.25),
    ("cleanup_leftover_bytes", "cleanup.leftover_bytes", 0.0),
    ("unknown_failure_rate", "reliability.unknown_failure_rate", 0.0),
]
for prefix, metric, target in metrics:
    summary = summaries.get(metric)
    if summary is None:
        print(f"{prefix}_metric={metric}")
        print(f"{prefix}_status=missing")
        continue
    value = summary.get("p95", summary.get("max"))
    count = summary.get("count", "missing")
    confidence = summary.get("percentile_availability", "missing")
    if metric in {
        "density.max_active_before_hot_to_first_stdout_p95_doubles",
    }:
        passed = value is not None and value >= target
    elif metric in {"cleanup.leftover_bytes", "reliability.unknown_failure_rate"}:
        passed = value == target
    else:
        passed = value is not None and value < target
    print(f"{prefix}_metric={metric}")
    print(f"{prefix}_count={count}")
    print(f"{prefix}_confidence={confidence}")
    print(f"{prefix}_p95={value}")
    print(f"{prefix}_target={target:g}")
    print(f"{prefix}_target_status={'pass' if passed else 'miss'}")

for concurrency in (1, 2, 4, 8, 16):
    for kind, metric in (
        ("sdk_create", f"debug.density.sdk_create_c{concurrency}_ms"),
        ("sdk_command", f"debug.density.sdk_command_c{concurrency}_ms"),
        ("hot_density", f"start.hot_to_first_stdout_density_c{concurrency}_ms"),
    ):
        summary = summaries.get(metric)
        if summary is None:
            continue
        print(f"density_{kind}_c{concurrency}_ms={summary.get('p95', summary.get('max'))}")
PY
}

emit_retained_shell_density_summary() {
  local artifact="$1"

  python3 - "$artifact" <<'PY'
import json
import math
import sys
from collections import defaultdict

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

samples_by_metric = defaultdict(list)
for sample in evidence.get("samples", []):
    samples_by_metric[sample.get("metric")].append(sample)

def nearest_rank(values, percentile):
    if not values:
        return "missing"
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, math.ceil(len(ordered) * percentile / 100) - 1))
    return f"{ordered[index]:.6f}"

print(f"retained_shell_density_artifact_kind={evidence.get('kind', 'missing')}")
print(f"retained_shell_density_sample_count={sum(len(samples) for samples in samples_by_metric.values())}")
for metric in sorted(samples_by_metric):
    samples = samples_by_metric[metric]
    values = [sample["value"] for sample in samples]
    tags = samples[0].get("tags", {})
    prefix = metric.removeprefix("debug.exec.").removesuffix("_ms")
    print(f"{prefix}_count={len(samples)}")
    print(f"{prefix}_confidence={tags.get('confidence', 'missing')}")
    print(f"{prefix}_levels={tags.get('concurrency_levels', 'missing')}")
    print(f"{prefix}_min_ms={min(values):.6f}")
    print(f"{prefix}_p50_ms={nearest_rank(values, 50)}")
    print(f"{prefix}_p95_ms={nearest_rank(values, 95)}")
    print(f"{prefix}_max_ms={max(values):.6f}")
    print(f"{prefix}_target_status={'pass' if max(values) < 25.0 else 'miss'}")
PY
}

emit_direct_exec_first_stdout_summary() {
  local artifact="$1"

  python3 - "$artifact" <<'PY'
import json
import math
import sys

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

samples = [
    sample for sample in evidence.get("samples", [])
    if sample.get("metric") == "debug.exec.direct_first_stdout_byte_ms"
]
values = sorted(sample["value"] for sample in samples)

def nearest_rank(percentile):
    if not values:
        return "missing"
    index = max(0, min(len(values) - 1, math.ceil(len(values) * percentile / 100) - 1))
    return f"{values[index]:.6f}"

tags = samples[0].get("tags", {}) if samples else {}
print(f"direct_exec_first_stdout_artifact_kind={evidence.get('kind', 'missing')}")
print(f"direct_exec_first_stdout_count={len(samples)}")
print(f"direct_exec_first_stdout_confidence={tags.get('confidence', 'missing')}")
print(f"direct_exec_first_stdout_min_ms={nearest_rank(0)}")
print(f"direct_exec_first_stdout_p50_ms={nearest_rank(50)}")
print(f"direct_exec_first_stdout_p95_ms={nearest_rank(95)}")
print(f"direct_exec_first_stdout_max_ms={nearest_rank(100)}")
print(f"direct_exec_first_stdout_target_status={'pass' if values and max(values) < 25.0 else 'miss'}")
PY
}

mkdir -p "$(dirname "$out")"

if pgrep -fl 'live_snapshot_restore|fk benchmark run|run-signed-live' >"$contention_file"; then
  echo "benchmark_contention=blocked"
  cat "$contention_file"
  exit 1
fi

if [[ -d "$restore_staging" ]]; then
  rm -rf "$restore_staging"
  echo "removed_stale_restore_staging=$restore_staging"
fi

cargo build --release -p firkin-cli

"$fk" benchmark doctor --mode signed-live --min-free-bytes "$min_free_bytes"
if [[ "${FIRKIN_BASELINE_NO_BUILD:-0}" == "1" ]]; then
  FIRKIN_LIVE_SHELL_DENSITY_LEVELS="$shell_density_levels" \
    FIRKIN_LIVE_RETAINED_SHELL_DENSITY_REPEATS="$retained_shell_density_repeats" \
    "$fk" benchmark run "$suite" --mode signed-live --duration "$duration" --out "$out" --no-build
else
  FIRKIN_LIVE_SHELL_DENSITY_LEVELS="$shell_density_levels" \
    FIRKIN_LIVE_RETAINED_SHELL_DENSITY_REPEATS="$retained_shell_density_repeats" \
    "$fk" benchmark run "$suite" --mode signed-live --duration "$duration" --out "$out"
fi
"$fk" benchmark baseline save "$out" --name "$name"
storage_context="$(emit_storage_context)"
printf '%s\n' "$storage_context" | tee "${out%.json}.storage.txt"
samples_out="${out%.json}.samples.json"
baseline_samples="$benchmark_root/baselines/${name}.samples.json"
if [[ -f "$samples_out" ]]; then
  mkdir -p "$(dirname "$baseline_samples")"
  cp "$samples_out" "$baseline_samples"
  echo "benchmark_baseline_samples=saved source=$samples_out path=$baseline_samples"
fi
traces_out="${out%.json}.traces.json"
baseline_traces="$benchmark_root/baselines/${name}.traces.json"
if [[ -f "$traces_out" ]]; then
  mkdir -p "$(dirname "$baseline_traces")"
  cp "$traces_out" "$baseline_traces"
  echo "benchmark_baseline_traces=saved source=$traces_out path=$baseline_traces"
fi
case "$suite" in
  agent-computer)
    product_pod_manifest="${out%.json}.product-pod-artifacts.txt"
    {
      echo "baseline_artifact=$out"
      echo "agent_computer_scorecard_report=${out%.json}.agent-computer-scorecard.txt"
      echo "product_pod_proof=$product_pod_proof"
      echo "autoscale_proof=$autoscale_proof"
      echo "baseline_sample_tier=$sample_tier"
      echo "baseline_default_sample_count=$default_sample_count"
      echo "agent_computer_min_samples=$agent_computer_min_samples"
      echo "product_pod_ready_deck_repeats=$ready_deck_repeats"
      echo "autoscale_repeats=$autoscale_repeats"
      echo "autoscale_min_samples=$autoscale_min_samples"
      echo "shell_density_levels=$shell_density_levels"
      echo "product_pod_ready_deck_density_levels=$ready_deck_density_levels"
      echo "product_pod_prestarted_agent_slot_density_levels=$prestarted_slot_density_levels"
      echo "rerun_command=scripts/run-firkin-decision-baseline.sh agent-computer"
      printf '%s\n' "$storage_context"
      emit_artifact_sidecars "agent_computer_scorecard" "$out"
    } >"$product_pod_manifest"
    "$fk" benchmark report-agent-computer-scorecard "$out" | tee "${out%.json}.agent-computer-scorecard.txt"
    "$fk" benchmark validate-agent-computer-scorecard --min-samples "$agent_computer_min_samples" "$out" | tee "${out%.json}.agent-computer-scorecard-validate.txt"
    set +e
    "$fk" benchmark validate-agent-computer-scorecard --min-samples "$agent_computer_min_samples" --require-promotable "$out" >"${out%.json}.agent-computer-scorecard-promotable.txt" 2>&1
    agent_computer_promotable_status=$?
    set -e
    if [[ "$agent_computer_promotable_status" == "0" ]]; then
      agent_computer_promotable="promotable"
    else
      agent_computer_promotable="blocked"
    fi
    cat "${out%.json}.agent-computer-scorecard-promotable.txt"
    set +e
    "$fk" benchmark validate-agent-computer-scorecard --min-samples "$agent_computer_min_samples" --require-snappy "$out" >"${out%.json}.agent-computer-scorecard-snappy.txt" 2>&1
    agent_computer_snappy_status=$?
    set -e
    if [[ "$agent_computer_snappy_status" == "0" ]]; then
      agent_computer_snappy="pass"
    else
      agent_computer_snappy="miss"
    fi
    cat "${out%.json}.agent-computer-scorecard-snappy.txt"
    {
      echo "agent_computer_scorecard_validate=${out%.json}.agent-computer-scorecard-validate.txt"
      echo "agent_computer_scorecard_promotable=${out%.json}.agent-computer-scorecard-promotable.txt"
      echo "agent_computer_scorecard_promotable_status=$agent_computer_promotable"
      echo "agent_computer_scorecard_snappy=${out%.json}.agent-computer-scorecard-snappy.txt"
      echo "agent_computer_scorecard_snappy_status=$agent_computer_snappy"
      echo "agent_computer_scorecard_min_samples=$agent_computer_min_samples"
    } >>"$product_pod_manifest"
    "$fk" benchmark report decision "$out" | tee "${out%.json}.decision.txt"
    if [[ -f "$traces_out" ]]; then
      "$fk" benchmark report-agent-computer-traces "$traces_out" | tee "${out%.json}.agent-computer-traces.txt"
      {
        echo "agent_computer_traces=$traces_out"
        echo "agent_computer_baseline_traces=$baseline_traces"
        echo "agent_computer_trace_report=${out%.json}.agent-computer-traces.txt"
        echo "agent_computer_trace_report_command=$fk benchmark report-agent-computer-traces $traces_out"
        echo "agent_computer_baseline_trace_report_command=$fk benchmark report-agent-computer-traces $baseline_traces"
      } >>"$product_pod_manifest"
    fi
    if [[ "$product_pod_proof" == "auto" || "$product_pod_proof" == "1" ]]; then
      product_pod_artifact="${out%.json}.product-pod-readiness.json"
      FIRKIN_LIVE_PRODUCT_POD_ARTIFACT="$product_pod_artifact" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_product_pod_readiness_writes_real_boundary_sample
      product_pod_trace_artifact="$(trace_artifact_for "$product_pod_artifact")"
      "$fk" benchmark report-agent-computer-traces "$product_pod_trace_artifact" | tee "${out%.json}.product-pod-readiness-traces.txt"
      {
        echo "product_pod_readiness_artifact=$product_pod_artifact"
        echo "product_pod_readiness_trace_artifact=$product_pod_trace_artifact"
        echo "product_pod_readiness_trace_report=${out%.json}.product-pod-readiness-traces.txt"
        echo "product_pod_readiness_report_command=$fk benchmark report-agent-computer-traces $product_pod_trace_artifact"
        emit_artifact_sidecars "product_pod_readiness" "$product_pod_artifact"
      } >>"$product_pod_manifest"
      echo "baseline_product_pod_readiness=$product_pod_artifact"
      echo "baseline_product_pod_readiness_trace_artifact=$product_pod_trace_artifact"
      echo "baseline_product_pod_readiness_trace_report=${out%.json}.product-pod-readiness-traces.txt"
      db_sidecar_artifact="${out%.json}.product-pod-db-sidecar.json"
      FIRKIN_LIVE_DB_SIDECAR_ARTIFACT="$db_sidecar_artifact" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_db_sidecar_readiness_writes_exact_sample
      db_sidecar_trace_artifact="$(trace_artifact_for "$db_sidecar_artifact")"
      "$fk" benchmark report-agent-computer-traces "$db_sidecar_trace_artifact" | tee "${out%.json}.product-pod-db-sidecar-traces.txt"
      db_sidecar_summary="$(python3 - "$db_sidecar_artifact" <<'PY'
import json
import sys

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

samples = evidence.get("samples", [])
sample = next((s for s in samples if s.get("metric") == "product.database_ready_ms"), None)
if sample is None:
    print("product_pod_db_sidecar_ms=missing")
else:
    tags = sample.get("tags", {})
    print(f"product_pod_db_sidecar_ms={sample.get('value', 'missing')}")
    print(f"product_pod_db_sidecar_confidence={tags.get('confidence', 'missing')}")
    print(f"product_pod_db_sidecar_measurement_boundary={tags.get('measurement_boundary', 'missing')}")
    print(f"product_pod_db_sidecar_database_boundary={tags.get('database_boundary', 'missing')}")
PY
)"
      printf '%s\n' "$db_sidecar_summary" | tee -a "$product_pod_manifest"
      {
        echo "product_pod_db_sidecar_artifact=$db_sidecar_artifact"
        echo "product_pod_db_sidecar_trace_artifact=$db_sidecar_trace_artifact"
        echo "product_pod_db_sidecar_trace_report=${out%.json}.product-pod-db-sidecar-traces.txt"
        echo "product_pod_db_sidecar_report_command=$fk benchmark report-agent-computer-traces $db_sidecar_trace_artifact"
        emit_artifact_sidecars "product_pod_db_sidecar" "$db_sidecar_artifact"
      } >>"$product_pod_manifest"
      echo "baseline_product_pod_db_sidecar=$db_sidecar_artifact"
      echo "baseline_product_pod_db_sidecar_trace_artifact=$db_sidecar_trace_artifact"
      echo "baseline_product_pod_db_sidecar_trace_report=${out%.json}.product-pod-db-sidecar-traces.txt"
      echo "baseline_product_pod_db_sidecar_report_command=$fk benchmark report-agent-computer-traces $db_sidecar_trace_artifact"
      browser_sidecar_artifact="${out%.json}.product-pod-browser-sidecar.json"
      FIRKIN_LIVE_BROWSER_SIDECAR_ARTIFACT="$browser_sidecar_artifact" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_browser_sidecar_readiness_writes_exact_sample
      browser_sidecar_trace_artifact="$(trace_artifact_for "$browser_sidecar_artifact")"
      "$fk" benchmark report-agent-computer-traces "$browser_sidecar_trace_artifact" | tee "${out%.json}.product-pod-browser-sidecar-traces.txt"
      browser_sidecar_summary="$(python3 - "$browser_sidecar_artifact" <<'PY'
import json
import sys

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

samples = evidence.get("samples", [])
sample = next((s for s in samples if s.get("metric") == "product.browser_ready_ms"), None)
if sample is None:
    print("product_pod_browser_sidecar_ms=missing")
else:
    tags = sample.get("tags", {})
    print(f"product_pod_browser_sidecar_ms={sample.get('value', 'missing')}")
    print(f"product_pod_browser_sidecar_confidence={tags.get('confidence', 'missing')}")
    print(f"product_pod_browser_sidecar_measurement_boundary={tags.get('measurement_boundary', 'missing')}")
    print(f"product_pod_browser_sidecar_browser_boundary={tags.get('browser_boundary', 'missing')}")
PY
)"
      printf '%s\n' "$browser_sidecar_summary" | tee -a "$product_pod_manifest"
      {
        echo "product_pod_browser_sidecar_artifact=$browser_sidecar_artifact"
        echo "product_pod_browser_sidecar_trace_artifact=$browser_sidecar_trace_artifact"
        echo "product_pod_browser_sidecar_trace_report=${out%.json}.product-pod-browser-sidecar-traces.txt"
        echo "product_pod_browser_sidecar_report_command=$fk benchmark report-agent-computer-traces $browser_sidecar_trace_artifact"
        emit_artifact_sidecars "product_pod_browser_sidecar" "$browser_sidecar_artifact"
      } >>"$product_pod_manifest"
      echo "baseline_product_pod_browser_sidecar=$browser_sidecar_artifact"
      echo "baseline_product_pod_browser_sidecar_trace_artifact=$browser_sidecar_trace_artifact"
      echo "baseline_product_pod_browser_sidecar_trace_report=${out%.json}.product-pod-browser-sidecar-traces.txt"
      echo "baseline_product_pod_browser_sidecar_report_command=$fk benchmark report-agent-computer-traces $browser_sidecar_trace_artifact"
      ready_deck_artifact="${out%.json}.product-pod-ready-deck.json"
      FIRKIN_LIVE_PRODUCT_POD_READY_DECK_ARTIFACT="$ready_deck_artifact" \
        FIRKIN_LIVE_PRODUCT_POD_READY_DECK_REPEATS="$ready_deck_repeats" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_product_pod_ready_deck_writes_real_boundary_resume_sample
      ready_deck_trace_artifact="$(trace_artifact_for "$ready_deck_artifact")"
      "$fk" benchmark report-agent-computer-traces "$ready_deck_trace_artifact" | tee "${out%.json}.product-pod-ready-deck-traces.txt"
      ready_deck_summary="$(python3 - "$ready_deck_artifact" <<'PY'
import json
import math
import sys

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

samples = evidence.get("samples", [])
values = sorted(sample["value"] for sample in samples)

def nearest_rank(percentile):
    if not values:
        return "missing"
    index = max(0, min(len(values) - 1, math.ceil(len(values) * percentile / 100) - 1))
    return f"{values[index]:.6f}"

confidences = sorted({
    sample.get("tags", {}).get("confidence", "missing") for sample in samples
})
print(f"product_pod_ready_deck_samples={len(samples)}")
print(f"product_pod_ready_deck_confidence={','.join(confidences) if confidences else 'missing'}")
print(f"product_pod_ready_deck_p50_ms={nearest_rank(50)}")
print(f"product_pod_ready_deck_p90_ms={nearest_rank(90)}")
print(f"product_pod_ready_deck_p95_ms={nearest_rank(95)}")
print(f"product_pod_ready_deck_p99_ms={nearest_rank(99)}")
print(f"product_pod_ready_deck_max_ms={nearest_rank(100)}")
PY
)"
      printf '%s\n' "$ready_deck_summary" | tee -a "$product_pod_manifest"
      {
        echo "product_pod_ready_deck_artifact=$ready_deck_artifact"
        echo "product_pod_ready_deck_trace_artifact=$ready_deck_trace_artifact"
        echo "product_pod_ready_deck_trace_report=${out%.json}.product-pod-ready-deck-traces.txt"
        echo "product_pod_ready_deck_report_command=$fk benchmark report-agent-computer-traces $ready_deck_trace_artifact"
        emit_artifact_sidecars "product_pod_ready_deck" "$ready_deck_artifact"
      } >>"$product_pod_manifest"
      echo "baseline_product_pod_ready_deck=$ready_deck_artifact"
      echo "baseline_product_pod_ready_deck_trace_artifact=$ready_deck_trace_artifact"
      echo "baseline_product_pod_ready_deck_trace_report=${out%.json}.product-pod-ready-deck-traces.txt"
      echo "baseline_product_pod_ready_deck_report_command=$fk benchmark report-agent-computer-traces $ready_deck_trace_artifact"
      ready_deck_density_artifact="${out%.json}.product-pod-ready-deck-density.json"
      FIRKIN_LIVE_PRODUCT_POD_READY_DECK_DENSITY_ARTIFACT="$ready_deck_density_artifact" \
        FIRKIN_LIVE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS="$ready_deck_density_levels" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_product_pod_ready_deck_density_writes_breakpoint_sample
      ready_deck_density_trace_artifact="$(trace_artifact_for "$ready_deck_density_artifact")"
      "$fk" benchmark report-agent-computer-traces "$ready_deck_density_trace_artifact" | tee "${out%.json}.product-pod-ready-deck-density-traces.txt"
      ready_deck_density_summary="$(python3 - "$ready_deck_density_artifact" <<'PY'
import json
import sys

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

samples = evidence.get("samples", [])
sample = next((s for s in samples if s.get("metric") == "density.max_agent_computers_before_ready_p95_doubles"), None)
if sample is None:
    print("product_pod_ready_deck_density_breakpoint=missing")
else:
    tags = sample.get("tags", {})
    print(f"product_pod_ready_deck_density_breakpoint={sample.get('value', 'missing')}")
    print(f"product_pod_ready_deck_density_confidence={tags.get('confidence', 'missing')}")
    print(f"product_pod_ready_deck_density_levels={tags.get('concurrency_levels', 'missing')}")
    print(f"product_pod_ready_deck_density_baseline_p95_ms={tags.get('baseline_p95_ms', 'missing')}")
    print(f"product_pod_ready_deck_density_threshold_p95_ms={tags.get('threshold_p95_ms', 'missing')}")
for sample in samples:
    metric = sample.get("metric", "")
    prefix = "debug.product.agent_computer_ready_deck_c"
    suffix = "_ms"
    if metric.startswith(prefix) and metric.endswith(suffix):
        level = metric[len(prefix):-len(suffix)]
        print(f"product_pod_ready_deck_density_c{level}_ms={sample.get('value', 'missing')}")
PY
)"
	      printf '%s\n' "$ready_deck_density_summary" | tee -a "$product_pod_manifest"
	      ready_deck_density_breakpoint="$(printf '%s\n' "$ready_deck_density_summary" | sed -n 's/^product_pod_ready_deck_density_breakpoint=//p')"
	      emit_density_status "product_pod_ready_deck_density" "$ready_deck_density_breakpoint" 4 | tee -a "$product_pod_manifest"
	      {
	        echo "product_pod_ready_deck_density_artifact=$ready_deck_density_artifact"
        echo "product_pod_ready_deck_density_trace_artifact=$ready_deck_density_trace_artifact"
        echo "product_pod_ready_deck_density_trace_report=${out%.json}.product-pod-ready-deck-density-traces.txt"
        echo "product_pod_ready_deck_density_report_command=$fk benchmark report-agent-computer-traces $ready_deck_density_trace_artifact"
        emit_artifact_sidecars "product_pod_ready_deck_density" "$ready_deck_density_artifact"
      } >>"$product_pod_manifest"
      echo "baseline_product_pod_ready_deck_density=$ready_deck_density_artifact"
      echo "baseline_product_pod_ready_deck_density_trace_artifact=$ready_deck_density_trace_artifact"
      echo "baseline_product_pod_ready_deck_density_trace_report=${out%.json}.product-pod-ready-deck-density-traces.txt"
      echo "baseline_product_pod_ready_deck_density_report_command=$fk benchmark report-agent-computer-traces $ready_deck_density_trace_artifact"
      prestarted_slot_density_artifact="${out%.json}.product-pod-prestarted-agent-slot-density.json"
      FIRKIN_LIVE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_ARTIFACT="$prestarted_slot_density_artifact" \
        FIRKIN_LIVE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS="$prestarted_slot_density_levels" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_product_pod_prestarted_agent_slot_density_writes_breakpoint_sample
      prestarted_slot_density_trace_artifact="$(trace_artifact_for "$prestarted_slot_density_artifact")"
      "$fk" benchmark report-agent-computer-traces "$prestarted_slot_density_trace_artifact" | tee "${out%.json}.product-pod-prestarted-agent-slot-density-traces.txt"
      prestarted_slot_density_summary="$(python3 - "$prestarted_slot_density_artifact" <<'PY'
import json
import sys

artifact = sys.argv[1]
with open(artifact) as file:
    evidence = json.load(file)

samples = evidence.get("samples", [])
sample = next((s for s in samples if s.get("metric") == "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles"), None)
if sample is None:
    print("product_pod_prestarted_agent_slot_density_breakpoint=missing")
else:
    tags = sample.get("tags", {})
    print(f"product_pod_prestarted_agent_slot_density_breakpoint={sample.get('value', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_confidence={tags.get('confidence', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_levels={tags.get('concurrency_levels', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_prestarted_slots={tags.get('prestarted_slots', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_measurement_boundary={tags.get('measurement_boundary', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_slot_surface={tags.get('slot_surface', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_excludes_container_add={tags.get('excludes_container_add', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_ready_signal={tags.get('ready_signal', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_output_wait_preserved={tags.get('output_wait_preserved', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_baseline_p95_ms={tags.get('baseline_p95_ms', 'missing')}")
    print(f"product_pod_prestarted_agent_slot_density_threshold_p95_ms={tags.get('threshold_p95_ms', 'missing')}")
for sample in samples:
    metric = sample.get("metric", "")
    prefix = "debug.product.prestarted_agent_slot_checkout_c"
    suffix = "_ms"
    if metric.startswith(prefix) and metric.endswith(suffix):
        level = metric[len(prefix):-len(suffix)]
        print(f"product_pod_prestarted_agent_slot_density_c{level}_ms={sample.get('value', 'missing')}")
PY
)"
	      printf '%s\n' "$prestarted_slot_density_summary" | tee -a "$product_pod_manifest"
	      prestarted_slot_density_breakpoint="$(printf '%s\n' "$prestarted_slot_density_summary" | sed -n 's/^product_pod_prestarted_agent_slot_density_breakpoint=//p')"
	      emit_density_status "product_pod_prestarted_agent_slot_density" "$prestarted_slot_density_breakpoint" 4 | tee -a "$product_pod_manifest"
	      {
	        echo "product_pod_prestarted_agent_slot_density_artifact=$prestarted_slot_density_artifact"
        echo "product_pod_prestarted_agent_slot_density_trace_artifact=$prestarted_slot_density_trace_artifact"
        echo "product_pod_prestarted_agent_slot_density_trace_report=${out%.json}.product-pod-prestarted-agent-slot-density-traces.txt"
        echo "product_pod_prestarted_agent_slot_density_report_command=$fk benchmark report-agent-computer-traces $prestarted_slot_density_trace_artifact"
        emit_artifact_sidecars "product_pod_prestarted_agent_slot_density" "$prestarted_slot_density_artifact"
      } >>"$product_pod_manifest"
      echo "baseline_product_pod_prestarted_agent_slot_density=$prestarted_slot_density_artifact"
      echo "baseline_product_pod_prestarted_agent_slot_density_trace_artifact=$prestarted_slot_density_trace_artifact"
      echo "baseline_product_pod_prestarted_agent_slot_density_trace_report=${out%.json}.product-pod-prestarted-agent-slot-density-traces.txt"
      echo "baseline_product_pod_prestarted_agent_slot_density_report_command=$fk benchmark report-agent-computer-traces $prestarted_slot_density_trace_artifact"
    fi
    if [[ "$autoscale_proof" == "auto" || "$autoscale_proof" == "1" ]]; then
      autoscale_artifact="${out%.json}.autoscale-scorecard.json"
      FIRKIN_LIVE_AUTOSCALE_ARTIFACT="$autoscale_artifact" \
        FIRKIN_LIVE_AUTOSCALE_REPEATS="$autoscale_repeats" \
        FIRKIN_LIVE_PRODUCT_POD_READY_DECK_DENSITY_LEVELS="$ready_deck_density_levels" \
        FIRKIN_LIVE_PRODUCT_POD_PRESTARTED_AGENT_SLOT_DENSITY_LEVELS="$prestarted_slot_density_levels" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_autoscale_scorecard_writes_product_path_artifact
	      "$fk" benchmark report-autoscale-scorecard "$autoscale_artifact" | tee "${out%.json}.autoscale-scorecard.txt"
	      autoscale_summary="$(emit_autoscale_summary "$autoscale_artifact")"
	      printf '%s\n' "$autoscale_summary" | tee -a "$product_pod_manifest"
	      "$fk" benchmark validate-autoscale-scorecard --min-samples "$autoscale_min_samples" "$autoscale_artifact" | tee "${out%.json}.autoscale-scorecard-validate.txt"
      set +e
      "$fk" benchmark validate-autoscale-scorecard --min-samples "$autoscale_min_samples" --require-promotable "$autoscale_artifact" >"${out%.json}.autoscale-scorecard-promotable.txt" 2>&1
      autoscale_promotable_status=$?
      set -e
      if [[ "$autoscale_promotable_status" == "0" ]]; then
        autoscale_promotable="promotable"
      else
        autoscale_promotable="blocked"
      fi
      cat "${out%.json}.autoscale-scorecard-promotable.txt"
      set +e
      "$fk" benchmark validate-autoscale-scorecard --min-samples "$autoscale_min_samples" --require-snappy "$autoscale_artifact" >"${out%.json}.autoscale-scorecard-snappy.txt" 2>&1
      autoscale_snappy_status=$?
      set -e
      if [[ "$autoscale_snappy_status" == "0" ]]; then
        autoscale_snappy="pass"
      else
        autoscale_snappy="miss"
      fi
      cat "${out%.json}.autoscale-scorecard-snappy.txt"
      {
        echo "autoscale_scorecard_artifact=$autoscale_artifact"
        echo "autoscale_scorecard_report=${out%.json}.autoscale-scorecard.txt"
        echo "autoscale_scorecard_validate=${out%.json}.autoscale-scorecard-validate.txt"
        echo "autoscale_scorecard_promotable=${out%.json}.autoscale-scorecard-promotable.txt"
        echo "autoscale_scorecard_promotable_status=$autoscale_promotable"
        echo "autoscale_scorecard_snappy=${out%.json}.autoscale-scorecard-snappy.txt"
        echo "autoscale_scorecard_snappy_status=$autoscale_snappy"
        echo "autoscale_scorecard_min_samples=$autoscale_min_samples"
        emit_artifact_sidecars "autoscale_scorecard" "$autoscale_artifact"
      } >>"$product_pod_manifest"
      echo "baseline_autoscale_scorecard=$autoscale_artifact"
      echo "baseline_autoscale_scorecard_report=${out%.json}.autoscale-scorecard.txt"
      echo "baseline_autoscale_scorecard_validate=${out%.json}.autoscale-scorecard-validate.txt"
      echo "baseline_autoscale_scorecard_promotable=${out%.json}.autoscale-scorecard-promotable.txt"
      echo "baseline_autoscale_scorecard_promotable_status=$autoscale_promotable"
      echo "baseline_autoscale_scorecard_snappy=${out%.json}.autoscale-scorecard-snappy.txt"
      echo "baseline_autoscale_scorecard_snappy_status=$autoscale_snappy"
      echo "baseline_autoscale_scorecard_min_samples=$autoscale_min_samples"
      emit_artifact_sidecars "baseline_autoscale_scorecard" "$autoscale_artifact"
    fi
    "$fk" benchmark proof product-pod-ready-deck --from "$product_pod_manifest" --out "${out%.json}.product-pod-ready-deck-proof.html"
    ;;
  *)
    "$fk" benchmark report lifecycle "$out" | tee "${out%.json}.lifecycle.txt"
    "$fk" benchmark report decision "$out" | tee "${out%.json}.decision.txt"
    emit_lifecycle_diagnostics "$out" | tee "${out%.json}.diagnostics.txt"
    if [[ "$suite" == "agent-core" && ( "$direct_exec_first_stdout_proof" == "auto" || "$direct_exec_first_stdout_proof" == "1" ) ]]; then
      direct_exec_first_stdout_artifact="${out%.json}.direct-exec-first-stdout.json"
      FIRKIN_LIVE_DIRECT_EXEC_FIRST_STDOUT_ARTIFACT="$direct_exec_first_stdout_artifact" \
        FIRKIN_LIVE_DIRECT_EXEC_FIRST_STDOUT_REPEATS="$direct_exec_first_stdout_proof_repeats" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_direct_exec_first_stdout_writes_repeat_samples
      emit_direct_exec_first_stdout_summary "$direct_exec_first_stdout_artifact" | tee "${out%.json}.direct-exec-first-stdout.txt"
      {
        echo "direct_exec_first_stdout_artifact=$direct_exec_first_stdout_artifact"
        echo "direct_exec_first_stdout_report=${out%.json}.direct-exec-first-stdout.txt"
        echo "direct_exec_first_stdout_report_command=cat ${out%.json}.direct-exec-first-stdout.txt"
      } >>"${out%.json}.direct-exec-first-stdout.txt"
      echo "baseline_direct_exec_first_stdout=$direct_exec_first_stdout_artifact"
      echo "baseline_direct_exec_first_stdout_report=${out%.json}.direct-exec-first-stdout.txt"
    fi
    if [[ "$suite" == "agent-core" && ( "$retained_shell_density_proof" == "auto" || "$retained_shell_density_proof" == "1" ) ]]; then
      retained_shell_density_artifact="${out%.json}.retained-shell-density.json"
      FIRKIN_LIVE_RETAINED_SHELL_DENSITY_ARTIFACT="$retained_shell_density_artifact" \
        FIRKIN_LIVE_RETAINED_SHELL_DENSITY_LEVELS="$retained_shell_density_proof_levels" \
        FIRKIN_LIVE_RETAINED_SHELL_DENSITY_REPEATS="$retained_shell_density_proof_repeats" \
        CARGO_INCREMENTAL=0 \
        scripts/run-signed-live-runtime-test.sh \
        live_runtime_retained_shell_density_reuse_writes_repeat_samples
      emit_retained_shell_density_summary "$retained_shell_density_artifact" | tee "${out%.json}.retained-shell-density.txt"
      {
        echo "retained_shell_density_artifact=$retained_shell_density_artifact"
        echo "retained_shell_density_report=${out%.json}.retained-shell-density.txt"
        echo "retained_shell_density_report_command=cat ${out%.json}.retained-shell-density.txt"
      } >>"${out%.json}.retained-shell-density.txt"
      echo "baseline_retained_shell_density=$retained_shell_density_artifact"
      echo "baseline_retained_shell_density_report=${out%.json}.retained-shell-density.txt"
    fi
    ;;
esac
"$fk" benchmark autoscale-contract | tee "${out%.json}.autoscale-contract.txt"

if [[ "${FIRKIN_BASELINE_KEEP_RESTORE_STAGING:-0}" != "1" && -d "$restore_staging" ]]; then
  rm -rf "$restore_staging"
  echo "removed_restore_staging_after_success=$restore_staging"
fi

emit_baseline_artifact_index

echo "baseline_artifact=$out"
echo "baseline_artifact_index=${out%.json}.artifacts.txt"
echo "baseline_storage_context=${out%.json}.storage.txt"
echo "baseline_sample_tier=$sample_tier"
echo "baseline_default_sample_count=$default_sample_count"
echo "baseline_shell_density_levels=$shell_density_levels"
echo "baseline_retained_shell_density_repeats=$retained_shell_density_repeats"
echo "baseline_retained_shell_density_proof=$retained_shell_density_proof"
echo "baseline_retained_shell_density_proof_levels=$retained_shell_density_proof_levels"
echo "baseline_retained_shell_density_proof_repeats=$retained_shell_density_proof_repeats"
echo "baseline_direct_exec_first_stdout_proof=$direct_exec_first_stdout_proof"
echo "baseline_direct_exec_first_stdout_proof_repeats=$direct_exec_first_stdout_proof_repeats"
case "$suite" in
  agent-computer)
	    echo "baseline_agent_computer_scorecard_report=${out%.json}.agent-computer-scorecard.txt"
	    echo "baseline_agent_computer_scorecard_validate=${out%.json}.agent-computer-scorecard-validate.txt"
	    echo "baseline_agent_computer_scorecard_promotable=${out%.json}.agent-computer-scorecard-promotable.txt"
	    echo "baseline_agent_computer_scorecard_promotable_status=$agent_computer_promotable"
	    echo "baseline_agent_computer_scorecard_snappy=${out%.json}.agent-computer-scorecard-snappy.txt"
	    echo "baseline_agent_computer_scorecard_snappy_status=$agent_computer_snappy"
	    echo "baseline_agent_computer_scorecard_min_samples=$agent_computer_min_samples"
	    echo "baseline_decision_report=${out%.json}.decision.txt"
	    echo "metric_index=agent_computer_ready_metric=product.agent_computer_ready_ms ready_deck_metric=product.agent_computer_resume_ms retained_batch_metric=exec.batch_100_small_commands_ms product_density_metric=density.max_agent_computers_before_ready_p95_doubles prestarted_slot_density_metric=density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles autoscale_metrics=autoscale.ready_queue_hit_rate_pct,autoscale.safe_spare_limiting_utilization_pct,autoscale.pressure_to_safe_floor_ms,autoscale.pressure_clear_to_ready_target_ms,autoscale.reserve_floor_violations,autoscale.active_evictions_due_to_pool_pressure"
    if [[ -f "$samples_out" ]]; then
      echo "baseline_agent_computer_samples=$baseline_samples"
    fi
    if [[ -f "$traces_out" ]]; then
      echo "baseline_agent_computer_traces=$baseline_traces"
      echo "baseline_agent_computer_trace_report=${out%.json}.agent-computer-traces.txt"
      echo "baseline_agent_computer_trace_report_command=$fk benchmark report-agent-computer-traces $baseline_traces"
    fi
    if [[ -f "${out%.json}.product-pod-readiness-traces.txt" ]]; then
      product_pod_artifact="${out%.json}.product-pod-readiness.json"
      product_pod_trace_artifact="$(trace_artifact_for "$product_pod_artifact")"
      echo "baseline_product_pod_readiness=$product_pod_artifact"
      echo "baseline_product_pod_readiness_trace_artifact=$product_pod_trace_artifact"
      echo "baseline_product_pod_readiness_trace_report=${out%.json}.product-pod-readiness-traces.txt"
      echo "baseline_product_pod_readiness_report_command=$fk benchmark report-agent-computer-traces $product_pod_trace_artifact"
      emit_artifact_sidecars "baseline_product_pod_readiness" "$product_pod_artifact"
    fi
    if [[ -f "${out%.json}.product-pod-db-sidecar-traces.txt" ]]; then
      db_sidecar_artifact="${out%.json}.product-pod-db-sidecar.json"
      db_sidecar_trace_artifact="$(trace_artifact_for "$db_sidecar_artifact")"
      echo "baseline_product_pod_db_sidecar=$db_sidecar_artifact"
      echo "baseline_product_pod_db_sidecar_trace_artifact=$db_sidecar_trace_artifact"
      echo "baseline_product_pod_db_sidecar_trace_report=${out%.json}.product-pod-db-sidecar-traces.txt"
      echo "baseline_product_pod_db_sidecar_report_command=$fk benchmark report-agent-computer-traces $db_sidecar_trace_artifact"
      emit_artifact_sidecars "baseline_product_pod_db_sidecar" "$db_sidecar_artifact"
    fi
    if [[ -f "${out%.json}.product-pod-browser-sidecar-traces.txt" ]]; then
      browser_sidecar_artifact="${out%.json}.product-pod-browser-sidecar.json"
      browser_sidecar_trace_artifact="$(trace_artifact_for "$browser_sidecar_artifact")"
      echo "baseline_product_pod_browser_sidecar=$browser_sidecar_artifact"
      echo "baseline_product_pod_browser_sidecar_trace_artifact=$browser_sidecar_trace_artifact"
      echo "baseline_product_pod_browser_sidecar_trace_report=${out%.json}.product-pod-browser-sidecar-traces.txt"
      echo "baseline_product_pod_browser_sidecar_report_command=$fk benchmark report-agent-computer-traces $browser_sidecar_trace_artifact"
      emit_artifact_sidecars "baseline_product_pod_browser_sidecar" "$browser_sidecar_artifact"
    fi
    if [[ -f "${out%.json}.product-pod-ready-deck-traces.txt" ]]; then
      ready_deck_artifact="${out%.json}.product-pod-ready-deck.json"
      ready_deck_trace_artifact="$(trace_artifact_for "$ready_deck_artifact")"
      echo "baseline_product_pod_ready_deck=$ready_deck_artifact"
      echo "baseline_product_pod_ready_deck_trace_artifact=$ready_deck_trace_artifact"
      echo "baseline_product_pod_ready_deck_trace_report=${out%.json}.product-pod-ready-deck-traces.txt"
      echo "baseline_product_pod_ready_deck_report_command=$fk benchmark report-agent-computer-traces $ready_deck_trace_artifact"
      emit_artifact_sidecars "baseline_product_pod_ready_deck" "$ready_deck_artifact"
    fi
    if [[ -f "${out%.json}.product-pod-ready-deck-density-traces.txt" ]]; then
      ready_deck_density_artifact="${out%.json}.product-pod-ready-deck-density.json"
      ready_deck_density_trace_artifact="$(trace_artifact_for "$ready_deck_density_artifact")"
      echo "baseline_product_pod_ready_deck_density=$ready_deck_density_artifact"
      echo "baseline_product_pod_ready_deck_density_trace_artifact=$ready_deck_density_trace_artifact"
      echo "baseline_product_pod_ready_deck_density_trace_report=${out%.json}.product-pod-ready-deck-density-traces.txt"
      echo "baseline_product_pod_ready_deck_density_report_command=$fk benchmark report-agent-computer-traces $ready_deck_density_trace_artifact"
      emit_artifact_sidecars "baseline_product_pod_ready_deck_density" "$ready_deck_density_artifact"
    fi
    if [[ -f "${out%.json}.product-pod-prestarted-agent-slot-density-traces.txt" ]]; then
      prestarted_slot_density_artifact="${out%.json}.product-pod-prestarted-agent-slot-density.json"
      prestarted_slot_density_trace_artifact="$(trace_artifact_for "$prestarted_slot_density_artifact")"
      echo "baseline_product_pod_prestarted_agent_slot_density=$prestarted_slot_density_artifact"
      echo "baseline_product_pod_prestarted_agent_slot_density_trace_artifact=$prestarted_slot_density_trace_artifact"
      echo "baseline_product_pod_prestarted_agent_slot_density_trace_report=${out%.json}.product-pod-prestarted-agent-slot-density-traces.txt"
      echo "baseline_product_pod_prestarted_agent_slot_density_report_command=$fk benchmark report-agent-computer-traces $prestarted_slot_density_trace_artifact"
      emit_artifact_sidecars "baseline_product_pod_prestarted_agent_slot_density" "$prestarted_slot_density_artifact"
    fi
    if [[ -f "${out%.json}.product-pod-artifacts.txt" ]]; then
      echo "baseline_product_pod_artifact_manifest=${out%.json}.product-pod-artifacts.txt"
      echo "baseline_product_pod_ready_deck_proof=${out%.json}.product-pod-ready-deck-proof.html"
    fi
    if [[ -f "${out%.json}.autoscale-scorecard.txt" ]]; then
      echo "baseline_autoscale_scorecard=${out%.json}.autoscale-scorecard.json"
      echo "baseline_autoscale_scorecard_report=${out%.json}.autoscale-scorecard.txt"
      echo "baseline_autoscale_scorecard_validate=${out%.json}.autoscale-scorecard-validate.txt"
      echo "baseline_autoscale_scorecard_promotable=${out%.json}.autoscale-scorecard-promotable.txt"
      echo "baseline_autoscale_scorecard_snappy=${out%.json}.autoscale-scorecard-snappy.txt"
      if grep -q 'not promotion-grade' "${out%.json}.autoscale-scorecard-promotable.txt"; then
        echo "baseline_autoscale_scorecard_promotable_status=blocked"
      else
        echo "baseline_autoscale_scorecard_promotable_status=promotable"
      fi
      if grep -q 'not snappy' "${out%.json}.autoscale-scorecard-snappy.txt"; then
        echo "baseline_autoscale_scorecard_snappy_status=miss"
      else
        echo "baseline_autoscale_scorecard_snappy_status=pass"
      fi
    fi
    echo "baseline_product_pod_ready_deck_density_levels=$ready_deck_density_levels"
    echo "baseline_product_pod_prestarted_agent_slot_density_levels=$prestarted_slot_density_levels"
    ;;
  *)
    echo "baseline_lifecycle_report=${out%.json}.lifecycle.txt"
    echo "baseline_decision_report=${out%.json}.decision.txt"
    echo "baseline_lifecycle_diagnostics=${out%.json}.diagnostics.txt"
    if [[ -f "${out%.json}.direct-exec-first-stdout.txt" ]]; then
      echo "baseline_direct_exec_first_stdout=${out%.json}.direct-exec-first-stdout.json"
      echo "baseline_direct_exec_first_stdout_report=${out%.json}.direct-exec-first-stdout.txt"
    fi
    if [[ -f "${out%.json}.retained-shell-density.txt" ]]; then
      echo "baseline_retained_shell_density=${out%.json}.retained-shell-density.json"
      echo "baseline_retained_shell_density_report=${out%.json}.retained-shell-density.txt"
    fi
    ;;
esac
echo "baseline_autoscale_contract=${out%.json}.autoscale-contract.txt"
