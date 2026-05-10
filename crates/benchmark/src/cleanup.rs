//! Cleanup benchmark sample helpers.
#![allow(missing_docs)]

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit, MAX_TAG_VALUE_BYTES};
use thiserror::Error as ThisError;

pub const CLEANUP_LEFTOVER_BYTES_METRIC: &str = "cleanup.leftover_bytes";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupScanEntry {
    name: String,
    leftover_bytes: u64,
}

impl CleanupScanEntry {
    #[must_use]
    pub fn new(name: impl Into<String>, leftover_bytes: u64) -> Self {
        Self {
            name: name.into(),
            leftover_bytes,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn leftover_bytes(&self) -> u64 {
        self.leftover_bytes
    }
}

#[derive(Debug, ThisError)]
pub enum CleanupSampleError {
    #[error("cleanup leftover byte total overflowed while adding `{name}`")]
    LeftoverBytesOverflow { name: String },
}

pub fn cleanup_leftover_bytes_sample(
    entries: impl IntoIterator<Item = CleanupScanEntry>,
) -> Result<BenchmarkSample, CleanupSampleError> {
    let entries = entries.into_iter().collect::<Vec<_>>();
    let leftover_bytes = cleanup_leftover_bytes_total(entries.iter())?;

    Ok(BenchmarkSample::from_static(
        CLEANUP_LEFTOVER_BYTES_METRIC,
        BenchmarkMetricKind::WorkloadResource,
        BenchmarkUnit::Bytes,
        leftover_bytes as f64,
    )
    .with_static_tag("source", "cleanup_scan")
    .with_dynamic_tag("entry_count", entries.len().to_string())
    .with_dynamic_tag("entries", cleanup_entry_names_tag(entries.iter())))
}

pub fn cleanup_leftover_bytes_total<'a>(
    entries: impl IntoIterator<Item = &'a CleanupScanEntry>,
) -> Result<u64, CleanupSampleError> {
    entries.into_iter().try_fold(0_u64, |total, entry| {
        total.checked_add(entry.leftover_bytes()).ok_or_else(|| {
            CleanupSampleError::LeftoverBytesOverflow {
                name: entry.name().to_owned(),
            }
        })
    })
}

fn cleanup_entry_names_tag<'a>(entries: impl IntoIterator<Item = &'a CleanupScanEntry>) -> String {
    let mut names = String::new();
    for entry in entries {
        let separator_len = usize::from(!names.is_empty());
        let available = MAX_TAG_VALUE_BYTES.saturating_sub(names.len() + separator_len);
        if available == 0 {
            break;
        }
        if separator_len == 1 {
            names.push(',');
        }
        let name = entry.name();
        if name.len() <= available {
            names.push_str(name);
        } else {
            for character in name.chars() {
                if names.len() + character.len_utf8() > MAX_TAG_VALUE_BYTES {
                    break;
                }
                names.push(character);
            }
            break;
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_named_cleanup_entries_into_leftover_bytes_sample() {
        let sample = cleanup_leftover_bytes_sample([
            CleanupScanEntry::new("runtime", 12),
            CleanupScanEntry::new("snapshots", 30),
            CleanupScanEntry::new("logs", 5),
        ])
        .expect("cleanup sample");

        assert_eq!(sample.metric(), CLEANUP_LEFTOVER_BYTES_METRIC);
        assert_eq!(sample.kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(sample.unit(), BenchmarkUnit::Bytes);
        assert_eq!(sample.value(), 47.0);
        assert_eq!(sample.tag_value("source"), Some("cleanup_scan"));
        assert_eq!(sample.tag_value("entry_count"), Some("3"));
        assert_eq!(sample.tag_value("entries"), Some("runtime,snapshots,logs"));
    }

    #[test]
    fn empty_cleanup_entries_emit_non_negative_zero_sample() {
        let sample =
            cleanup_leftover_bytes_sample([]).expect("empty cleanup scans should be measurable");

        assert_eq!(sample.value(), 0.0);
        assert_eq!(sample.tag_value("entry_count"), Some("0"));
        assert_eq!(sample.tag_value("entries"), Some(""));
    }

    #[test]
    fn rejects_overflow_instead_of_wrapping_byte_total() {
        let error = cleanup_leftover_bytes_sample([
            CleanupScanEntry::new("first", u64::MAX),
            CleanupScanEntry::new("second", 1),
        ])
        .expect_err("overflow");

        assert!(matches!(
            error,
            CleanupSampleError::LeftoverBytesOverflow { name } if name == "second"
        ));
    }

    #[test]
    fn entry_names_tag_is_bounded_to_trace_tag_value_limit() {
        let sample = cleanup_leftover_bytes_sample([
            CleanupScanEntry::new("x".repeat(MAX_TAG_VALUE_BYTES), 1),
            CleanupScanEntry::new("second", 2),
        ])
        .expect("cleanup sample");

        let entries = sample.tag_value("entries").expect("entries tag");
        assert_eq!(entries.len(), MAX_TAG_VALUE_BYTES);
        assert!(entries.chars().all(|value| value == 'x'));
    }
}
