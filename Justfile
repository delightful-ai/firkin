set shell := ["bash", "-cu"]

live-runtime test:
    scripts/run-signed-live-runtime-test.sh "{{test}}"

live-runtime-concurrent-stdin:
    scripts/run-signed-live-runtime-test.sh live_vendored_sdk_retains_concurrent_stdin_through_firkin_domain_proxy

live-runtime-concurrent-commands:
    scripts/run-signed-live-runtime-test.sh live_vendored_sdk_runs_concurrent_commands_through_firkin_domain_proxy

live-runtime-code-interpreter-probe:
    scripts/run-signed-live-runtime-test.sh live_vendored_sdk_reaches_code_interpreter_probe_through_firkin_domain_proxy

live-runtime-code-interpreter-execute:
    scripts/run-signed-live-runtime-test.sh live_code_interpreter_execute_runs_bash_through_firkin_domain_proxy

live-runtime-code-interpreter-concurrent-execute:
    scripts/run-signed-live-runtime-test.sh live_code_interpreter_execute_routes_two_active_sandboxes

live-runtime-code-interpreter-python-context:
    scripts/run-signed-live-runtime-test.sh live_code_interpreter_python_context_survives_execute_requests

live-runtime-concurrent-files:
    scripts/run-signed-live-runtime-test.sh live_vendored_sdk_uses_concurrent_filesystems_through_firkin_domain_proxy

live-runtime-warm-pool:
    scripts/run-signed-live-runtime-test.sh live_snapshot_warm_pool_checkouts_retained_session

live-runtime-warm-pool-product-route:
    scripts/run-signed-live-runtime-test.sh live_vendored_sdk_uses_prewarmed_template_through_firkin_domain_proxy

live-runtime-host-scan:
    scripts/run-signed-live-runtime-test.sh live_firkin_runtime_adapter_publishes_active_vz_marker_for_host_scan

live-runtime-stuck-vm-cleanup:
    scripts/run-signed-live-runtime-test.sh live_stuck_vm_cleanup_terminates_marked_host_process

live-runtime-hygiene-pressure:
    scripts/run-signed-live-runtime-test.sh live_runtime_hygiene_maintenance_reclaims_unreferenced_vz_snapshot_directory_and_rotates_log

live-runtime-template-build:
    scripts/run-signed-live-runtime-test.sh live_template_build_snapshot_clones_repo_and_restores

live-runtime-freshness-sync:
    scripts/run-signed-live-runtime-test.sh live_runtime_freshness_sync_fast_forwards_public_repo_after_restore

live-runtime-freshness-product-route:
    scripts/run-signed-live-runtime-test.sh live_freshness_sync_product_route_fast_forwards_public_repo_after_restore

live-runtime-continuation:
    scripts/run-signed-live-runtime-test.sh live_continuation_snapshot_capture_restores_session_state

live-runtime-followup-route:
    scripts/run-signed-live-runtime-test.sh live_followup_product_route_restores_continuation_snapshot

live-runtime-create-snapshot-route:
    scripts/run-signed-live-runtime-test.sh live_create_snapshot_product_route_restores_followup_state

live-runtime-integrity:
    scripts/run-signed-live-runtime-test.sh live_runtime_adapter_rejects_mutated_snapshot_integrity_before_restore

live-runtime-soak-smoke:
    mkdir -p target/firkin-live-evidence
    FIRKIN_LIVE_SOAK_SECONDS=1 FIRKIN_LIVE_SOAK_ARTIFACT=target/firkin-live-evidence/live-soak-evidence-smoke.json FIRKIN_LIVE_SOAK_BENCHMARK_ARTIFACT=target/firkin-live-evidence/live-benchmark-evidence.json scripts/run-signed-live-runtime-test.sh live_product_route_soak_writes_evidence_artifact

live-runtime-soak-24h:
    mkdir -p target/firkin-live-evidence
    just live-runtime-benchmark-slo-gate
    FIRKIN_LIVE_SOAK_SECONDS=86400 FIRKIN_LIVE_SOAK_ARTIFACT=target/firkin-live-evidence/live-soak-evidence-24h.json FIRKIN_LIVE_SOAK_BENCHMARK_ARTIFACT=target/firkin-live-evidence/live-benchmark-evidence.json scripts/run-signed-live-runtime-test.sh live_product_route_soak_writes_evidence_artifact
    cargo run -q -p firkin-cli -- benchmark validate-soak target/firkin-live-evidence/live-soak-evidence-24h.json

live-runtime-benchmark-evidence:
    scripts/run-signed-live-runtime-test.sh live_runtime_benchmark_evidence_writes_required_lifecycle_artifact

live-runtime-benchmark-slo-gate:
    mkdir -p target/firkin-live-evidence
    FIRKIN_LIVE_BENCHMARK_ARTIFACT=target/firkin-live-evidence/live-benchmark-evidence.json FIRKIN_LIVE_RESTORE_TIMING_ARTIFACT=target/firkin-live-evidence/restore-timings.json scripts/run-signed-live-runtime-test.sh live_runtime_benchmark_evidence_writes_required_lifecycle_artifact
    cargo run -q -p firkin-cli -- benchmark validate-lifecycle-slo target/firkin-live-evidence/live-benchmark-evidence.json --min-samples 1

live-runtime-benchmark-representative:
    mkdir -p target/firkin-live-evidence
    FIRKIN_LIVE_BENCHMARK_REPEATS=3 FIRKIN_LIVE_BENCHMARK_ARTIFACT=target/firkin-live-evidence/live-benchmark-evidence-representative.json FIRKIN_LIVE_RESTORE_TIMING_ARTIFACT=target/firkin-live-evidence/restore-timings-representative.json scripts/run-signed-live-runtime-test.sh live_runtime_benchmark_evidence_writes_required_lifecycle_artifact
    cargo run -q -p firkin-cli -- benchmark validate-lifecycle-slo target/firkin-live-evidence/live-benchmark-evidence-representative.json --min-samples 3

live-runtime-overhead-evidence:
    mkdir -p target/firkin-live-evidence
    FIRKIN_LIVE_OVERHEAD_ARTIFACT=target/firkin-live-evidence/live-overhead-evidence.json scripts/run-signed-live-runtime-test.sh live_runtime_overhead_evidence_writes_required_overhead_artifact

live-runtime-overhead-slo-gate:
    mkdir -p target/firkin-live-evidence
    FIRKIN_LIVE_OVERHEAD_ARTIFACT=target/firkin-live-evidence/live-overhead-evidence.json scripts/run-signed-live-runtime-test.sh live_runtime_overhead_evidence_writes_required_overhead_artifact
    cargo run -q -p firkin-cli -- benchmark validate-overhead-slo target/firkin-live-evidence/live-overhead-evidence.json --min-samples 1

live-runtime-overhead-representative:
    mkdir -p target/firkin-live-evidence
    FIRKIN_LIVE_OVERHEAD_REPEATS=3 FIRKIN_LIVE_OVERHEAD_ARTIFACT=target/firkin-live-evidence/live-overhead-evidence-representative.json scripts/run-signed-live-runtime-test.sh live_runtime_overhead_evidence_writes_required_overhead_artifact

live-apple-vz-core-smoke:
    scripts/run-signed-live-runtime-test.sh --package firkin-core --features real-vm --test builder live_busybox_output_runs_in_implicit_vm

live-apple-vz-benchmark-suite:
    mkdir -p target/firkin-live-evidence
    just live-runtime-benchmark-representative
    just live-runtime-overhead-representative
    just live-runtime-soak-smoke
    just live-runtime-pod-asif

live-runtime-pod-autoscale:
    mkdir -p target/firkin-live-evidence
    FIRKIN_POD_AUTOSCALE_ARTIFACT="$PWD/target/firkin-live-evidence/pod-autoscale-evidence.json" scripts/run-signed-live-runtime-test.sh --test pod_autoscale live_apple_vz_product_pod_autoscales_64_shared_template_containers
    FIRKIN_POD_AUTOSCALE_ARTIFACT="$PWD/target/firkin-live-evidence/pod-autoscale-evidence.json" cargo test -q -p firkin-runtime --test pod_autoscale pod_autoscale_evidence_artifact_at_env_path_is_valid -- --ignored --exact

live-runtime-pod-asif:
    scripts/run-signed-live-runtime-test.sh --test product_pods live_apple_vz_product_pod_route_uses_asif_pod_store
