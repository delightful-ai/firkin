//! Guest PSI benchmark helpers.
#![allow(missing_docs)]

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use serde::Deserialize;
use std::path::Path;
use thiserror::Error as ThisError;

pub const IO_FULL_AVG10_METRIC: &str = "sandbox.pressure.io_full_avg10";
pub const IO_SOME_AVG10_METRIC: &str = "sandbox.pressure.io_some_avg10";
pub const GUEST_PSI_PREREQUISITE: &str = "rebuilt signed live kernel with CONFIG_PSI=y, CONFIG_PSI_DEFAULT_DISABLED unset, and procfs mounted at /proc";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuestIoPressure {
    io_full_avg10: f64,
    io_some_avg10: Option<f64>,
}

impl GuestIoPressure {
    pub fn parse_proc_pressure_io(contents: &str) -> Result<Self, GuestPressureError> {
        Ok(Self {
            io_full_avg10: parse_pressure_avg10(contents, "full")?,
            io_some_avg10: parse_pressure_avg10(contents, "some").ok(),
        })
    }

    pub fn from_emitted_json(bytes: impl AsRef<[u8]>) -> Result<Self, GuestPressureError> {
        let emitted: EmittedGuestIoPressure = serde_json::from_slice(bytes.as_ref())?;
        if !emitted.signed_live {
            return Err(GuestPressureError::UnsignedLiveEmission);
        }
        if emitted.source != "/proc/pressure/io" {
            return Err(GuestPressureError::UnexpectedSource {
                actual_source: emitted.source,
            });
        }
        Ok(Self {
            io_full_avg10: emitted.io_full_avg10,
            io_some_avg10: emitted.io_some_avg10,
        })
    }

    #[must_use]
    pub const fn io_full_avg10(self) -> f64 {
        self.io_full_avg10
    }

    #[must_use]
    pub const fn io_some_avg10(self) -> Option<f64> {
        self.io_some_avg10
    }

    #[must_use]
    pub fn into_samples(self) -> Vec<BenchmarkSample> {
        let mut samples = vec![
            BenchmarkSample::from_static(
                IO_FULL_AVG10_METRIC,
                BenchmarkMetricKind::WorkloadResource,
                BenchmarkUnit::Percent,
                self.io_full_avg10,
            )
            .with_static_tag("source", "guest-proc-pressure-io")
            .with_static_tag("signed_live", "true"),
        ];

        if let Some(avg10) = self.io_some_avg10 {
            samples.push(
                BenchmarkSample::from_static(
                    IO_SOME_AVG10_METRIC,
                    BenchmarkMetricKind::WorkloadResource,
                    BenchmarkUnit::Percent,
                    avg10,
                )
                .with_static_tag("source", "guest-proc-pressure-io")
                .with_static_tag("signed_live", "true"),
            );
        }

        samples
    }
}

#[derive(Debug, ThisError)]
pub enum GuestPressureError {
    #[error("guest pressure file is missing the `{line}` line")]
    MissingPressureLine { line: &'static str },
    #[error("guest pressure `{line}` line is missing avg10")]
    MissingAvg10 { line: &'static str },
    #[error("guest pressure `{line}` avg10 value `{value}` is invalid: {source}")]
    InvalidAvg10 {
        line: &'static str,
        value: String,
        #[source]
        source: std::num::ParseFloatError,
    },
    #[error("guest pressure JSON is not signed as live")]
    UnsignedLiveEmission,
    #[error("guest pressure JSON source is `{actual_source}`, expected /proc/pressure/io")]
    UnexpectedSource { actual_source: String },
    #[error("guest pressure JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[must_use]
pub const fn signed_live_guest_io_pressure_script() -> &'static str {
    r#"set -eu
python3 - <<'PY'
import json

source = "/proc/pressure/io"
values = {"signed_live": True, "source": source}
with open(source, "r", encoding="utf-8") as pressure:
    for line in pressure:
        fields = line.split()
        if not fields:
            continue
        name = fields[0]
        avg10 = next((field[6:] for field in fields[1:] if field.startswith("avg10=")), None)
        if avg10 is None:
            continue
        if name == "full":
            values["io_full_avg10"] = float(avg10)
        elif name == "some":
            values["io_some_avg10"] = float(avg10)

if "io_full_avg10" not in values:
    raise SystemExit("missing full avg10 in /proc/pressure/io")

print(json.dumps(values, sort_keys=True, separators=(",", ":")))
PY
"#
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestPsiReadiness {
    kernel_config_psi: bool,
    kernel_config_default_enabled: bool,
    kernel_artifact_current: Option<bool>,
}

impl GuestPsiReadiness {
    #[must_use]
    pub fn from_kernel_config(config: &str) -> Self {
        let kernel_config_psi = config.lines().any(|line| line == "CONFIG_PSI=y");
        let kernel_config_default_enabled = config
            .lines()
            .any(|line| line == "# CONFIG_PSI_DEFAULT_DISABLED is not set");
        Self {
            kernel_config_psi,
            kernel_config_default_enabled,
            kernel_artifact_current: None,
        }
    }

    #[must_use]
    pub fn from_kernel_config_and_artifact(
        config: &str,
        config_path: &Path,
        artifact_path: &Path,
    ) -> Self {
        Self {
            kernel_artifact_current: Some(kernel_artifact_is_current(config_path, artifact_path)),
            ..Self::from_kernel_config(config)
        }
    }

    #[must_use]
    pub const fn kernel_config_psi(&self) -> bool {
        self.kernel_config_psi
    }

    #[must_use]
    pub const fn kernel_config_default_enabled(&self) -> bool {
        self.kernel_config_default_enabled
    }

    #[must_use]
    pub const fn kernel_artifact_current(&self) -> Option<bool> {
        self.kernel_artifact_current
    }

    #[must_use]
    pub fn source_config_ready(&self) -> bool {
        self.kernel_config_psi && self.kernel_config_default_enabled
    }

    #[must_use]
    pub fn signed_live_prerequisite_ready(&self) -> bool {
        self.source_config_ready() && self.kernel_artifact_current == Some(true)
    }

    #[must_use]
    pub fn missing_prerequisite(&self) -> Option<&'static str> {
        if !self.kernel_config_psi {
            Some("kernel/config-arm64 must set CONFIG_PSI=y")
        } else if !self.kernel_config_default_enabled {
            Some("kernel/config-arm64 must leave CONFIG_PSI_DEFAULT_DISABLED unset")
        } else if self.kernel_artifact_current == Some(false) {
            Some("rebuild bin/vmlinux from kernel/config-arm64 and sign the live harness")
        } else if self.kernel_artifact_current.is_none() {
            Some("verify the signed-live kernel artifact was rebuilt from kernel/config-arm64")
        } else {
            None
        }
    }
}

fn kernel_artifact_is_current(config_path: &Path, artifact_path: &Path) -> bool {
    let Ok(config_modified) = config_path.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };
    let Ok(artifact_modified) = artifact_path.metadata().and_then(|meta| meta.modified()) else {
        return false;
    };
    artifact_modified >= config_modified
}

fn parse_pressure_avg10(
    contents: &str,
    line_name: &'static str,
) -> Result<f64, GuestPressureError> {
    let line = contents
        .lines()
        .find(|line| line.split_whitespace().next() == Some(line_name))
        .ok_or(GuestPressureError::MissingPressureLine { line: line_name })?;
    let raw = line
        .split_whitespace()
        .find_map(|field| field.strip_prefix("avg10="))
        .ok_or(GuestPressureError::MissingAvg10 { line: line_name })?;
    raw.parse::<f64>()
        .map_err(|source| GuestPressureError::InvalidAvg10 {
            line: line_name,
            value: raw.to_owned(),
            source,
        })
}

#[derive(Debug, Deserialize)]
struct EmittedGuestIoPressure {
    signed_live: bool,
    source: String,
    io_full_avg10: f64,
    #[serde(default)]
    io_some_avg10: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use firkin_trace::{BenchmarkMetricKind, BenchmarkUnit};

    #[test]
    fn parses_io_pressure_avg10_values() {
        let pressure = GuestIoPressure::parse_proc_pressure_io(
            "some avg10=0.25 avg60=0.05 avg300=0.01 total=99\n\
             full avg10=1.50 avg60=0.25 avg300=0.05 total=7\n",
        )
        .expect("pressure");

        assert_eq!(pressure.io_some_avg10(), Some(0.25));
        assert_eq!(pressure.io_full_avg10(), 1.50);
    }

    #[test]
    fn rejects_missing_or_invalid_io_full_avg10() {
        assert!(matches!(
            GuestIoPressure::parse_proc_pressure_io("some avg10=0.25 total=99\n"),
            Err(GuestPressureError::MissingPressureLine { line: "full" })
        ));
        assert!(matches!(
            GuestIoPressure::parse_proc_pressure_io("full avg10=nope total=7\n"),
            Err(GuestPressureError::InvalidAvg10 { line: "full", .. })
        ));
    }

    #[test]
    fn emits_signed_live_guest_json_from_script_output() {
        let script = signed_live_guest_io_pressure_script();

        assert!(script.contains("/proc/pressure/io"));
        assert!(script.contains("\"signed_live\""));
        assert!(script.contains("\"io_full_avg10\""));
        assert!(script.contains("\"io_some_avg10\""));
    }

    #[test]
    fn bundled_arm64_kernel_config_exposes_proc_pressure_io() {
        let config = include_str!("../../../kernel/config-arm64");

        assert!(config.lines().any(|line| line == "CONFIG_PSI=y"));
        assert!(
            config
                .lines()
                .any(|line| line == "# CONFIG_PSI_DEFAULT_DISABLED is not set")
        );
        assert!(!config.lines().any(|line| line == "# CONFIG_PSI is not set"));
    }

    #[test]
    fn guest_psi_readiness_names_missing_prerequisites() {
        let missing_psi =
            GuestPsiReadiness::from_kernel_config("# CONFIG_PSI is not set\nCONFIG_PROC_FS=y\n");
        assert!(!missing_psi.source_config_ready());
        assert_eq!(
            missing_psi.missing_prerequisite(),
            Some("kernel/config-arm64 must set CONFIG_PSI=y")
        );

        let disabled_by_default = GuestPsiReadiness::from_kernel_config(
            "CONFIG_PSI=y\nCONFIG_PSI_DEFAULT_DISABLED=y\nCONFIG_PROC_FS=y\n",
        );
        assert!(!disabled_by_default.source_config_ready());
        assert_eq!(
            disabled_by_default.missing_prerequisite(),
            Some("kernel/config-arm64 must leave CONFIG_PSI_DEFAULT_DISABLED unset")
        );

        let ready_source = GuestPsiReadiness::from_kernel_config(
            "CONFIG_PSI=y\n# CONFIG_PSI_DEFAULT_DISABLED is not set\n",
        );
        assert!(ready_source.source_config_ready());
        assert!(!ready_source.signed_live_prerequisite_ready());
        assert_eq!(
            ready_source.missing_prerequisite(),
            Some("verify the signed-live kernel artifact was rebuilt from kernel/config-arm64")
        );
    }

    #[test]
    fn converts_emitted_json_to_benchmark_samples() {
        let samples = GuestIoPressure::from_emitted_json(
            br#"{"signed_live":true,"source":"/proc/pressure/io","io_full_avg10":1.5,"io_some_avg10":0.25}"#,
        )
        .expect("samples")
        .into_samples();

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].metric(), IO_FULL_AVG10_METRIC);
        assert_eq!(samples[0].kind(), BenchmarkMetricKind::WorkloadResource);
        assert_eq!(samples[0].unit(), BenchmarkUnit::Percent);
        assert_eq!(samples[0].value(), 1.5);
        assert_eq!(samples[1].metric(), IO_SOME_AVG10_METRIC);
        assert_eq!(samples[1].value(), 0.25);
    }
}
