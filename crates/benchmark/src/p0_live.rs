//! Exact P0 live-harness measurement helpers.
#![allow(missing_docs)]

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use thiserror::Error as ThisError;

pub const IO_FULL_AVG10_METRIC: &str = "sandbox.pressure.io_full_avg10";
pub const BOOT_FAILURE_RATE_METRIC: &str = "sandbox.reliability.boot_failure_rate";
pub const UNKNOWN_FAILURE_RATE_METRIC: &str = "reliability.unknown_failure_rate";
pub const CLEANUP_LEFTOVER_BYTES_METRIC: &str = "cleanup.leftover_bytes";

#[derive(Debug, ThisError)]
pub enum P0LiveMeasurementError {
    #[error("guest pressure file is missing the `{line}` line")]
    MissingPressureLine { line: &'static str },
    #[error("guest pressure `{line}` line is missing avg10")]
    MissingAvg10 { line: &'static str },
    #[error("guest pressure avg10 value `{value}` is invalid: {source}")]
    InvalidAvg10 {
        value: String,
        #[source]
        source: std::num::ParseFloatError,
    },
    #[error("reliability sample population is empty")]
    EmptyReliabilityPopulation,
    #[error("cleanup leftover scan failed while {operation} `{path}`: {source}")]
    CleanupIo {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxReliabilityOutcome {
    Ready,
    BootFailure,
    UnknownFailure,
}

#[must_use]
pub fn io_full_avg10_sample(avg10: f64) -> BenchmarkSample {
    BenchmarkSample::from_static(
        IO_FULL_AVG10_METRIC,
        BenchmarkMetricKind::WorkloadResource,
        BenchmarkUnit::Percent,
        avg10,
    )
}

pub fn io_full_avg10_sample_from_proc_pressure_io(
    contents: &str,
) -> Result<BenchmarkSample, P0LiveMeasurementError> {
    Ok(io_full_avg10_sample(parse_pressure_avg10(
        contents, "full",
    )?))
}

pub fn parse_pressure_avg10(
    contents: &str,
    line_name: &'static str,
) -> Result<f64, P0LiveMeasurementError> {
    let line = contents
        .lines()
        .find(|line| line.split_whitespace().next() == Some(line_name))
        .ok_or(P0LiveMeasurementError::MissingPressureLine { line: line_name })?;
    let raw = line
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))
        .ok_or(P0LiveMeasurementError::MissingAvg10 { line: line_name })?;
    raw.parse::<f64>()
        .map_err(|source| P0LiveMeasurementError::InvalidAvg10 {
            value: raw.to_owned(),
            source,
        })
}

pub fn reliability_rate_samples(
    outcomes: &[SandboxReliabilityOutcome],
) -> Result<[BenchmarkSample; 2], P0LiveMeasurementError> {
    if outcomes.is_empty() {
        return Err(P0LiveMeasurementError::EmptyReliabilityPopulation);
    }

    let total = outcomes.len() as f64;
    let boot_failures = outcomes
        .iter()
        .filter(|outcome| **outcome == SandboxReliabilityOutcome::BootFailure)
        .count() as f64;
    let unknown_failures = outcomes
        .iter()
        .filter(|outcome| **outcome == SandboxReliabilityOutcome::UnknownFailure)
        .count() as f64;

    Ok([
        reliability_rate_sample(BOOT_FAILURE_RATE_METRIC, boot_failures, total),
        reliability_rate_sample(UNKNOWN_FAILURE_RATE_METRIC, unknown_failures, total),
    ])
}

fn reliability_rate_sample(metric: &'static str, count: f64, total: f64) -> BenchmarkSample {
    BenchmarkSample::from_static(
        metric,
        BenchmarkMetricKind::WorkloadResource,
        BenchmarkUnit::Percent,
        (count / total) * 100.0,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupLeftoverRoots {
    roots: Vec<PathBuf>,
}

impl CleanupLeftoverRoots {
    #[must_use]
    pub fn new(roots: impl IntoIterator<Item = impl Into<PathBuf>>) -> Self {
        Self {
            roots: roots.into_iter().map(Into::into).collect(),
        }
    }

    pub fn measure_sample(&self) -> Result<BenchmarkSample, P0LiveMeasurementError> {
        let bytes = self.leftover_bytes()?;
        Ok(BenchmarkSample::from_static(
            CLEANUP_LEFTOVER_BYTES_METRIC,
            BenchmarkMetricKind::WorkloadResource,
            BenchmarkUnit::Bytes,
            bytes as f64,
        ))
    }

    pub fn leftover_bytes(&self) -> Result<u64, P0LiveMeasurementError> {
        self.roots
            .iter()
            .try_fold(0_u64, |total, root| Ok(total + path_logical_bytes(root)?))
    }
}

fn path_logical_bytes(path: &Path) -> Result<u64, P0LiveMeasurementError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(source) => {
            return Err(P0LiveMeasurementError::CleanupIo {
                operation: "stat cleanup path",
                path: path.to_owned(),
                source,
            });
        }
    };

    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    let mut total = 0_u64;
    for entry in fs::read_dir(path).map_err(|source| P0LiveMeasurementError::CleanupIo {
        operation: "read cleanup directory",
        path: path.to_owned(),
        source,
    })? {
        let entry = entry.map_err(|source| P0LiveMeasurementError::CleanupIo {
            operation: "read cleanup directory entry",
            path: path.to_owned(),
            source,
        })?;
        total += path_logical_bytes(&entry.path())?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_io_full_avg10_from_linux_pressure_file() {
        let sample = io_full_avg10_sample_from_proc_pressure_io(
            "some avg10=0.00 avg60=0.05 avg300=0.10 total=12\n\
             full avg10=1.25 avg60=0.25 avg300=0.05 total=7\n",
        )
        .expect("pressure sample");

        assert_eq!(sample.metric(), IO_FULL_AVG10_METRIC);
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Percent);
        assert_eq!(sample.value(), 1.25);
    }

    #[test]
    fn rejects_pressure_without_full_line_or_avg10() {
        assert!(matches!(
            io_full_avg10_sample_from_proc_pressure_io("some avg10=0.00 total=1"),
            Err(P0LiveMeasurementError::MissingPressureLine { line: "full" })
        ));
        assert!(matches!(
            io_full_avg10_sample_from_proc_pressure_io("full avg60=0.00 total=1"),
            Err(P0LiveMeasurementError::MissingAvg10 { line: "full" })
        ));
    }

    #[test]
    fn reliability_rates_require_live_population_and_classify_failures() {
        assert!(matches!(
            reliability_rate_samples(&[]),
            Err(P0LiveMeasurementError::EmptyReliabilityPopulation)
        ));

        let samples = reliability_rate_samples(&[
            SandboxReliabilityOutcome::Ready,
            SandboxReliabilityOutcome::BootFailure,
            SandboxReliabilityOutcome::UnknownFailure,
            SandboxReliabilityOutcome::Ready,
        ])
        .expect("reliability samples");

        assert_eq!(samples[0].metric(), BOOT_FAILURE_RATE_METRIC);
        assert_eq!(samples[0].unit(), BenchmarkUnit::Percent);
        assert_eq!(samples[0].value(), 25.0);
        assert_eq!(samples[1].metric(), UNKNOWN_FAILURE_RATE_METRIC);
        assert_eq!(samples[1].value(), 25.0);
    }

    #[test]
    fn cleanup_leftover_roots_sum_logical_file_bytes_and_ignore_missing_roots() {
        let temp = tempfile::tempdir().expect("tempdir");
        let active = temp.path().join("active-vms");
        let logs = temp.path().join("logs");
        fs::create_dir_all(active.join("vm-1")).expect("active dir");
        fs::create_dir_all(&logs).expect("logs dir");
        fs::write(active.join("vm-1").join("heartbeat"), b"12345").expect("heartbeat");
        fs::write(logs.join("runtime.log"), b"abcdef").expect("log");

        let sample = CleanupLeftoverRoots::new([active, logs, temp.path().join("missing")])
            .measure_sample()
            .expect("cleanup sample");

        assert_eq!(sample.metric(), CLEANUP_LEFTOVER_BYTES_METRIC);
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Bytes);
        assert_eq!(sample.value(), 11.0);
    }
}
