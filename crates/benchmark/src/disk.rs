//! Disk benchmark harness payloads and source-output parsing.
#![allow(missing_docs)]

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use serde::Deserialize;
use thiserror::Error as ThisError;

pub const METADATA_CREATE_STAT_UNLINK_METRIC: &str = "sandbox.disk.metadata_create_stat_unlink_ms";
pub const FSYNC_P99_METRIC: &str = "sandbox.disk.fsync_p99_us";
pub const SPARSE_BLOAT_AFTER_DELETE_METRIC: &str = "disk.sparse_bloat_after_delete";
pub const SPARSE_BLOAT_AFTER_TRIM_METRIC: &str = "disk.sparse_bloat_after_trim";
pub const HOST_BYTES_RECLAIMED_AFTER_TRIM_METRIC: &str = "disk.host_bytes_reclaimed_after_trim";
pub const TRIM_RECLAIM_BYTES_PER_SEC_METRIC: &str = "sandbox.disk.trim_reclaim_bytes_per_sec";

#[derive(Debug, ThisError)]
pub enum DiskBenchmarkHarnessError {
    #[error("disk benchmark output was not valid json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("disk benchmark output did not contain finite positive {field}")]
    InvalidPositiveNumber { field: &'static str },
    #[error("disk benchmark output did not contain finite non-negative {field}")]
    InvalidNonNegativeNumber { field: &'static str },
    #[error("disk benchmark output had zero guest-used bytes for sparse ratio")]
    ZeroGuestUsedBytes,
    #[error("disk benchmark output had zero trim duration")]
    ZeroTrimDuration,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct GuestDiskBenchmarkOutput {
    pub metadata_create_stat_unlink_ms: f64,
    pub fsync_p99_us: f64,
    pub trim_reclaimed_bytes: u64,
    pub trim_duration_ms: f64,
}

impl GuestDiskBenchmarkOutput {
    pub fn parse_json(bytes: impl AsRef<[u8]>) -> Result<Self, DiskBenchmarkHarnessError> {
        let output: Self = serde_json::from_slice(bytes.as_ref())?;
        output.validate()?;
        Ok(output)
    }

    pub fn into_samples(self) -> Vec<BenchmarkSample> {
        vec![
            BenchmarkSample::new(
                METADATA_CREATE_STAT_UNLINK_METRIC,
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                self.metadata_create_stat_unlink_ms,
            )
            .with_static_tag("source", "guest_disk_harness"),
            BenchmarkSample::new(
                FSYNC_P99_METRIC,
                BenchmarkMetricKind::WorkloadResource,
                BenchmarkUnit::Microseconds,
                self.fsync_p99_us,
            )
            .with_static_tag("source", "guest_disk_harness"),
            BenchmarkSample::new(
                TRIM_RECLAIM_BYTES_PER_SEC_METRIC,
                BenchmarkMetricKind::WorkloadResource,
                BenchmarkUnit::BytesPerSecond,
                self.trim_reclaim_bytes_per_sec(),
            )
            .with_static_tag("source", "guest_disk_harness"),
        ]
    }

    pub fn trim_reclaim_bytes_per_sec(self) -> f64 {
        self.trim_reclaimed_bytes as f64 / (self.trim_duration_ms / 1000.0)
    }

    fn validate(self) -> Result<(), DiskBenchmarkHarnessError> {
        require_positive(
            self.metadata_create_stat_unlink_ms,
            "metadata_create_stat_unlink_ms",
        )?;
        require_positive(self.fsync_p99_us, "fsync_p99_us")?;
        require_non_negative(self.trim_reclaimed_bytes as f64, "trim_reclaimed_bytes")?;
        if self.trim_duration_ms == 0.0 {
            return Err(DiskBenchmarkHarnessError::ZeroTrimDuration);
        }
        require_positive(self.trim_duration_ms, "trim_duration_ms")?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct GuestDiskCoreBenchmarkOutput {
    pub metadata_create_stat_unlink_ms: f64,
    pub fsync_p99_us: f64,
}

impl GuestDiskCoreBenchmarkOutput {
    pub fn parse_json(bytes: impl AsRef<[u8]>) -> Result<Self, DiskBenchmarkHarnessError> {
        let output: Self = serde_json::from_slice(bytes.as_ref())?;
        output.validate()?;
        Ok(output)
    }

    pub fn into_samples(self) -> Vec<BenchmarkSample> {
        vec![
            BenchmarkSample::new(
                METADATA_CREATE_STAT_UNLINK_METRIC,
                BenchmarkMetricKind::LifecycleLatency,
                BenchmarkUnit::Milliseconds,
                self.metadata_create_stat_unlink_ms,
            )
            .with_static_tag("source", "guest_disk_core_harness"),
            BenchmarkSample::new(
                FSYNC_P99_METRIC,
                BenchmarkMetricKind::WorkloadResource,
                BenchmarkUnit::Microseconds,
                self.fsync_p99_us,
            )
            .with_static_tag("source", "guest_disk_core_harness"),
        ]
    }

    fn validate(self) -> Result<(), DiskBenchmarkHarnessError> {
        require_positive(
            self.metadata_create_stat_unlink_ms,
            "metadata_create_stat_unlink_ms",
        )?;
        require_positive(self.fsync_p99_us, "fsync_p99_us")?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct HostGuestDiskUsageOutput {
    pub host_allocated_bytes: u64,
    pub guest_used_bytes: u64,
}

impl HostGuestDiskUsageOutput {
    pub fn parse_json(bytes: impl AsRef<[u8]>) -> Result<Self, DiskBenchmarkHarnessError> {
        let output: Self = serde_json::from_slice(bytes.as_ref())?;
        output.validate()?;
        Ok(output)
    }

    pub fn sparse_bloat_ratio(self) -> f64 {
        self.host_allocated_bytes as f64 / self.guest_used_bytes as f64
    }

    pub fn into_sample(self) -> BenchmarkSample {
        self.into_sample_with_metric(SPARSE_BLOAT_AFTER_TRIM_METRIC)
    }

    pub fn into_sample_with_metric(self, metric: &'static str) -> BenchmarkSample {
        BenchmarkSample::new(
            metric,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Ratio,
            self.sparse_bloat_ratio(),
        )
        .with_static_tag("source", "host_guest_disk_usage")
    }

    fn validate(self) -> Result<(), DiskBenchmarkHarnessError> {
        require_non_negative(self.host_allocated_bytes as f64, "host_allocated_bytes")?;
        if self.guest_used_bytes == 0 {
            return Err(DiskBenchmarkHarnessError::ZeroGuestUsedBytes);
        }
        Ok(())
    }
}

pub fn guest_disk_benchmark_script(
    workdir: &str,
    metadata_files: u32,
    fsync_writes: u32,
) -> String {
    format!(
        r#"set -eu
workdir={workdir:?}
metadata_files={metadata_files}
fsync_writes={fsync_writes}
rm -rf "$workdir"
mkdir -p "$workdir"
python3 - "$workdir" "$metadata_files" "$fsync_writes" <<'PY'
import json, os, subprocess, sys, time

workdir = sys.argv[1]
metadata_files = int(sys.argv[2])
fsync_writes = int(sys.argv[3])
metadata_dir = os.path.join(workdir, "metadata")
fsync_path = os.path.join(workdir, "fsync.bin")
trim_dir = os.path.join(workdir, "trim")
os.makedirs(metadata_dir, exist_ok=True)
os.makedirs(trim_dir, exist_ok=True)

start = time.perf_counter()
for index in range(metadata_files):
    path = os.path.join(metadata_dir, f"file-{{index}}")
    with open(path, "wb") as handle:
        handle.write(b"x")
    os.stat(path)
    os.unlink(path)
metadata_ms = (time.perf_counter() - start) * 1000.0

latencies_us = []
with open(fsync_path, "wb", buffering=0) as handle:
    for index in range(fsync_writes):
        handle.write((str(index).zfill(4096)).encode("ascii")[:4096])
        start = time.perf_counter()
        os.fsync(handle.fileno())
        latencies_us.append((time.perf_counter() - start) * 1000000.0)
p99 = sorted(latencies_us)[max(0, int(len(latencies_us) * 0.99) - 1)]

trim_path = os.path.join(trim_dir, "reclaim.bin")
with open(trim_path, "wb", buffering=0) as handle:
    block = b"0" * 1048576
    for _ in range(64):
        handle.write(block)
    os.fsync(handle.fileno())
os.unlink(trim_path)
before = time.perf_counter()
trim = subprocess.run(["fstrim", "-v", workdir], check=True, text=True, capture_output=True)
trim_ms = (time.perf_counter() - before) * 1000.0
reclaimed = 0
for token in trim.stdout.replace(":", " ").split():
    if token.isdigit():
        reclaimed = int(token)
        break

print(json.dumps({{
    "metadata_create_stat_unlink_ms": metadata_ms,
    "fsync_p99_us": p99,
    "trim_reclaimed_bytes": reclaimed,
    "trim_duration_ms": trim_ms,
}}))
PY
"#,
    )
}

pub fn guest_disk_core_benchmark_script(
    workdir: &str,
    metadata_files: u32,
    fsync_writes: u32,
) -> String {
    format!(
        r#"set -eu
workdir={workdir:?}
metadata_files={metadata_files}
fsync_writes={fsync_writes}
rm -rf "$workdir"
mkdir -p "$workdir"
python3 - "$workdir" "$metadata_files" "$fsync_writes" <<'PY'
import json, os, sys, time

workdir = sys.argv[1]
metadata_files = int(sys.argv[2])
fsync_writes = int(sys.argv[3])
metadata_dir = os.path.join(workdir, "metadata")
fsync_path = os.path.join(workdir, "fsync.bin")
os.makedirs(metadata_dir, exist_ok=True)

start = time.perf_counter()
for index in range(metadata_files):
    path = os.path.join(metadata_dir, f"file-{{index}}")
    with open(path, "wb") as handle:
        handle.write(b"x")
    os.stat(path)
    os.unlink(path)
metadata_ms = (time.perf_counter() - start) * 1000.0

latencies_us = []
with open(fsync_path, "wb", buffering=0) as handle:
    for index in range(fsync_writes):
        handle.write((str(index).zfill(4096)).encode("ascii")[:4096])
        start = time.perf_counter()
        os.fsync(handle.fileno())
        latencies_us.append((time.perf_counter() - start) * 1000000.0)
p99 = sorted(latencies_us)[max(0, int(len(latencies_us) * 0.99) - 1)]

print(json.dumps({{
    "metadata_create_stat_unlink_ms": metadata_ms,
    "fsync_p99_us": p99,
}}))
PY
"#,
    )
}

pub fn host_guest_disk_usage_json(host_allocated_bytes: u64, guest_used_bytes: u64) -> String {
    format!(
        r#"{{"host_allocated_bytes":{host_allocated_bytes},"guest_used_bytes":{guest_used_bytes}}}"#
    )
}

fn require_positive(value: f64, field: &'static str) -> Result<(), DiskBenchmarkHarnessError> {
    if value.is_finite() && value > 0.0 {
        return Ok(());
    }
    Err(DiskBenchmarkHarnessError::InvalidPositiveNumber { field })
}

fn require_non_negative(value: f64, field: &'static str) -> Result<(), DiskBenchmarkHarnessError> {
    if value.is_finite() && value >= 0.0 {
        return Ok(());
    }
    Err(DiskBenchmarkHarnessError::InvalidNonNegativeNumber { field })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_disk_output_parses_and_emits_p0_samples() {
        let output = GuestDiskBenchmarkOutput::parse_json(
            br#"{
                "metadata_create_stat_unlink_ms": 12.5,
                "fsync_p99_us": 222.0,
                "trim_reclaimed_bytes": 4096,
                "trim_duration_ms": 2.0
            }"#,
        )
        .expect("valid guest output");

        let samples = output.into_samples();

        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].metric(), METADATA_CREATE_STAT_UNLINK_METRIC);
        assert_eq!(samples[0].unit(), BenchmarkUnit::Milliseconds);
        assert_eq!(samples[1].metric(), FSYNC_P99_METRIC);
        assert_eq!(samples[1].unit(), BenchmarkUnit::Microseconds);
        assert_eq!(samples[2].metric(), TRIM_RECLAIM_BYTES_PER_SEC_METRIC);
        assert_eq!(samples[2].unit(), BenchmarkUnit::BytesPerSecond);
        assert_eq!(samples[2].value(), 2_048_000.0);
        assert_eq!(samples[2].tag_value("source"), Some("guest_disk_harness"));
    }

    #[test]
    fn guest_disk_output_rejects_zero_trim_duration() {
        let error = GuestDiskBenchmarkOutput::parse_json(
            br#"{
                "metadata_create_stat_unlink_ms": 1.0,
                "fsync_p99_us": 2.0,
                "trim_reclaimed_bytes": 1,
                "trim_duration_ms": 0.0
            }"#,
        )
        .expect_err("zero trim duration rejects");

        assert!(matches!(error, DiskBenchmarkHarnessError::ZeroTrimDuration));
    }

    #[test]
    fn host_guest_usage_parses_and_emits_sparse_bloat_ratio() {
        let output = HostGuestDiskUsageOutput::parse_json(
            host_guest_disk_usage_json(12_000, 3_000).as_bytes(),
        )
        .expect("valid usage");
        let sample = output.into_sample();

        assert_eq!(sample.metric(), SPARSE_BLOAT_AFTER_TRIM_METRIC);
        assert_eq!(sample.unit(), BenchmarkUnit::Ratio);
        assert_eq!(sample.value(), 4.0);
        assert_eq!(sample.tag_value("source"), Some("host_guest_disk_usage"));
    }

    #[test]
    fn host_guest_usage_can_emit_named_disk_bloat_stage() {
        let output = HostGuestDiskUsageOutput::parse_json(
            host_guest_disk_usage_json(12_000, 3_000).as_bytes(),
        )
        .expect("valid usage");
        let sample = output.into_sample_with_metric(SPARSE_BLOAT_AFTER_DELETE_METRIC);

        assert_eq!(sample.metric(), SPARSE_BLOAT_AFTER_DELETE_METRIC);
        assert_eq!(sample.unit(), BenchmarkUnit::Ratio);
        assert_eq!(sample.value(), 4.0);
    }

    #[test]
    fn host_guest_usage_rejects_zero_guest_used_bytes() {
        let error = HostGuestDiskUsageOutput::parse_json(
            br#"{"host_allocated_bytes":12,"guest_used_bytes":0}"#,
        )
        .expect_err("zero guest-used rejects");

        assert!(matches!(
            error,
            DiskBenchmarkHarnessError::ZeroGuestUsedBytes
        ));
    }

    #[test]
    fn guest_disk_core_output_parses_and_emits_p0_samples() {
        let output = GuestDiskCoreBenchmarkOutput::parse_json(
            br#"{
                "metadata_create_stat_unlink_ms": 8.0,
                "fsync_p99_us": 111.0
            }"#,
        )
        .expect("valid core output");

        let samples = output.into_samples();

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].metric(), METADATA_CREATE_STAT_UNLINK_METRIC);
        assert_eq!(samples[0].unit(), BenchmarkUnit::Milliseconds);
        assert_eq!(
            samples[0].tag_value("source"),
            Some("guest_disk_core_harness")
        );
        assert_eq!(samples[1].metric(), FSYNC_P99_METRIC);
        assert_eq!(samples[1].unit(), BenchmarkUnit::Microseconds);
        assert_eq!(
            samples[1].tag_value("source"),
            Some("guest_disk_core_harness")
        );
    }

    #[test]
    fn guest_disk_core_script_reports_core_fields_without_trim() {
        let script = guest_disk_core_benchmark_script("/tmp/firkin-disk", 16, 8);

        assert!(script.contains("metadata_create_stat_unlink_ms"));
        assert!(script.contains("fsync_p99_us"));
        assert!(!script.contains("fstrim"));
    }
}
