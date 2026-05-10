//! Signed-live sandbox reliability benchmark helpers.
#![allow(missing_docs)]

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use thiserror::Error as ThisError;

pub const BOOT_FAILURE_RATE_METRIC: &str = "sandbox.reliability.boot_failure_rate";
pub const UNKNOWN_FAILURE_RATE_METRIC: &str = "reliability.unknown_failure_rate";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedLiveReliabilityAttemptCounts {
    pub ready: u64,
    pub boot_failures: u64,
    pub unknown_failures: u64,
}

impl SignedLiveReliabilityAttemptCounts {
    #[must_use]
    pub const fn new(ready: u64, boot_failures: u64, unknown_failures: u64) -> Self {
        Self {
            ready,
            boot_failures,
            unknown_failures,
        }
    }

    pub fn validate(self) -> Result<ValidatedSignedLiveReliabilityAttemptCounts, ReliabilityError> {
        let total_attempts = self
            .ready
            .checked_add(self.boot_failures)
            .and_then(|total| total.checked_add(self.unknown_failures))
            .ok_or(ReliabilityError::AttemptCountOverflow)?;

        if total_attempts == 0 {
            return Err(ReliabilityError::EmptyAttemptPopulation);
        }

        Ok(ValidatedSignedLiveReliabilityAttemptCounts {
            counts: self,
            total_attempts,
        })
    }

    pub fn into_samples(self) -> Result<[BenchmarkSample; 2], ReliabilityError> {
        self.validate()
            .map(ValidatedSignedLiveReliabilityAttemptCounts::into_samples)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedSignedLiveReliabilityAttemptCounts {
    counts: SignedLiveReliabilityAttemptCounts,
    total_attempts: u64,
}

impl ValidatedSignedLiveReliabilityAttemptCounts {
    #[must_use]
    pub const fn counts(self) -> SignedLiveReliabilityAttemptCounts {
        self.counts
    }

    #[must_use]
    pub const fn total_attempts(self) -> u64 {
        self.total_attempts
    }

    #[must_use]
    pub fn boot_failure_rate_percent(self) -> f64 {
        percent(self.counts.boot_failures, self.total_attempts)
    }

    #[must_use]
    pub fn unknown_failure_rate_percent(self) -> f64 {
        percent(self.counts.unknown_failures, self.total_attempts)
    }

    #[must_use]
    pub fn into_samples(self) -> [BenchmarkSample; 2] {
        [
            reliability_sample(BOOT_FAILURE_RATE_METRIC, self.boot_failure_rate_percent()),
            reliability_sample(
                UNKNOWN_FAILURE_RATE_METRIC,
                self.unknown_failure_rate_percent(),
            ),
        ]
    }
}

impl TryFrom<SignedLiveReliabilityAttemptCounts> for ValidatedSignedLiveReliabilityAttemptCounts {
    type Error = ReliabilityError;

    fn try_from(counts: SignedLiveReliabilityAttemptCounts) -> Result<Self, Self::Error> {
        counts.validate()
    }
}

#[derive(Debug, ThisError)]
pub enum ReliabilityError {
    #[error("signed-live reliability attempt population is empty")]
    EmptyAttemptPopulation,
    #[error("signed-live reliability attempt counts overflow u64")]
    AttemptCountOverflow,
}

fn percent(count: u64, total: u64) -> f64 {
    (count as f64 / total as f64) * 100.0
}

fn reliability_sample(metric: &'static str, value: f64) -> BenchmarkSample {
    BenchmarkSample::from_static(
        metric,
        BenchmarkMetricKind::WorkloadResource,
        BenchmarkUnit::Percent,
        value,
    )
    .with_static_tag("source", "signed-live-reliability")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_signed_live_failure_rates_from_classified_counts() {
        let counts = SignedLiveReliabilityAttemptCounts::new(17, 2, 1);
        let validated = counts.validate().expect("valid counts");

        assert_eq!(validated.total_attempts(), 20);
        assert_eq!(validated.boot_failure_rate_percent(), 10.0);
        assert_eq!(validated.unknown_failure_rate_percent(), 5.0);
    }

    #[test]
    fn converts_validated_counts_to_workload_percent_samples() {
        let samples = SignedLiveReliabilityAttemptCounts::new(8, 1, 1)
            .into_samples()
            .expect("samples");

        assert_eq!(samples[0].metric(), BOOT_FAILURE_RATE_METRIC);
        assert_eq!(samples[0].kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(samples[0].unit(), BenchmarkUnit::Percent);
        assert_eq!(samples[0].value(), 10.0);
        assert_eq!(
            samples[0].tag_value("source"),
            Some("signed-live-reliability")
        );

        assert_eq!(samples[1].metric(), UNKNOWN_FAILURE_RATE_METRIC);
        assert_eq!(samples[1].kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(samples[1].unit(), BenchmarkUnit::Percent);
        assert_eq!(samples[1].value(), 10.0);
        assert_eq!(
            samples[1].tag_value("source"),
            Some("signed-live-reliability")
        );
    }

    #[test]
    fn rejects_empty_attempt_population() {
        let error = SignedLiveReliabilityAttemptCounts::new(0, 0, 0)
            .validate()
            .expect_err("empty population rejects");

        assert!(matches!(error, ReliabilityError::EmptyAttemptPopulation));
    }

    #[test]
    fn rejects_overflowing_attempt_population() {
        let error = SignedLiveReliabilityAttemptCounts::new(u64::MAX, 1, 0)
            .validate()
            .expect_err("overflow rejects");

        assert!(matches!(error, ReliabilityError::AttemptCountOverflow));
    }
}
