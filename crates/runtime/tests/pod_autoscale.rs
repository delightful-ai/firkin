//! Signed live pod autoscaling evidence.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use firkin_single_node::{AppleVzLocalRuntimeDriver, LogStore, PortRegistry};
use serde::{Deserialize, Serialize};
use {
    firkin_e2b_server::LocalRuntimeBackend,
    firkin_e2b_wire::{
        PodContainerCreateRequest, PodCreateRequest, PodEmptyDir, PodStoreOptions,
        PodVolumeMountRequest,
    },
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct LatencyStats {
    p50: u128,
    p95: u128,
    max: u128,
}

#[derive(Debug, Deserialize, Serialize)]
struct PodAutoscaleEvidence {
    containers: usize,
    image: String,
    pod_store_bytes: u64,
    shared_rootfs: bool,
    template_cache_entries: usize,
    image_format: String,
    create_pod_ms: u128,
    add_container_ms: LatencyStats,
    remove_container_ms: LatencyStats,
    guest_usage_before_remove_bytes: u64,
    guest_usage_after_remove_bytes: u64,
    host_allocated_before_trim_bytes: u64,
    host_allocated_after_trim_bytes: u64,
    failures: Vec<String>,
}

#[tokio::test]
#[ignore = "signed live Apple/VZ pod autoscale benchmark; boots a VM and creates many containers"]
#[allow(clippy::too_many_lines)]
async fn live_apple_vz_product_pod_autoscales_64_shared_template_containers() {
    let containers = env_usize("FIRKIN_POD_AUTOSCALE_CONTAINERS", 64);
    assert!(
        containers > 1,
        "autoscale benchmark needs at least 2 containers"
    );
    let image = std::env::var("FIRKIN_POD_AUTOSCALE_IMAGE")
        .unwrap_or_else(|_| "python:3.12-alpine".to_owned());
    let pod_store_bytes = env_u64(
        "FIRKIN_POD_AUTOSCALE_POD_STORE_BYTES",
        7 * 1024 * 1024 * 1024,
    );
    let artifact = pod_autoscale_artifact_path();
    let temp = tempfile::tempdir().expect("autoscale runtime tempdir");
    let snapshot_dir = temp.path().join("snapshots");
    let pod_id = format!("pod-autoscale-{}", uuid::Uuid::new_v4().simple());
    let pod_store_path = temp
        .path()
        .join("runtime")
        .join(format!("pod-{pod_id}"))
        .join("pod-store.ext4");
    let driver = AppleVzLocalRuntimeDriver::with_snapshot_dir(
        &image,
        PortRegistry::default(),
        LogStore::default(),
        &snapshot_dir,
    );
    let metrics_driver = driver.clone();
    let mut backend = LocalRuntimeBackend::new(driver, "2026-05-06T12:00:00Z");

    let create_start = Instant::now();
    backend
        .create_pod(PodCreateRequest {
            pod_id: Some(pod_id.clone()),
            timeout: Some(600),
            metadata: BTreeMap::from([("benchmark".to_owned(), "pod-autoscale".to_owned())]),
            empty_dirs: vec![PodEmptyDir {
                name: "work".to_owned(),
            }],
            pod_store: PodStoreOptions {
                size_bytes: pod_store_bytes,
                ..PodStoreOptions::default()
            },
            containers: vec![autoscale_container(0)],
        })
        .await
        .expect("create autoscale pod");
    let create_pod_ms = create_start.elapsed().as_millis();

    let mut add_samples = Vec::with_capacity(containers.saturating_sub(1));
    for index in 1..containers {
        let start = Instant::now();
        backend
            .add_pod_container(&pod_id, autoscale_container(index))
            .await
            .unwrap_or_else(|error| panic!("add autoscale container ci-{index}: {error}"));
        add_samples.push(start.elapsed().as_millis());
    }
    let guest_usage_before_remove_bytes = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read pod-store usage before remove");
    let template_cache_entries = metrics_driver
        .pod_template_cache_entries(&pod_id)
        .await
        .expect("read pod template cache entries");
    let host_allocated_before_trim_bytes = allocated_bytes(&pod_store_path);

    let mut remove_samples = Vec::with_capacity(containers.saturating_sub(1));
    for index in (1..containers).rev() {
        let start = Instant::now();
        backend
            .delete_pod_container(&pod_id, &format!("ci-{index}"))
            .await
            .unwrap_or_else(|error| panic!("remove autoscale container ci-{index}: {error}"));
        remove_samples.push(start.elapsed().as_millis());
    }
    let guest_usage_after_remove_bytes = metrics_driver
        .pod_store_used_bytes(&pod_id)
        .await
        .expect("read pod-store usage after remove");
    let host_allocated_after_trim_bytes = allocated_bytes(&pod_store_path);
    backend
        .delete_pod(&pod_id)
        .await
        .expect("delete autoscale pod");

    let evidence = PodAutoscaleEvidence {
        containers,
        image,
        pod_store_bytes,
        shared_rootfs: true,
        template_cache_entries,
        image_format: "raw".to_owned(),
        create_pod_ms,
        add_container_ms: latency_stats(&add_samples).expect("add latency samples"),
        remove_container_ms: latency_stats(&remove_samples).expect("remove latency samples"),
        guest_usage_before_remove_bytes,
        guest_usage_after_remove_bytes,
        host_allocated_before_trim_bytes,
        host_allocated_after_trim_bytes,
        failures: Vec::new(),
    };
    validate_pod_autoscale_evidence(&evidence).expect("valid pod autoscale evidence");
    write_json_artifact(&artifact, &evidence);
    assert!(evidence.failures.is_empty());
}

fn autoscale_container(index: usize) -> PodContainerCreateRequest {
    PodContainerCreateRequest {
        name: format!("ci-{index}"),
        template_id: "base".to_owned(),
        command: vec![
            "/bin/sh".to_owned(),
            "-lc".to_owned(),
            format!("printf ready >/work/ready-ci-{index}; sleep 300"),
        ],
        env_vars: BTreeMap::new(),
        empty_dir_mounts: vec![PodVolumeMountRequest {
            name: "work".to_owned(),
            path: "/work".to_owned(),
            read_only: false,
        }],
        capture_output: false,
    }
}

fn pod_autoscale_artifact_path() -> PathBuf {
    std::env::var_os("FIRKIN_POD_AUTOSCALE_ARTIFACT").map_or_else(
        || PathBuf::from("target/firkin-live-evidence/pod-autoscale-evidence.json"),
        PathBuf::from,
    )
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn latency_stats(samples: &[u128]) -> Option<LatencyStats> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    Some(LatencyStats {
        p50: percentile(&sorted, 50),
        p95: percentile(&sorted, 95),
        max: *sorted.last().expect("nonempty samples have last"),
    })
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    let index = ((sorted.len() - 1) * percentile).div_ceil(100);
    sorted[index]
}

fn validate_pod_autoscale_evidence(evidence: &PodAutoscaleEvidence) -> Result<(), String> {
    if !evidence.failures.is_empty() {
        return Err(format!("pod autoscale failures: {:?}", evidence.failures));
    }
    if evidence.containers < 2 {
        return Err("pod autoscale evidence requires at least two containers".to_owned());
    }
    if evidence.image.is_empty() {
        return Err("pod autoscale evidence image is empty".to_owned());
    }
    if evidence.pod_store_bytes == 0 {
        return Err("pod autoscale evidence pod store size is zero".to_owned());
    }
    if !evidence.shared_rootfs {
        return Err("pod autoscale evidence did not use shared rootfs mode".to_owned());
    }
    if evidence.template_cache_entries != 1 {
        return Err(format!(
            "pod autoscale evidence expected one shared template cache entry, got {}",
            evidence.template_cache_entries
        ));
    }
    if evidence.image_format != "raw" {
        return Err(format!(
            "pod autoscale evidence expected raw image format, got {}",
            evidence.image_format
        ));
    }
    validate_latency("add_container_ms", evidence.add_container_ms)?;
    validate_latency("remove_container_ms", evidence.remove_container_ms)?;
    if evidence.create_pod_ms == 0 {
        return Err("pod autoscale evidence create_pod_ms is zero".to_owned());
    }
    if evidence.guest_usage_before_remove_bytes == 0 {
        return Err("pod autoscale evidence guest usage before remove is zero".to_owned());
    }
    if evidence.guest_usage_after_remove_bytes == 0 {
        return Err("pod autoscale evidence guest usage after remove is zero".to_owned());
    }
    if evidence.host_allocated_before_trim_bytes == 0 {
        return Err("pod autoscale evidence host allocation before trim is zero".to_owned());
    }
    if evidence.host_allocated_after_trim_bytes == 0 {
        return Err("pod autoscale evidence host allocation after trim is zero".to_owned());
    }
    Ok(())
}

fn validate_latency(name: &str, stats: LatencyStats) -> Result<(), String> {
    if stats.p50 > stats.p95 || stats.p95 > stats.max {
        return Err(format!(
            "{name} percentiles are not ordered: p50={} p95={} max={}",
            stats.p50, stats.p95, stats.max
        ));
    }
    Ok(())
}

fn allocated_bytes(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::metadata(path) else {
        return 0;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        metadata.len()
    }
}

fn write_json_artifact(path: &Path, evidence: &PodAutoscaleEvidence) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create pod autoscale evidence dir");
    }
    let json = serde_json::to_vec_pretty(evidence).expect("encode pod autoscale evidence");
    std::fs::write(path, json).expect("write pod autoscale evidence");
}

#[test]
fn latency_stats_reports_required_percentiles() {
    let stats = latency_stats(&[10, 20, 30, 40, 50]).unwrap();

    assert_eq!(stats.p50, 30);
    assert_eq!(stats.p95, 50);
    assert_eq!(stats.max, 50);
    assert!(latency_stats(&[]).is_none());
}

#[test]
fn pod_autoscale_evidence_validator_requires_shared_template() {
    let mut evidence = valid_pod_autoscale_evidence();
    evidence.template_cache_entries = 2;

    let error = validate_pod_autoscale_evidence(&evidence).unwrap_err();

    assert!(error.contains("one shared template cache entry"));
}

#[test]
fn pod_autoscale_evidence_validator_accepts_required_shape() {
    validate_pod_autoscale_evidence(&valid_pod_autoscale_evidence()).unwrap();
}

#[test]
#[ignore = "validates the signed live autoscale artifact after it is written"]
fn pod_autoscale_evidence_artifact_at_env_path_is_valid() {
    let artifact = pod_autoscale_artifact_path();
    let bytes = std::fs::read(&artifact).expect("read pod autoscale evidence artifact");
    let evidence: PodAutoscaleEvidence =
        serde_json::from_slice(&bytes).expect("decode pod autoscale evidence artifact");

    validate_pod_autoscale_evidence(&evidence).expect("valid pod autoscale evidence artifact");
}

fn valid_pod_autoscale_evidence() -> PodAutoscaleEvidence {
    PodAutoscaleEvidence {
        containers: 64,
        image: "python:3.12-alpine".to_owned(),
        pod_store_bytes: 7 * 1024 * 1024 * 1024,
        shared_rootfs: true,
        template_cache_entries: 1,
        image_format: "raw".to_owned(),
        create_pod_ms: 1,
        add_container_ms: LatencyStats {
            p50: 10,
            p95: 20,
            max: 30,
        },
        remove_container_ms: LatencyStats {
            p50: 10,
            p95: 20,
            max: 30,
        },
        guest_usage_before_remove_bytes: 1,
        guest_usage_after_remove_bytes: 1,
        host_allocated_before_trim_bytes: 1,
        host_allocated_after_trim_bytes: 1,
        failures: Vec::new(),
    }
}
