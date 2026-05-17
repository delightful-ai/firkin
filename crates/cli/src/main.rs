//! Development CLI for exercising the firkin library.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Display;
use std::io::Write;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use clap::{Parser, Subcommand, ValueEnum};
use firkin::e2b::{
    ControlPlaneHttpServer, DomainProxyTlsIdentity, HostRuntimeAdapter, LifecycleClock,
    LocalRuntimeBackend, SystemLifecycleClock,
};
use firkin::oci::{Client, Reference};
use firkin::types::Hostname;
use firkin::vminitd_bytes::{VMEXEC_AARCH64, VMINITD_AARCH64};
use firkin::vmm::{BootLog, KernelImage, Network, VirtualMachine, VmConfig};
use firkin::{Container, Platform, Rootfs};

const IO_FULL_AVG10_METRIC: &str = "sandbox.pressure.io_full_avg10";
const P0_MEMORY_METRICS: &[&str] = &[
    "sandbox.mem.idle_host_footprint_bytes",
    "sandbox.mem.post_task_residual_bytes",
    "sandbox.mem.reclaim_effectiveness_ratio",
];

#[derive(Debug, Parser)]
#[command(name = "fk", version, about = "Development CLI for firkin")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Pull an OCI image into the local firkin cache.
    Pull(ImageArgs),
    /// Pull an OCI image and run a command in an implicit VM.
    Run(RunArgs),
    /// Inspect or delete Firkin-owned temporary runtime artifacts.
    Clear(ClearArgs),
    /// Print Firkin CLI and library configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Debug host/runtime readiness.
    Debug {
        #[command(subcommand)]
        command: DebugCommand,
    },
    /// Run local E2B-compatible development services.
    E2b {
        #[command(subcommand)]
        command: E2bCommand,
    },
    /// Inspect, summarize, and gate Firkin benchmark evidence.
    Benchmark {
        #[command(subcommand)]
        command: BenchmarkCommand,
    },
    /// Inspect production-substrate contracts and acceptance targets.
    Substrate {
        #[command(subcommand)]
        command: SubstrateCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DebugCommand {
    /// Print Virtualization.framework capability and signing preflight data.
    Preflight,
    /// Boot vminitd in a VM, print assigned runtime state, then stop it.
    Boot(DebugBootArgs),
}

#[derive(Debug, Subcommand)]
enum E2bCommand {
    /// Serve the host-backed local E2B control plane and domain proxy.
    Host(E2bHostArgs),
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Print the effective Firkin storage roots.
    Show(ConfigShowArgs),
}

#[derive(Debug, Subcommand)]
enum BenchmarkCommand {
    /// Print the stable Firkin benchmark metric catalog.
    Catalog,
    /// Print the hard-cut P0 benchmark contract.
    P0Contract,
    /// Print the autoscale efficiency benchmark contract.
    AutoscaleContract,
    /// Print the decision-grade benchmark metric contract.
    MetricContract,
    /// Print benchmark metric phase ownership policy.
    PhaseOwners,
    /// Print live measurement coverage for required P0 scorecard metrics.
    Coverage(BenchmarkCoverageArgs),
    /// Print the P0 memory attribution collector promotion gate.
    MemoryAttribution,
    /// Preflight benchmark prerequisites without running a benchmark.
    Doctor(BenchmarkDoctorArgs),
    /// Run a benchmark suite and write the requested evidence artifact.
    Run(BenchmarkRunArgs),
    /// Save and list local benchmark baselines.
    Baseline {
        #[command(subcommand)]
        command: BenchmarkBaselineCommand,
    },
    /// Compare two benchmark evidence artifacts.
    Compare(BenchmarkCompareArgs),
    /// Generate an HTML proof artifact for one milestone.
    Proof(BenchmarkProofArgs),
    /// Check whether the local benchmark loop is ready for a performance sprint.
    SprintReady(BenchmarkSprintReadyArgs),
    /// Write a markdown P0 optimization sprint record.
    SprintRecord(BenchmarkSprintRecordArgs),
    /// Print benchmark suites and the metric each case should emit.
    Suites(BenchmarkSuitesArgs),
    /// Print lifecycle and overhead benchmark SLO targets.
    Targets,
    /// Validate raw `BenchmarkSample` JSON and write an agent scorecard artifact.
    WriteScorecard(WriteScorecardArgs),
    /// Validate raw `BenchmarkSample` JSON and write an autoscale scorecard artifact.
    WriteAutoscaleScorecard(WriteAutoscaleScorecardArgs),
    /// Validate raw `BenchmarkSample` JSON and write an agent-computer scorecard artifact.
    WriteAgentComputerScorecard(WriteAgentComputerScorecardArgs),
    /// Validate an agent scorecard artifact against required P0 metrics.
    ValidateScorecard(ValidateScorecardArgs),
    /// Validate an autoscale scorecard artifact against required autoscale metrics.
    ValidateAutoscaleScorecard(ValidateAutoscaleScorecardArgs),
    /// Validate an agent-computer scorecard artifact against required product-path metrics.
    ValidateAgentComputerScorecard(ValidateAgentComputerScorecardArgs),
    /// Validate a lifecycle benchmark artifact against configured SLO targets.
    ValidateLifecycleSlo(ValidateLifecycleSloArgs),
    /// Validate a Firkin overhead artifact against configured SLO targets.
    ValidateOverheadSlo(ValidateOverheadSloArgs),
    /// Validate a single-node soak evidence artifact.
    ValidateSoak(ValidateSoakArgs),
    /// Print p50/p90/p95/p99/max summaries from an agent scorecard artifact.
    ReportScorecard(ReportScorecardArgs),
    /// Print p50/p90/p95/p99/max summaries from an autoscale scorecard artifact.
    ReportAutoscaleScorecard(ReportAutoscaleScorecardArgs),
    /// Print p50/p90/p95/p99/max summaries from an agent-computer scorecard artifact.
    ReportAgentComputerScorecard(ReportAgentComputerScorecardArgs),
    /// Print phase summaries from an agent-computer raw trace sidecar.
    ReportAgentComputerTraces(ReportAgentComputerTracesArgs),
    /// Print p50/p90/p95/p99/max summaries from a benchmark evidence artifact.
    Report(BenchmarkReportArgs),
}

#[derive(Debug, Parser)]
#[allow(clippy::struct_field_names)]
struct ConfigShowArgs {
    /// Durable state root. Defaults to `$FIRKIN_STATE_DIR` or `~/.firkin/state`.
    #[arg(long)]
    state_root: Option<PathBuf>,
    /// Rebuildable cache root. Defaults to `$FIRKIN_CACHE_DIR` or `~/.firkin/cache`.
    #[arg(long)]
    cache_root: Option<PathBuf>,
    /// Benchmark artifact root. Defaults to `$FIRKIN_BENCHMARK_DIR` or `~/.firkin/benchmarks`.
    #[arg(long)]
    benchmark_root: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum BenchmarkMode {
    /// Host-only proof or inspection that does not run Apple/VZ.
    HostOnly,
    /// Signed live Apple/VZ path through the repo-local harness.
    SignedLive,
}

impl BenchmarkMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::HostOnly => "host-only",
            Self::SignedLive => "signed-live",
        }
    }
}

#[derive(Debug, Parser)]
struct BenchmarkCoverageArgs {
    /// Fail if any P0 metric is not exact enough for optimization.
    #[arg(long)]
    strict: bool,
    /// Optional benchmark, overhead, or scorecard artifact to include in the coverage report.
    #[arg(long)]
    artifact: Vec<PathBuf>,
}

#[derive(Debug, Parser)]
struct BenchmarkDoctorArgs {
    /// Benchmark mode to preflight.
    #[arg(long, value_enum, default_value_t = BenchmarkMode::SignedLive)]
    mode: BenchmarkMode,
    /// Durable state root. Defaults to `$FIRKIN_STATE_DIR` or `~/.firkin/state`.
    #[arg(long)]
    state_root: Option<PathBuf>,
    /// Rebuildable cache root. Defaults to `$FIRKIN_CACHE_DIR` or `~/.firkin/cache`.
    #[arg(long)]
    cache_root: Option<PathBuf>,
    /// Benchmark artifact root. Defaults to `$FIRKIN_BENCHMARK_DIR` or `~/.firkin/benchmarks`.
    #[arg(long)]
    benchmark_root: Option<PathBuf>,
    /// Minimum free bytes required on the benchmark artifact volume.
    #[arg(long, default_value_t = 15 * 1024 * 1024 * 1024_u64)]
    min_free_bytes: u64,
}

#[derive(Debug, Parser)]
struct BenchmarkRunArgs {
    /// Benchmark suite to run, for example `agent-core` or `overhead`.
    suite: String,
    /// Benchmark mode.
    #[arg(long, value_enum, default_value_t = BenchmarkMode::SignedLive)]
    mode: BenchmarkMode,
    /// Target run duration label. Signed-live lifecycle currently maps this to repeat count.
    #[arg(long, default_value = "60s")]
    duration: BenchmarkDuration,
    /// Output evidence artifact.
    #[arg(long)]
    out: PathBuf,
    /// Reuse the newest matching signed-live test binary instead of invoking Cargo.
    #[arg(long)]
    no_build: bool,
}

#[derive(Debug, Subcommand)]
enum BenchmarkBaselineCommand {
    /// Save an artifact as a named local baseline.
    Save(BenchmarkBaselineSaveArgs),
    /// List saved local baselines.
    List(BenchmarkBaselineListArgs),
}

#[derive(Debug, Parser)]
struct BenchmarkBaselineSaveArgs {
    /// Evidence artifact to save.
    artifact: PathBuf,
    /// Baseline name.
    #[arg(long)]
    name: String,
    /// Benchmark artifact root. Defaults to `$FIRKIN_BENCHMARK_DIR` or `~/.firkin/benchmarks`.
    #[arg(long)]
    benchmark_root: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct BenchmarkBaselineListArgs {
    /// Benchmark artifact root. Defaults to `$FIRKIN_BENCHMARK_DIR` or `~/.firkin/benchmarks`.
    #[arg(long)]
    benchmark_root: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct BenchmarkCompareArgs {
    /// Baseline evidence artifact.
    baseline: PathBuf,
    /// Current evidence artifact.
    current: PathBuf,
    /// Ranking mode.
    #[arg(long, value_enum, default_value_t = BenchmarkCompareRank::Bottlenecks)]
    rank: BenchmarkCompareRank,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum BenchmarkCompareRank {
    /// Rank current bottlenecks by p95.
    Bottlenecks,
    /// Rank regressions by p95 delta.
    Regressions,
    /// Rank improvements by p95 delta.
    Improvements,
}

impl BenchmarkCompareRank {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Bottlenecks => "bottlenecks",
            Self::Regressions => "regressions",
            Self::Improvements => "improvements",
        }
    }
}

#[derive(Debug, Parser)]
struct BenchmarkProofArgs {
    /// Milestone id, for example `m1`, `m2`, or `m3`.
    milestone: String,
    /// Evidence text or JSON artifact to summarize.
    #[arg(long = "from")]
    source: PathBuf,
    /// Output HTML artifact.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Parser)]
struct BenchmarkSprintReadyArgs {
    /// Suite to optimize.
    #[arg(long, default_value = "agent-core")]
    suite: String,
    /// Named baseline to require.
    #[arg(long)]
    baseline: String,
    /// Benchmark mode.
    #[arg(long, value_enum, default_value_t = BenchmarkMode::SignedLive)]
    mode: BenchmarkMode,
    /// Optional current artifact to compare against the baseline.
    #[arg(long)]
    current_artifact: Option<PathBuf>,
    /// Required overhead artifact to prove instrumentation overhead stays bounded.
    #[arg(long)]
    overhead_artifact: Option<PathBuf>,
    /// Optional scorecard artifact carrying exact required P0 metrics.
    #[arg(long)]
    scorecard_artifact: Option<PathBuf>,
    /// Benchmark artifact root. Defaults to `$FIRKIN_BENCHMARK_DIR` or `~/.firkin/benchmarks`.
    #[arg(long)]
    benchmark_root: Option<PathBuf>,
    /// Minimum free bytes required on the benchmark artifact volume.
    #[arg(long, default_value_t = 15 * 1024 * 1024 * 1024_u64)]
    min_free_bytes: u64,
}

#[derive(Debug, Parser)]
struct BenchmarkSprintRecordArgs {
    /// Suite to optimize.
    #[arg(long, default_value = "agent-core")]
    suite: String,
    /// Named baseline to require.
    #[arg(long)]
    baseline: String,
    /// Benchmark mode used in embedded operator commands.
    #[arg(long, value_enum, default_value_t = BenchmarkMode::SignedLive)]
    mode: BenchmarkMode,
    /// Current evidence artifact to compare against the baseline.
    #[arg(long)]
    current_artifact: PathBuf,
    /// Required overhead artifact to prove instrumentation overhead stays bounded.
    #[arg(long)]
    overhead_artifact: PathBuf,
    /// Optional scorecard artifact carrying exact required P0 metrics.
    #[arg(long)]
    scorecard_artifact: Option<PathBuf>,
    /// Markdown output path.
    #[arg(long)]
    out: PathBuf,
    /// Benchmark artifact root. Defaults to `$FIRKIN_BENCHMARK_DIR` or `~/.firkin/benchmarks`.
    #[arg(long)]
    benchmark_root: Option<PathBuf>,
    /// Minimum free bytes required on the benchmark artifact volume.
    #[arg(long, default_value_t = 15 * 1024 * 1024 * 1024_u64)]
    min_free_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BenchmarkDuration(Duration);

impl BenchmarkDuration {
    const fn as_secs(self) -> u64 {
        self.0.as_secs()
    }
}

impl FromStr for BenchmarkDuration {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_duration(value).map(Self)
    }
}

#[derive(Debug, Subcommand)]
enum SubstrateCommand {
    /// Print production-substrate acceptance checks and current evidence state.
    AcceptanceChecklist,
    /// Validate a single-node soak evidence artifact.
    ValidateSoak(ValidateSoakArgs),
    /// Write manifest and integrity sidecars for an existing snapshot artifact.
    SnapshotSidecars(SubstrateSnapshotSidecarsArgs),
    /// Run one snapshot artifact GC and log rotation hygiene pass.
    HygieneOnce(SubstrateHygieneOnceArgs),
    /// Run periodic snapshot artifact GC and log rotation until interrupted.
    HygieneDaemon(SubstrateHygieneDaemonArgs),
    /// Render a launchd plist for the periodic hygiene daemon.
    HygieneLaunchdPlist(SubstrateHygieneLaunchdPlistArgs),
    /// Write a launchd plist for the periodic hygiene daemon to disk.
    HygieneLaunchdInstall(SubstrateHygieneLaunchdInstallArgs),
    /// Bootstrap and kickstart a launchd hygiene daemon plist.
    HygieneLaunchdBootstrap(SubstrateHygieneLaunchdBootstrapArgs),
    /// Print launchd state for the hygiene daemon.
    HygieneLaunchdStatus(SubstrateHygieneLaunchdStatusArgs),
    /// Render a launchd plist for periodic one-shot reconciliation.
    ReconcileLaunchdPlist(SubstrateReconcileLaunchdPlistArgs),
    /// Write a launchd plist for periodic one-shot reconciliation to disk.
    ReconcileLaunchdInstall(SubstrateReconcileLaunchdInstallArgs),
    /// Bootstrap and kickstart a launchd reconcile plist.
    ReconcileLaunchdBootstrap(SubstrateReconcileLaunchdBootstrapArgs),
    /// Print launchd state for the reconcile job.
    ReconcileLaunchdStatus(SubstrateReconcileLaunchdStatusArgs),
    /// Run one filesystem restart/stuck-VM reconciliation pass.
    ReconcileOnce(SubstrateReconcileOnceArgs),
    /// Print stuck-VM cleanup decisions from heartbeat observations.
    StuckVmPlan(SubstrateStuckVmPlanArgs),
    /// Scan filesystem runtime markers and print restart/stuck-VM decisions.
    HostScan(SubstrateHostScanArgs),
}

#[derive(Debug, Parser)]
struct ValidateLifecycleSloArgs {
    /// Benchmark evidence JSON artifact to validate.
    artifact: PathBuf,
    /// Minimum sample count required per lifecycle metric.
    #[arg(long, default_value_t = 1)]
    min_samples: usize,
}

#[derive(Debug, Parser)]
struct ValidateOverheadSloArgs {
    /// Firkin overhead evidence JSON artifact to validate.
    artifact: PathBuf,
    /// Minimum sample count required per overhead metric.
    #[arg(long, default_value_t = 1)]
    min_samples: usize,
}

#[derive(Debug, Parser)]
struct BenchmarkReportArgs {
    /// Benchmark artifact kind to read.
    kind: BenchmarkReportKind,
    /// Benchmark evidence JSON artifact to summarize.
    artifact: PathBuf,
}

#[derive(Debug, Parser)]
struct BenchmarkSuitesArgs {
    /// Optional suite id to print. Omit to print every suite.
    suite: Option<String>,
}

#[derive(Debug, Parser)]
struct WriteScorecardArgs {
    /// Raw JSON array of `BenchmarkSample`s to validate.
    samples: PathBuf,
    /// Output agent scorecard evidence JSON artifact.
    artifact: PathBuf,
    /// Minimum sample count required per P0 dashboard metric.
    #[arg(long, default_value_t = 1)]
    min_samples: usize,
}

#[derive(Debug, Parser)]
struct WriteAutoscaleScorecardArgs {
    /// Raw JSON array of `BenchmarkSample`s to validate.
    samples: PathBuf,
    /// Output autoscale scorecard evidence JSON artifact.
    artifact: PathBuf,
    /// Minimum sample count required per autoscale dashboard metric.
    #[arg(long, default_value_t = 1)]
    min_samples: usize,
}

#[derive(Debug, Parser)]
struct WriteAgentComputerScorecardArgs {
    /// Raw JSON array of `BenchmarkSample`s to validate.
    samples: PathBuf,
    /// Output agent-computer scorecard evidence JSON artifact.
    artifact: PathBuf,
    /// Minimum sample count required per agent-computer dashboard metric.
    #[arg(long, default_value_t = 1)]
    min_samples: usize,
}

#[derive(Debug, Parser)]
struct ValidateScorecardArgs {
    /// Agent scorecard evidence JSON artifact to validate.
    artifact: PathBuf,
    /// Minimum sample count required per P0 dashboard metric.
    #[arg(long, default_value_t = 1)]
    min_samples: usize,
    /// Fail if any P0 dashboard metric misses its snappy target.
    #[arg(long)]
    require_snappy: bool,
}

#[derive(Debug, Parser)]
struct ValidateAutoscaleScorecardArgs {
    /// Autoscale scorecard evidence JSON artifact to validate.
    artifact: PathBuf,
    /// Minimum sample count required per autoscale dashboard metric.
    #[arg(long, default_value_t = 1)]
    min_samples: usize,
    /// Fail if any autoscale dashboard metric is not promotion-grade.
    #[arg(long)]
    require_promotable: bool,
    /// Fail if any autoscale dashboard metric misses its snappy target.
    #[arg(long)]
    require_snappy: bool,
}

#[derive(Debug, Parser)]
struct ValidateAgentComputerScorecardArgs {
    /// Agent-computer scorecard evidence JSON artifact to validate.
    artifact: PathBuf,
    /// Minimum sample count required per agent-computer dashboard metric.
    #[arg(long, default_value_t = 1)]
    min_samples: usize,
    /// Fail if any product-path scorecard metric is not promotion-grade.
    #[arg(long)]
    require_promotable: bool,
    /// Fail if any product-path scorecard metric misses its snappy target.
    #[arg(long)]
    require_snappy: bool,
}

#[derive(Debug, Parser)]
struct ReportScorecardArgs {
    /// Agent scorecard evidence JSON artifact to summarize.
    artifact: PathBuf,
}

#[derive(Debug, Parser)]
struct ReportAutoscaleScorecardArgs {
    /// Autoscale scorecard evidence JSON artifact to summarize.
    artifact: PathBuf,
}

#[derive(Debug, Parser)]
struct ReportAgentComputerScorecardArgs {
    /// Agent-computer scorecard evidence JSON artifact to summarize.
    artifact: PathBuf,
}

#[derive(Debug, Parser)]
struct ReportAgentComputerTracesArgs {
    /// Agent-computer raw `SandboxEventTrace` sidecar JSON artifact to summarize.
    artifact: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum BenchmarkReportKind {
    /// Lifecycle latency evidence artifact.
    Lifecycle,
    /// Firkin overhead evidence artifact.
    Overhead,
    /// Decision-grade confidence report for any supported benchmark artifact.
    Decision,
}

#[derive(Debug, Parser)]
struct ValidateSoakArgs {
    /// Soak evidence JSON artifact to validate.
    artifact: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum SnapshotSidecarKind {
    /// Base template snapshot used as a normal session-create source.
    BaseTemplate,
    /// Continuation snapshot used to resume a prior session.
    Continuation,
}

impl SnapshotSidecarKind {
    const fn manifest_kind(self) -> firkin::substrate::SnapshotArtifactKind {
        match self {
            Self::BaseTemplate => firkin::substrate::SnapshotArtifactKind::BaseTemplate,
            Self::Continuation => firkin::substrate::SnapshotArtifactKind::Continuation,
        }
    }

    const fn output_label(self) -> &'static str {
        match self {
            Self::BaseTemplate => "base_template",
            Self::Continuation => "continuation",
        }
    }
}

#[derive(Debug, Parser)]
struct SubstrateSnapshotSidecarsArgs {
    /// Existing snapshot artifact to register.
    #[arg(long)]
    artifact: PathBuf,
    /// Operator-visible snapshot/template/session identifier.
    #[arg(long)]
    logical_id: String,
    /// Snapshot artifact kind.
    #[arg(long)]
    kind: SnapshotSidecarKind,
}

#[derive(Debug, Parser)]
struct SubstrateHygieneOnceArgs {
    /// Snapshot artifact root to garbage collect.
    #[arg(long)]
    snapshot_root: PathBuf,
    /// Runtime log root to rotate.
    #[arg(long)]
    log_root: PathBuf,
    /// Directory containing direct `*.manifest.json` snapshot sidecars.
    #[arg(long)]
    manifest_root: Option<PathBuf>,
    /// Rotate logs larger than this many bytes.
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    max_log_bytes: u64,
    /// Compress rotated logs as `.gz`.
    #[arg(long)]
    gzip_logs: bool,
}

#[derive(Debug, Parser)]
struct SubstrateHygieneDaemonArgs {
    /// Snapshot artifact root to garbage collect.
    #[arg(long)]
    snapshot_root: PathBuf,
    /// Runtime log root to rotate.
    #[arg(long)]
    log_root: PathBuf,
    /// Directory containing direct `*.manifest.json` snapshot sidecars.
    #[arg(long)]
    manifest_root: Option<PathBuf>,
    /// Rotate logs larger than this many bytes.
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    max_log_bytes: u64,
    /// Seconds between hygiene ticks.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    interval_seconds: u64,
    /// Compress rotated logs as `.gz`.
    #[arg(long)]
    gzip_logs: bool,
}

#[derive(Debug, Parser)]
struct SubstrateHygieneLaunchdPlistArgs {
    /// launchd job label.
    #[arg(long, default_value = "com.firkin.substrate.hygiene")]
    label: String,
    /// Absolute path to the signed `fk` binary launchd should execute.
    #[arg(long)]
    fk_bin: PathBuf,
    /// Snapshot artifact root to garbage collect.
    #[arg(long)]
    snapshot_root: PathBuf,
    /// Runtime log root to rotate.
    #[arg(long)]
    log_root: PathBuf,
    /// Directory containing direct `*.manifest.json` snapshot sidecars.
    #[arg(long)]
    manifest_root: Option<PathBuf>,
    /// Rotate logs larger than this many bytes.
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    max_log_bytes: u64,
    /// Seconds between hygiene ticks.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    interval_seconds: u64,
    /// Compress rotated logs as `.gz`.
    #[arg(long)]
    gzip_logs: bool,
    /// launchd `StandardOutPath`.
    #[arg(long)]
    standard_out_path: Option<PathBuf>,
    /// launchd `StandardErrorPath`.
    #[arg(long)]
    standard_error_path: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct SubstrateHygieneLaunchdInstallArgs {
    /// Destination plist path, for example `~/Library/LaunchAgents/com.firkin.substrate.hygiene.plist`.
    #[arg(long)]
    plist_path: PathBuf,
    /// launchd plist content.
    #[command(flatten)]
    launchd: SubstrateHygieneLaunchdPlistArgs,
}

#[derive(Debug, Parser)]
struct SubstrateHygieneLaunchdBootstrapArgs {
    /// launchd domain, for example `gui/501` or `system`.
    #[arg(long)]
    domain: String,
    /// launchd job label to kickstart after bootstrap.
    #[arg(long, default_value = "com.firkin.substrate.hygiene")]
    label: String,
    /// Destination plist path to bootstrap.
    #[arg(long)]
    plist_path: PathBuf,
}

#[derive(Debug, Parser)]
struct SubstrateHygieneLaunchdStatusArgs {
    /// launchd domain, for example `gui/501` or `system`.
    #[arg(long)]
    domain: String,
    /// launchd job label.
    #[arg(long, default_value = "com.firkin.substrate.hygiene")]
    label: String,
}

#[derive(Debug, Parser)]
struct SubstrateReconcileLaunchdPlistArgs {
    /// launchd job label.
    #[arg(long, default_value = "com.firkin.substrate.reconcile")]
    label: String,
    /// Absolute path to the signed `fk` binary launchd should execute.
    #[arg(long)]
    fk_bin: PathBuf,
    /// Directory containing active VM heartbeat marker files.
    #[arg(long)]
    active_vm_root: PathBuf,
    /// Directory containing snapshot artifact markers.
    #[arg(long)]
    snapshot_root: PathBuf,
    /// Directory containing runtime log markers.
    #[arg(long)]
    log_root: PathBuf,
    /// Directory containing stale runtime process markers.
    #[arg(long)]
    process_root: PathBuf,
    /// Directory that receives quarantined ambiguous runtime markers.
    #[arg(long)]
    quarantine_root: PathBuf,
    /// Heartbeat age threshold in seconds before a VM should be cleaned up.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    heartbeat_timeout_seconds: u64,
    /// Seconds between one-shot reconciliation launches.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    interval_seconds: u64,
    /// launchd `StandardOutPath`.
    #[arg(long)]
    standard_out_path: Option<PathBuf>,
    /// launchd `StandardErrorPath`.
    #[arg(long)]
    standard_error_path: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct SubstrateReconcileLaunchdInstallArgs {
    /// Destination plist path, for example `~/Library/LaunchAgents/com.firkin.substrate.reconcile.plist`.
    #[arg(long)]
    plist_path: PathBuf,
    /// launchd plist content.
    #[command(flatten)]
    launchd: SubstrateReconcileLaunchdPlistArgs,
}

#[derive(Debug, Parser)]
struct SubstrateReconcileLaunchdBootstrapArgs {
    /// launchd domain, for example `gui/501` or `system`.
    #[arg(long)]
    domain: String,
    /// launchd job label to kickstart after bootstrap.
    #[arg(long, default_value = "com.firkin.substrate.reconcile")]
    label: String,
    /// Destination plist path to bootstrap.
    #[arg(long)]
    plist_path: PathBuf,
}

#[derive(Debug, Parser)]
struct SubstrateReconcileLaunchdStatusArgs {
    /// launchd domain, for example `gui/501` or `system`.
    #[arg(long)]
    domain: String,
    /// launchd job label.
    #[arg(long, default_value = "com.firkin.substrate.reconcile")]
    label: String,
}

#[derive(Debug, Parser)]
struct SubstrateStuckVmPlanArgs {
    /// Heartbeat age threshold in seconds before a VM should be cleaned up.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    heartbeat_timeout_seconds: u64,
    /// Observed VM heartbeat age as `VM_ID=AGE_SECONDS`. Use an empty `VM_ID` to model an ambiguous record.
    #[arg(long = "vm", value_name = "VM_ID=AGE_SECONDS")]
    vms: Vec<StuckVmCliObservation>,
}

#[derive(Debug, Parser)]
struct SubstrateHostScanArgs {
    /// Directory containing active VM heartbeat marker files.
    #[arg(long)]
    active_vm_root: PathBuf,
    /// Directory containing snapshot artifact markers.
    #[arg(long)]
    snapshot_root: PathBuf,
    /// Directory containing runtime log markers.
    #[arg(long)]
    log_root: PathBuf,
    /// Directory containing stale runtime process markers.
    #[arg(long)]
    process_root: PathBuf,
    /// Heartbeat age threshold in seconds before a VM should be cleaned up.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    heartbeat_timeout_seconds: u64,
}

#[derive(Debug, Parser)]
struct SubstrateReconcileOnceArgs {
    /// Directory containing active VM heartbeat marker files.
    #[arg(long)]
    active_vm_root: PathBuf,
    /// Directory containing snapshot artifact markers.
    #[arg(long)]
    snapshot_root: PathBuf,
    /// Directory containing runtime log markers.
    #[arg(long)]
    log_root: PathBuf,
    /// Directory containing stale runtime process markers.
    #[arg(long)]
    process_root: PathBuf,
    /// Directory that receives quarantined ambiguous runtime markers.
    #[arg(long)]
    quarantine_root: PathBuf,
    /// Heartbeat age threshold in seconds before a VM should be cleaned up.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
    heartbeat_timeout_seconds: u64,
}

#[derive(Debug, serde::Serialize)]
struct SubstrateHostScanReport<'a> {
    host_scan: &'static str,
    heartbeat_timeout_seconds: u64,
    restart_decision_count: usize,
    stuck_vm_decision_count: usize,
    restart_decisions: Vec<SubstrateRestartDecisionReport<'a>>,
    stuck_vm_decisions: Vec<SubstrateStuckVmDecisionReport<'a>>,
}

#[derive(Debug, serde::Serialize)]
struct SubstrateRestartDecisionReport<'a> {
    id: &'a str,
    kind: &'static str,
    decision: &'static str,
}

#[derive(Debug, serde::Serialize)]
struct SubstrateReconcileOnceReport {
    reconcile_once: &'static str,
    restart: SubstrateRestartReconcileCounts,
    stuck_vm: SubstrateStuckVmReconcileCounts,
}

#[derive(Debug, serde::Serialize)]
struct SubstrateRestartReconcileCounts {
    recovered: usize,
    cleaned: usize,
    quarantined: usize,
}

#[derive(Debug, serde::Serialize)]
struct SubstrateStuckVmReconcileCounts {
    preserved: usize,
    cleaned: usize,
    quarantined: usize,
}

#[derive(Debug, serde::Serialize)]
struct SubstrateStuckVmDecisionReport<'a> {
    id: &'a str,
    heartbeat_age_seconds: u64,
    runtime_pid: Option<u32>,
    decision: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StuckVmCliObservation {
    id: String,
    heartbeat_age_seconds: u64,
}

impl FromStr for StuckVmCliObservation {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (id, age) = value
            .split_once('=')
            .ok_or_else(|| "expected VM_ID=AGE_SECONDS".to_owned())?;
        let heartbeat_age_seconds = age
            .parse::<u64>()
            .map_err(|error| format!("invalid AGE_SECONDS `{age}`: {error}"))?;
        Ok(Self {
            id: id.to_owned(),
            heartbeat_age_seconds,
        })
    }
}

#[derive(Debug, Parser)]
struct ImageArgs {
    /// OCI image reference.
    image: String,
    /// Target platform: linux/arm64, linux/arm64/v8, or linux/amd64.
    #[arg(long)]
    platform: Option<String>,
}

#[derive(Debug, Parser)]
struct RunArgs {
    /// OCI image reference.
    image: String,
    /// Target platform: linux/arm64, linux/arm64/v8, or linux/amd64.
    #[arg(long)]
    platform: Option<String>,
    /// Command and arguments to run. Use `--` before values that begin with `-`.
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

#[derive(Debug, Parser)]
#[allow(clippy::struct_excessive_bools)]
struct ClearArgs {
    /// Include durable Firkin state root.
    #[arg(long)]
    state: bool,
    /// Include rebuildable Firkin cache root.
    #[arg(long)]
    cache: bool,
    /// Include benchmark artifact root.
    #[arg(long)]
    benchmarks: bool,
    /// Include state, cache, and benchmark roots.
    #[arg(long)]
    all: bool,
    /// Include legacy TMPDIR Firkin roots. This is opt-in and never implied.
    #[arg(long)]
    legacy_tmp: bool,
    /// Report what would be removed. This is the default unless `--yes` is set.
    #[arg(long)]
    dry_run: bool,
    /// Delete selected roots. Without this flag, `fk clear` only reports what
    /// would be removed.
    #[arg(long)]
    yes: bool,
    /// Durable state root to scan. Defaults to `$FIRKIN_STATE_DIR` or
    /// `~/.firkin/state`.
    #[arg(long)]
    state_root: Option<PathBuf>,
    /// Rebuildable cache root to scan with `--include-caches`. Defaults to
    /// `$FIRKIN_CACHE_DIR` or `~/.firkin/cache`.
    #[arg(long)]
    cache_root: Option<PathBuf>,
    /// Benchmark artifact root to scan. Defaults to `$FIRKIN_BENCHMARK_DIR` or
    /// `~/.firkin/benchmarks`.
    #[arg(long)]
    benchmark_root: Option<PathBuf>,
    /// Temp root to scan for legacy Firkin runtime artifacts. Defaults to the
    /// process TMPDIR.
    #[arg(long)]
    tmp_root: Option<PathBuf>,
    /// Only include roots whose own mtime is older than this duration. Supports
    /// suffixes `s`, `m`, `h`, and `d`.
    #[arg(long)]
    older_than: Option<ClearOlderThan>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ClearOlderThan(Duration);

impl ClearOlderThan {
    const fn as_duration(self) -> Duration {
        self.0
    }
}

impl FromStr for ClearOlderThan {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_duration(value).map(Self)
    }
}

#[derive(Debug, Parser)]
struct DebugBootArgs {
    /// Kernel image path. Defaults to `FIRKIN_KERNEL_PATH` or repo-local `bin/vmlinux`.
    #[arg(long)]
    kernel: Option<PathBuf>,
    /// Append serial boot output to this file.
    #[arg(long)]
    boot_log: Option<PathBuf>,
    /// Use vmnet shared networking with the given CIDR subnet instead of NAT.
    #[arg(long)]
    vmnet_subnet: Option<String>,
    /// Seconds to keep the VM running before shutdown.
    #[arg(long, default_value_t = 0)]
    hold_secs: u64,
}

#[derive(Debug, Parser)]
struct E2bHostArgs {
    /// Control-plane listen address.
    #[arg(long, default_value = "127.0.0.1:49980")]
    control_addr: SocketAddr,
    /// Domain-proxy listen address.
    #[arg(long, default_value = "127.0.0.1:49981")]
    proxy_addr: SocketAddr,
    /// SDK-visible sandbox domain.
    #[arg(long, default_value = "cube.localhost")]
    domain: String,
    /// Host runtime sandbox root. Defaults to `$FIRKIN_STATE_DIR/e2b/sandboxes`
    /// or `~/.firkin/state/e2b/sandboxes`.
    #[arg(long)]
    root: Option<PathBuf>,
    /// Control-plane state JSON path. Defaults to
    /// `$FIRKIN_STATE_DIR/e2b/state.json` or `~/.firkin/state/e2b/state.json`.
    #[arg(long)]
    state: Option<PathBuf>,
    /// Do not require the SDK-visible wildcard host to resolve locally.
    #[arg(long)]
    skip_domain_preflight: bool,
    /// Require this E2B API key on local control-plane requests.
    #[arg(long, env = "FIRKIN_E2B_API_KEY")]
    api_key: Option<String>,
    /// Seconds between sandbox timeout-expiration passes.
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..))]
    lifecycle_interval_seconds: u64,
    /// PEM certificate chain for HTTPS domain-proxy traffic.
    #[arg(long)]
    proxy_tls_cert: Option<PathBuf>,
    /// PEM private key for HTTPS domain-proxy traffic.
    #[arg(long)]
    proxy_tls_key: Option<PathBuf>,
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Pull(args) => pull(args).await,
        Command::Run(args) => Box::pin(run(args)).await,
        Command::Clear(args) => {
            run_clear(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Config {
            command: ConfigCommand::Show(args),
        } => {
            write_config_show(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Debug {
            command: DebugCommand::Preflight,
        } => debug_preflight(),
        Command::Debug {
            command: DebugCommand::Boot(args),
        } => debug_boot(args).await,
        Command::E2b {
            command: E2bCommand::Host(args),
        } => e2b_host(args).await,
        Command::Benchmark {
            command: BenchmarkCommand::Catalog,
        } => {
            write_benchmark_catalog(std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::P0Contract,
        } => {
            write_benchmark_p0_contract(std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::AutoscaleContract,
        } => {
            write_benchmark_autoscale_contract(std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::MetricContract,
        } => {
            write_benchmark_metric_contract(std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::PhaseOwners,
        } => {
            write_benchmark_phase_owners(std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::Coverage(args),
        } => {
            write_benchmark_coverage(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::MemoryAttribution,
        } => {
            write_benchmark_memory_attribution(std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::Doctor(args),
        } => {
            write_benchmark_doctor(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::Run(args),
        } => {
            run_benchmark_suite(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command:
                BenchmarkCommand::Baseline {
                    command: BenchmarkBaselineCommand::Save(args),
                },
        } => {
            save_benchmark_baseline(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command:
                BenchmarkCommand::Baseline {
                    command: BenchmarkBaselineCommand::List(args),
                },
        } => {
            list_benchmark_baselines(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::Compare(args),
        } => {
            compare_benchmark_artifacts(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::Proof(args),
        } => {
            write_benchmark_proof(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::SprintReady(args),
        } => {
            write_benchmark_sprint_ready(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::SprintRecord(args),
        } => {
            write_benchmark_sprint_record(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::Suites(args),
        } => {
            write_benchmark_suites(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::Targets,
        } => {
            write_benchmark_targets(std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::WriteScorecard(args),
        } => {
            write_scorecard_artifact(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::WriteAutoscaleScorecard(args),
        } => {
            write_autoscale_scorecard_artifact(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::WriteAgentComputerScorecard(args),
        } => {
            write_agent_computer_scorecard_artifact(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ValidateScorecard(args),
        } => {
            validate_scorecard(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ValidateAutoscaleScorecard(args),
        } => {
            validate_autoscale_scorecard(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ValidateAgentComputerScorecard(args),
        } => {
            validate_agent_computer_scorecard(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ValidateLifecycleSlo(args),
        } => {
            validate_lifecycle_slo(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ValidateOverheadSlo(args),
        } => {
            validate_overhead_slo(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ValidateSoak(args),
        }
        | Command::Substrate {
            command: SubstrateCommand::ValidateSoak(args),
        } => {
            validate_soak(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ReportScorecard(args),
        } => {
            write_scorecard_report(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ReportAutoscaleScorecard(args),
        } => {
            write_autoscale_scorecard_report(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ReportAgentComputerScorecard(args),
        } => {
            write_agent_computer_scorecard_report(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::ReportAgentComputerTraces(args),
        } => {
            write_agent_computer_trace_report(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Benchmark {
            command: BenchmarkCommand::Report(args),
        } => {
            write_benchmark_report(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::AcceptanceChecklist,
        } => {
            write_substrate_acceptance_checklist(std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::SnapshotSidecars(args),
        } => {
            write_substrate_snapshot_sidecars(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::HygieneOnce(args),
        } => {
            run_substrate_hygiene_once(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::HygieneDaemon(args),
        } => run_substrate_hygiene_daemon(&args, std::io::stdout()).await,
        Command::Substrate {
            command: SubstrateCommand::HygieneLaunchdPlist(args),
        } => {
            write_substrate_hygiene_launchd_plist(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::HygieneLaunchdInstall(args),
        } => {
            install_substrate_hygiene_launchd_plist(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::HygieneLaunchdBootstrap(args),
        } => {
            run_substrate_hygiene_launchd_bootstrap(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::HygieneLaunchdStatus(args),
        } => {
            run_substrate_hygiene_launchd_status(&args)?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::ReconcileLaunchdPlist(args),
        } => {
            write_substrate_reconcile_launchd_plist(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::ReconcileLaunchdInstall(args),
        } => {
            install_substrate_reconcile_launchd_plist(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::ReconcileLaunchdBootstrap(args),
        } => {
            run_substrate_reconcile_launchd_bootstrap(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::ReconcileLaunchdStatus(args),
        } => {
            run_substrate_reconcile_launchd_status(&args)?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::ReconcileOnce(args),
        } => {
            run_substrate_reconcile_once(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::StuckVmPlan(args),
        } => {
            write_substrate_stuck_vm_plan(&args, std::io::stdout())?;
            Ok(())
        }
        Command::Substrate {
            command: SubstrateCommand::HostScan(args),
        } => {
            write_substrate_host_scan(&args, std::io::stdout())?;
            Ok(())
        }
    }
}

async fn pull(args: ImageArgs) -> Result<(), Box<dyn Error>> {
    let reference = Reference::parse(&args.image)?;
    let bundle = client(args.platform.as_deref())?.pull(&reference).await?;

    println!("reference={}", bundle.reference());
    println!("digest={}", bundle.digest());
    println!("platform={:?}", bundle.platform());
    Ok(())
}

async fn run(args: RunArgs) -> Result<(), Box<dyn Error>> {
    let reference = Reference::parse(&args.image)?;
    let bundle = client(args.platform.as_deref())?.pull(&reference).await?;

    let mut builder = Container::builder("fk-run")?
        .image_config(bundle.config())
        .rootfs(Rootfs::oci_bundle(bundle));
    if !args.command.is_empty() {
        builder = builder.command(args.command);
    }

    let output = builder.output().await?;
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;

    if output.status.success() {
        Ok(())
    } else {
        std::process::exit(output.status.code().unwrap_or(1));
    }
}

fn write_config_show(args: &ConfigShowArgs, mut writer: impl Write) -> std::io::Result<()> {
    let storage = storage_config_from_roots(args.state_root.clone(), args.cache_root.clone());
    let benchmark_root = args
        .benchmark_root
        .clone()
        .unwrap_or_else(firkin_benchmark_root);
    writeln!(writer, "config=firkin-storage-v1")?;
    writeln!(writer, "state_root={}", storage.state_root().display())?;
    writeln!(writer, "cache_root={}", storage.cache_root().display())?;
    writeln!(writer, "benchmark_root={}", benchmark_root.display())?;
    writeln!(
        writer,
        "runtime_continuation_root={}",
        storage.runtime_continuation_root().display()
    )?;
    writeln!(
        writer,
        "runtime_restore_staging_root={}",
        storage.runtime_restore_staging_root().display()
    )?;
    writeln!(
        writer,
        "single_node_snapshot_root={}",
        storage.single_node_snapshot_root().display()
    )?;
    Ok(())
}

fn storage_config_from_roots(
    state_root: Option<PathBuf>,
    cache_root: Option<PathBuf>,
) -> firkin_runtime::FirkinStorageConfig {
    if state_root.is_none() && cache_root.is_none() {
        return firkin_runtime::FirkinStorageConfig::from_env();
    }
    firkin_runtime::FirkinStorageConfig::from_roots(
        state_root.unwrap_or_else(firkin_runtime::firkin_state_root),
        cache_root.unwrap_or_else(firkin_runtime::firkin_cache_root),
    )
}

fn firkin_benchmark_root() -> PathBuf {
    std::env::var_os("FIRKIN_BENCHMARK_DIR").map_or_else(
        || firkin_base_from_home(std::env::var_os("HOME")).join("benchmarks"),
        PathBuf::from,
    )
}

fn firkin_base_from_home(home: Option<std::ffi::OsString>) -> PathBuf {
    home.map_or_else(
        || PathBuf::from(".firkin"),
        |home| PathBuf::from(home).join(".firkin"),
    )
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    let (digits, multiplier) = if let Some(stripped) = value.strip_suffix('s') {
        (stripped, 1)
    } else if let Some(stripped) = value.strip_suffix('m') {
        (stripped, 60)
    } else if let Some(stripped) = value.strip_suffix('h') {
        (stripped, 60 * 60)
    } else if let Some(stripped) = value.strip_suffix('d') {
        (stripped, 24 * 60 * 60)
    } else {
        (value, 1)
    };
    let amount = digits
        .parse::<u64>()
        .map_err(|error| format!("invalid duration `{value}`: {error}"))?;
    Ok(Duration::from_secs(amount.saturating_mul(multiplier)))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClearRoot {
    label: &'static str,
    path: PathBuf,
    kind: ClearRootKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClearRootKind {
    State,
    LegacyTemp,
    Cache,
    Benchmarks,
}

impl ClearRootKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::LegacyTemp => "legacy_temp",
            Self::Cache => "cache",
            Self::Benchmarks => "benchmarks",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClearRootReport {
    label: &'static str,
    kind: ClearRootKind,
    path: PathBuf,
    exists: bool,
    bytes: u64,
    action: ClearAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClearAction {
    Missing,
    SkippedRecent,
    WouldDelete,
    Deleted,
}

impl ClearAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::SkippedRecent => "skipped_recent",
            Self::WouldDelete => "would_delete",
            Self::Deleted => "deleted",
        }
    }
}

fn run_clear(args: &ClearArgs, writer: impl Write) -> Result<(), Box<dyn Error>> {
    let tmp_root = args.tmp_root.clone().unwrap_or_else(std::env::temp_dir);
    let storage = storage_config_from_roots(args.state_root.clone(), args.cache_root.clone());
    let benchmark_root = args
        .benchmark_root
        .clone()
        .unwrap_or_else(firkin_benchmark_root);
    let selection = ClearSelection::from_args(args);
    let roots = clear_roots(&tmp_root, &storage, &benchmark_root, selection)?;
    write_clear_report(
        &roots,
        args.yes && !args.dry_run,
        args.older_than.map(ClearOlderThan::as_duration),
        writer,
    )?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
struct ClearSelection {
    state: bool,
    cache: bool,
    benchmarks: bool,
    legacy_tmp: bool,
}

impl ClearSelection {
    fn from_args(args: &ClearArgs) -> Self {
        let any_primary = args.state || args.cache || args.benchmarks || args.all;
        Self {
            state: args.all || args.state || !any_primary,
            cache: args.all || args.cache,
            benchmarks: args.all || args.benchmarks || !any_primary,
            legacy_tmp: args.legacy_tmp,
        }
    }
}

fn clear_roots(
    tmp_root: &Path,
    storage: &firkin_runtime::FirkinStorageConfig,
    benchmark_root: &Path,
    selection: ClearSelection,
) -> std::io::Result<Vec<ClearRoot>> {
    let mut roots = Vec::new();
    if selection.state {
        roots.push(ClearRoot {
            label: "firkin_state",
            path: storage.state_root().to_path_buf(),
            kind: ClearRootKind::State,
        });
    }
    if selection.cache {
        roots.push(ClearRoot {
            label: "firkin_cache",
            path: storage.cache_root().to_path_buf(),
            kind: ClearRootKind::Cache,
        });
    }
    if selection.benchmarks {
        roots.push(ClearRoot {
            label: "firkin_benchmarks",
            path: benchmark_root.to_path_buf(),
            kind: ClearRootKind::Benchmarks,
        });
    }
    if selection.legacy_tmp {
        roots.push(ClearRoot {
            label: "legacy_tmp_runtime_continuations",
            path: tmp_root.join("firkin-runtime-continuations"),
            kind: ClearRootKind::LegacyTemp,
        });
        roots.push(ClearRoot {
            label: "legacy_tmp_runtime_restore_staging",
            path: tmp_root.join("firkin-runtime-restore-staging"),
            kind: ClearRootKind::LegacyTemp,
        });
        roots.push(ClearRoot {
            label: "legacy_tmp_single_node_snapshots",
            path: tmp_root.join("firkin-single-node-snapshots"),
            kind: ClearRootKind::LegacyTemp,
        });
        roots.push(ClearRoot {
            label: "legacy_tmp_firkin_cache",
            path: tmp_root.join("firkin"),
            kind: ClearRootKind::Cache,
        });
        if let Ok(entries) = std::fs::read_dir(tmp_root) {
            for entry in entries {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if is_firkin_temp_cache_name(name) {
                    roots.push(ClearRoot {
                        label: "legacy_tmp_firkin_named_cache",
                        path: entry.path(),
                        kind: ClearRootKind::Cache,
                    });
                }
            }
        }
    }

    roots.sort_by(|left, right| left.path.cmp(&right.path));
    roots.dedup_by(|left, right| left.path == right.path);
    Ok(roots)
}

fn is_firkin_temp_cache_name(name: &str) -> bool {
    (name.starts_with("firkin-live-") && name.ends_with("-cache"))
        || name == "firkin-real-vm-replay-cache"
        || name == "firkin-single-node-proxy-test"
}

fn write_clear_report(
    roots: &[ClearRoot],
    delete: bool,
    older_than: Option<Duration>,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let mut reports = Vec::with_capacity(roots.len());
    for root in roots {
        reports.push(clear_root_report(root, delete, older_than)?);
    }

    let reclaimable_bytes = reports
        .iter()
        .filter(|report| report.exists)
        .map(|report| report.bytes)
        .sum::<u64>();
    writeln!(
        writer,
        "clear={} roots={} reclaimable_bytes={}",
        if delete { "deleted" } else { "dry_run" },
        reports.len(),
        reclaimable_bytes
    )?;
    for report in reports {
        writeln!(
            writer,
            "root={} kind={} path={} exists={} bytes={} action={}",
            report.label,
            report.kind.as_str(),
            report.path.display(),
            report.exists,
            report.bytes,
            report.action.as_str()
        )?;
    }
    if !delete {
        writeln!(writer, "hint=rerun_with_--yes_to_delete")?;
    }
    Ok(())
}

fn clear_root_report(
    root: &ClearRoot,
    delete: bool,
    older_than: Option<Duration>,
) -> Result<ClearRootReport, Box<dyn Error>> {
    validate_clear_root_safety(&root.path)?;
    if !root.path.exists() {
        return Ok(ClearRootReport {
            label: root.label,
            kind: root.kind,
            path: root.path.clone(),
            exists: false,
            bytes: 0,
            action: ClearAction::Missing,
        });
    }

    let bytes = path_size_bytes(&root.path)?;
    if let Some(older_than) = older_than
        && !path_is_older_than(&root.path, older_than)?
    {
        return Ok(ClearRootReport {
            label: root.label,
            kind: root.kind,
            path: root.path.clone(),
            exists: true,
            bytes,
            action: ClearAction::SkippedRecent,
        });
    }
    if delete {
        remove_path(&root.path)?;
    }
    Ok(ClearRootReport {
        label: root.label,
        kind: root.kind,
        path: root.path.clone(),
        exists: true,
        bytes,
        action: if delete {
            ClearAction::Deleted
        } else {
            ClearAction::WouldDelete
        },
    })
}

fn validate_clear_root_safety(path: &Path) -> std::io::Result<()> {
    if path.as_os_str().is_empty()
        || path == Path::new("/")
        || path == Path::new("/tmp")
        || path == std::env::temp_dir()
        || std::env::var_os("HOME").is_some_and(|home| path == Path::new(&home))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("refusing to clear unsafe Firkin root {}", path.display()),
        ));
    }
    Ok(())
}

fn path_is_older_than(path: &Path, age: Duration) -> std::io::Result<bool> {
    let modified = std::fs::symlink_metadata(path)?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|elapsed| elapsed >= age))
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

fn path_size_bytes(path: &Path) -> std::io::Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }

    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        total += path_size_bytes(&entry?.path())?;
    }
    Ok(total)
}

fn debug_preflight() -> Result<(), Box<dyn Error>> {
    let info = firkin::vmm::preflight()?;
    println!("macos={}", info.macos_version());
    println!("architecture={:?}", info.architecture());
    println!(
        "nested_virtualization_supported={}",
        info.nested_virtualization_supported()
    );
    println!("rosetta_available={}", info.rosetta_available());
    println!("codesigned={:?}", info.codesigned());
    println!(
        "has_virtualization_entitlement={:?}",
        info.has_virtualization_entitlement()
    );
    write_runtime_capabilities(
        std::io::stdout(),
        firkin::apple_local_runtime_capabilities(),
    )?;
    Ok(())
}

fn write_runtime_capabilities(
    mut writer: impl Write,
    capabilities: firkin::RuntimeCapabilities,
) -> std::io::Result<()> {
    writeln!(writer, "runtime_backend={}", capabilities.backend())?;
    for capability in capabilities.supported() {
        writeln!(writer, "supported_capability={}", capability.name())?;
    }
    for capability in capabilities.unsupported() {
        writeln!(
            writer,
            "unsupported_capability={} reason={}",
            capability.name(),
            capability.reason().unwrap_or("")
        )?;
    }
    Ok(())
}

fn write_benchmark_catalog(mut writer: impl Write) -> std::io::Result<()> {
    let catalog = firkin::evidence::benchmark_metric_catalog();
    writeln!(
        writer,
        "manifest=firkin-benchmark-metric-catalog-v1 metrics={} p0_metrics={} autoscale_metrics={}",
        catalog.len(),
        firkin::evidence::P0_SCORECARD_METRICS.len(),
        firkin::evidence::AUTOSCALE_EFFICIENCY_SCORECARD_METRICS.len()
    )?;
    for metric in catalog {
        writeln!(
            writer,
            "metric={} group={} kind={:?} unit={} requirement={} notes={}",
            metric.name,
            metric.group.as_str(),
            metric.kind,
            benchmark_unit_label(metric.unit),
            metric.requirement.as_str(),
            metric.notes
        )?;
    }
    Ok(())
}

fn write_benchmark_p0_contract(mut writer: impl Write) -> Result<(), Box<dyn Error>> {
    let suite = firkin::benchmark::benchmark_suite("agent-core")
        .ok_or_else(|| "missing benchmark suite `agent-core`".to_owned())?;
    let coverage = firkin::evidence::p0_scorecard_measurement_coverage()
        .iter()
        .map(|metric| (metric.metric, metric))
        .collect::<BTreeMap<_, _>>();
    writeln!(
        writer,
        "manifest=firkin-benchmark-p0-contract-v1 suite={} p0_metrics={}",
        suite.id,
        firkin::evidence::P0_SCORECARD_METRICS.len()
    )?;
    for metric in firkin::evidence::P0_SCORECARD_METRICS {
        let definition = firkin::evidence::benchmark_metric_definition(metric)
            .ok_or_else(|| format!("P0 metric `{metric}` missing catalog definition"))?;
        let case = suite
            .cases
            .iter()
            .find(|case| case.metric == *metric)
            .ok_or_else(|| format!("P0 metric `{metric}` missing agent-core suite case"))?;
        let coverage = coverage
            .get(metric)
            .ok_or_else(|| format!("P0 metric `{metric}` missing measurement coverage"))?;
        writeln!(
            writer,
            "metric={} group={} kind={:?} unit={} case={} source={} status={}",
            definition.name,
            definition.group.as_str(),
            definition.kind,
            benchmark_unit_label(definition.unit),
            case.id,
            coverage.source,
            coverage.status.as_str()
        )?;
    }
    Ok(())
}

fn write_benchmark_autoscale_contract(mut writer: impl Write) -> Result<(), Box<dyn Error>> {
    let coverage = firkin::evidence::autoscale_efficiency_measurement_coverage()
        .iter()
        .map(|metric| (metric.metric, metric))
        .collect::<BTreeMap<_, _>>();
    writeln!(
        writer,
        "manifest=firkin-benchmark-autoscale-contract-v1 autoscale_metrics={}",
        firkin::evidence::AUTOSCALE_EFFICIENCY_SCORECARD_METRICS.len()
    )?;
    for metric in firkin::evidence::AUTOSCALE_EFFICIENCY_SCORECARD_METRICS {
        let definition = firkin::evidence::benchmark_metric_definition(metric)
            .ok_or_else(|| format!("autoscale metric `{metric}` missing catalog definition"))?;
        let coverage = coverage
            .get(metric)
            .ok_or_else(|| format!("autoscale metric `{metric}` missing measurement coverage"))?;
        let ownership = firkin::evidence::benchmark_metric_ownership(metric);
        writeln!(
            writer,
            "metric={} group={} kind={:?} unit={} requirement={} source={} status={} owner={} phase={}",
            definition.name,
            definition.group.as_str(),
            definition.kind,
            benchmark_unit_label(definition.unit),
            definition.requirement.as_str(),
            coverage.source,
            coverage.status.as_str(),
            ownership.owner,
            ownership.phase_label
        )?;
    }
    Ok(())
}

fn write_benchmark_metric_contract(mut writer: impl Write) -> std::io::Result<()> {
    let contract = firkin::evidence::decision_grade_metric_contract();
    writeln!(
        writer,
        "manifest=firkin-decision-grade-metric-contract-v1 metrics={}",
        contract.len()
    )?;
    for metric in contract {
        let policy = metric.percentile_policy();
        writeln!(
            writer,
            "metric={} start={} end={} lifecycle={} workload={} profile={} owner={} p95_min_samples={} p99_min_samples={} level={} included=\"{}\" excluded=\"{}\"",
            metric.metric(),
            metric.start_event().as_str(),
            metric.end_event().as_str(),
            metric.lifecycle().as_str(),
            metric.workload().as_str(),
            metric.profile().as_str(),
            metric.owner(),
            policy.p95_min_samples(),
            policy.p99_min_samples(),
            metric.level().as_str(),
            metric.included_phases(),
            metric.excluded_phases()
        )?;
    }
    Ok(())
}

fn write_benchmark_phase_owners(mut writer: impl Write) -> std::io::Result<()> {
    let rules = firkin::evidence::benchmark_metric_ownership_table();
    writeln!(
        writer,
        "manifest=firkin-benchmark-phase-owners-v1 rules={}",
        rules.len()
    )?;
    for rule in rules {
        writeln!(
            writer,
            "match={} phase={} owner={} next_action={}",
            rule.metric_match.as_str(),
            rule.phase_label,
            rule.owner,
            rule.next_action_hint
        )?;
    }
    Ok(())
}

fn write_benchmark_memory_attribution(mut writer: impl Write) -> std::io::Result<()> {
    let blocker = firkin::benchmark::P0_MEMORY_ATTRIBUTION_BLOCKER;
    writeln!(
        writer,
        "manifest=firkin-benchmark-memory-attribution-v1 status=blocked required_collector={} current_source={} exact_vm_scoped={}",
        firkin::benchmark::EXACT_MEMORY_ATTRIBUTION_COLLECTOR_REQUIREMENT,
        blocker.attribution_source,
        blocker.is_exact_vm_scoped()
    )?;
    writeln!(writer, "blocker={}", blocker.blocker)?;
    writeln!(writer, "next_spike={}", blocker.next_spike)?;
    for metric in [
        "sandbox.mem.idle_host_footprint_bytes",
        "sandbox.mem.post_task_residual_bytes",
        "sandbox.mem.reclaim_effectiveness_ratio",
    ] {
        writeln!(
            writer,
            "metric={} promotion=requires:{}",
            metric,
            firkin::benchmark::EXACT_MEMORY_ATTRIBUTION_COLLECTOR_REQUIREMENT
        )?;
    }
    Ok(())
}

fn write_benchmark_coverage(
    args: &BenchmarkCoverageArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let coverage = firkin::evidence::p0_scorecard_measurement_coverage();
    let artifacts = args
        .artifact
        .iter()
        .map(|path| load_benchmark_summaries(path))
        .collect::<Result<Vec<_>, _>>()?;
    let artifact_paths = if args.artifact.is_empty() {
        "-".to_owned()
    } else {
        args.artifact
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    let artifact_kinds = if artifacts.is_empty() {
        "-".to_owned()
    } else {
        artifacts
            .iter()
            .map(|artifact| artifact.kind)
            .collect::<Vec<_>>()
            .join(",")
    };
    writeln!(
        writer,
        "manifest=firkin-benchmark-measurement-coverage-v1 p0_metrics={} strict={} artifact={} artifact_kind={}",
        coverage.len(),
        args.strict,
        artifact_paths,
        artifact_kinds
    )?;
    let mut strict_failures = Vec::new();
    for metric in coverage {
        let artifact_metric = coverage_artifact_metric(metric, &artifacts);
        let artifact_present = artifact_metric.present;
        let reported_status = coverage_reported_status(metric, &artifact_metric);
        let p95_min_samples = p95_min_samples_for_metric(metric.metric);
        let p99_min_samples = p99_min_samples_for_metric(metric.metric);
        let confidence =
            coverage_confidence_label(artifact_present, metric.metric, artifact_metric.count);
        let p95_status =
            coverage_p95_status(artifact_present, artifact_metric.count, p95_min_samples);
        let p99_status =
            coverage_p99_status(artifact_present, artifact_metric.count, p99_min_samples);
        if args.strict
            && (reported_status != firkin::evidence::BenchmarkMeasurementStatus::SignedLiveExact
                || !artifact_present
                || p95_status != "decision_grade")
        {
            strict_failures.push(metric.metric);
        }
        let artifact_metric_name = artifact_metric.source_metric;
        let artifact_status = if artifact_metric.present {
            "present"
        } else {
            "missing"
        };
        let artifact_count = artifact_metric.count;
        let artifact_kind = artifact_metric.artifact_kind;
        writeln!(
            writer,
            "metric={} status={} source={} artifact_metric={} artifact_status={} artifact_count={} artifact_kind={} confidence={} p95_min_samples={} p95_status={} p99_min_samples={} p99_status={} notes={}",
            metric.metric,
            reported_status.as_str(),
            metric.source,
            artifact_metric_name,
            artifact_status,
            artifact_count,
            artifact_kind,
            confidence,
            p95_min_samples,
            p95_status,
            p99_min_samples,
            p99_status,
            metric.notes
        )?;
    }
    if args.strict && !strict_failures.is_empty() {
        return Err(format!(
            "strict benchmark coverage failed for {} P0 metrics: {}",
            strict_failures.len(),
            strict_failures.join(",")
        )
        .into());
    }
    Ok(())
}

fn coverage_reported_status(
    metric: &firkin::evidence::BenchmarkMeasurementCoverage,
    artifact_metric: &CoverageArtifactMetric,
) -> firkin::evidence::BenchmarkMeasurementStatus {
    let exact_live_artifact_present = metric.status
        == firkin::evidence::BenchmarkMeasurementStatus::NeedsLiveHarness
        && artifact_metric.present
        && (metric.metric == IO_FULL_AVG10_METRIC
            || (P0_MEMORY_METRICS.contains(&metric.metric)
                && artifact_metric.artifact_kind == "overhead"));
    if exact_live_artifact_present {
        firkin::evidence::BenchmarkMeasurementStatus::SignedLiveExact
    } else {
        metric.status
    }
}

struct CoverageArtifactMetric {
    source_metric: &'static str,
    present: bool,
    count: usize,
    artifact_kind: &'static str,
}

fn coverage_artifact_metric(
    metric: &firkin::evidence::BenchmarkMeasurementCoverage,
    artifacts: &[LoadedBenchmarkSummaries],
) -> CoverageArtifactMetric {
    let (source_kind, source_metric) = metric
        .source
        .split_once(':')
        .map_or((None, metric.metric), |(source, source_metric)| {
            (coverage_source_artifact_kind(source), source_metric)
        });
    let mut best = CoverageArtifactMetric {
        source_metric,
        present: false,
        count: 0,
        artifact_kind: "-",
    };
    for artifact in artifacts {
        if source_kind.is_some_and(|kind| artifact.kind != kind) {
            continue;
        }
        let count = artifact
            .summaries
            .iter()
            .find(|summary| summary.metric() == source_metric)
            .map_or(0, firkin::evidence::BenchmarkSummary::count);
        if count > best.count {
            best.count = count;
            best.present = count > 0;
            best.artifact_kind = artifact.kind;
        }
    }
    best
}

fn coverage_source_artifact_kind(source: &str) -> Option<&'static str> {
    match source {
        "live_runtime_benchmark_evidence" => Some("lifecycle"),
        "live_runtime_overhead_evidence" => Some("overhead"),
        _ => None,
    }
}

fn coverage_confidence_label(present: bool, metric: &str, count: usize) -> &'static str {
    if present {
        percentile_availability_for_metric(metric, count).as_str()
    } else {
        "missing"
    }
}

fn coverage_p95_status(present: bool, count: usize, min_samples: usize) -> &'static str {
    if !present {
        "missing"
    } else if count >= min_samples {
        "decision_grade"
    } else {
        "collect_more_samples"
    }
}

fn coverage_p99_status(present: bool, count: usize, min_samples: usize) -> &'static str {
    if !present {
        "missing"
    } else if count >= min_samples {
        "decision_grade"
    } else {
        "experimental"
    }
}

fn percentile_availability_for_metric(
    metric: &str,
    count: usize,
) -> firkin::evidence::PercentileAvailability {
    firkin::evidence::decision_grade_metric_contract()
        .iter()
        .find(|contract| contract.metric() == metric)
        .map_or_else(
            || firkin::evidence::PercentileAvailability::for_sample_count(count),
            |contract| {
                let policy = contract.percentile_policy();
                firkin::evidence::PercentileAvailability::for_sample_policy(
                    count,
                    policy.p95_min_samples(),
                    policy.p99_min_samples(),
                )
            },
        )
}

fn p95_min_samples_for_metric(metric: &str) -> usize {
    firkin::evidence::decision_grade_metric_contract()
        .iter()
        .find(|contract| contract.metric() == metric)
        .map_or(100, |contract| {
            contract.percentile_policy().p95_min_samples()
        })
}

fn p99_min_samples_for_metric(metric: &str) -> usize {
    firkin::evidence::decision_grade_metric_contract()
        .iter()
        .find(|contract| contract.metric() == metric)
        .map_or(500, |contract| {
            contract.percentile_policy().p99_min_samples()
        })
}

fn write_benchmark_doctor(
    args: &BenchmarkDoctorArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let storage = storage_config_from_roots(args.state_root.clone(), args.cache_root.clone());
    let benchmark_root = args
        .benchmark_root
        .clone()
        .unwrap_or_else(firkin_benchmark_root);
    let mut failures = Vec::new();

    writeln!(
        writer,
        "benchmark_doctor=started mode={} min_free_bytes={}",
        args.mode.as_str(),
        args.min_free_bytes
    )?;
    write_storage_preflight(&storage, &benchmark_root, &mut writer, &mut failures)?;
    write_disk_preflight(
        &benchmark_root,
        args.min_free_bytes,
        &mut writer,
        &mut failures,
    )?;
    if args.mode == BenchmarkMode::SignedLive {
        write_signed_live_preflight(&mut writer, &mut failures)?;
    }
    if failures.is_empty() {
        writeln!(
            writer,
            "benchmark_doctor=passed mode={}",
            args.mode.as_str()
        )?;
        Ok(())
    } else {
        writeln!(
            writer,
            "benchmark_doctor=failed mode={} failures={}",
            args.mode.as_str(),
            failures.len()
        )?;
        for failure in &failures {
            writeln!(writer, "failure={failure}")?;
        }
        Err(format!("benchmark doctor failed: {}", failures.join(", ")).into())
    }
}

fn write_storage_preflight(
    storage: &firkin_runtime::FirkinStorageConfig,
    benchmark_root: &Path,
    writer: &mut impl Write,
    failures: &mut Vec<String>,
) -> std::io::Result<()> {
    for (label, path) in [
        ("state_root", storage.state_root()),
        ("cache_root", storage.cache_root()),
        ("benchmark_root", benchmark_root),
    ] {
        let writable = ensure_writable_dir(path).is_ok();
        if !writable {
            failures.push(format!("{label}_not_writable"));
        }
        writeln!(
            writer,
            "check={} path={} writable={}",
            label,
            path.display(),
            writable
        )?;
    }
    Ok(())
}

fn ensure_writable_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    let probe_nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let probe = path.join(format!(
        ".firkin-write-test-{}-{probe_nonce}",
        std::process::id()
    ));
    std::fs::write(&probe, b"ok")?;
    std::fs::remove_file(probe)
}

fn write_disk_preflight(
    path: &Path,
    min_free_bytes: u64,
    writer: &mut impl Write,
    failures: &mut Vec<String>,
) -> std::io::Result<()> {
    let free_bytes = disk_free_bytes(path).unwrap_or(0);
    if free_bytes < min_free_bytes {
        failures.push("disk_free_below_floor".to_owned());
    }
    writeln!(
        writer,
        "check=disk_free path={} free_bytes={} min_free_bytes={} ok={}",
        path.display(),
        free_bytes,
        min_free_bytes,
        free_bytes >= min_free_bytes
    )
}

fn disk_free_bytes(path: &Path) -> std::io::Result<u64> {
    let output = ProcessCommand::new("df").arg("-Pk").arg(path).output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("df -Pk failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .nth(1)
        .ok_or_else(|| std::io::Error::other("df output missing data line"))?;
    let available_kib = line
        .split_whitespace()
        .nth(3)
        .ok_or_else(|| std::io::Error::other("df output missing available column"))?
        .parse::<u64>()
        .map_err(std::io::Error::other)?;
    Ok(available_kib.saturating_mul(1024))
}

fn write_signed_live_preflight(
    writer: &mut impl Write,
    failures: &mut Vec<String>,
) -> std::io::Result<()> {
    match firkin::vmm::preflight() {
        Ok(info) => {
            let arch_ok = matches!(info.architecture(), firkin::vmm::HostArch::Arm64);
            if !arch_ok {
                failures.push("host_not_arm64".to_owned());
            }
            writeln!(
                writer,
                "check=vz_host macos={} architecture={:?} nested_virtualization_supported={} rosetta_available={} ok={}",
                info.macos_version(),
                info.architecture(),
                info.nested_virtualization_supported(),
                info.rosetta_available(),
                arch_ok
            )?;
            writeln!(
                writer,
                "check=current_executable_signing codesigned={:?} virtualization_entitlement={:?}",
                info.codesigned(),
                info.has_virtualization_entitlement()
            )?;
        }
        Err(error) => {
            failures.push(format!("vz_preflight_error:{error}"));
        }
    }
    for (label, path) in [
        (
            "signed_live_script",
            Path::new("scripts/run-signed-live-runtime-test.sh"),
        ),
        ("vz_entitlements", Path::new("signing/vz.entitlements")),
    ] {
        let exists = path.exists();
        if !exists {
            failures.push(format!("{label}_missing"));
        }
        writeln!(
            writer,
            "check={} path={} exists={}",
            label,
            path.display(),
            exists
        )?;
    }
    write_guest_psi_preflight(writer, failures)?;
    writeln!(writer, "check=vminitd_bytes available=true")?;
    Ok(())
}

fn write_guest_psi_preflight(
    writer: &mut impl Write,
    failures: &mut Vec<String>,
) -> std::io::Result<()> {
    let config_path = Path::new("kernel/config-arm64");
    let kernel_path = Path::new("bin/vmlinux");
    let readiness = firkin::benchmark::GuestPsiReadiness::from_kernel_config_and_artifact(
        include_str!("../../../kernel/config-arm64"),
        config_path,
        kernel_path,
    );
    let missing = readiness.missing_prerequisite().unwrap_or("");
    if !readiness.signed_live_prerequisite_ready() {
        failures.push(format!("guest_psi_unavailable:{missing}"));
    }
    writeln!(
        writer,
        "check=guest_psi metric=sandbox.pressure.io_full_avg10 source_config_psi={} source_config_default_enabled={} kernel_artifact_current={} exact_ready={} prerequisite=\"{}\" missing=\"{}\"",
        readiness.kernel_config_psi(),
        readiness.kernel_config_default_enabled(),
        readiness.kernel_artifact_current().unwrap_or(false),
        readiness.signed_live_prerequisite_ready(),
        firkin::benchmark::GUEST_PSI_PREREQUISITE,
        missing
    )
}

fn write_benchmark_suites(
    args: &BenchmarkSuitesArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let suites = if let Some(suite) = &args.suite {
        vec![
            firkin::benchmark::benchmark_suite(suite)
                .ok_or_else(|| format!("unknown benchmark suite `{suite}`"))?,
        ]
    } else {
        firkin::benchmark::benchmark_suites()
            .iter()
            .collect::<Vec<_>>()
    };
    writeln!(
        writer,
        "manifest=firkin-benchmark-suites-v1 suites={}",
        suites.len()
    )?;
    for suite in suites {
        writeln!(
            writer,
            "suite={} title={} cases={} purpose={}",
            suite.id,
            suite.title,
            suite.cases.len(),
            suite.purpose
        )?;
        for case in suite.cases {
            writeln!(
                writer,
                "case={} suite={} metric={} execution={} notes={}",
                case.id,
                suite.id,
                case.metric,
                case.execution.as_str(),
                case.notes
            )?;
        }
    }
    Ok(())
}

fn write_benchmark_targets(mut writer: impl Write) -> std::io::Result<()> {
    writeln!(writer, "manifest=firkin-benchmark-targets-v1")?;
    writeln!(writer, "unit=milliseconds")?;
    for target in firkin::substrate::REQUIRED_LIFECYCLE_LATENCY_TARGETS {
        writeln!(
            writer,
            "target={} p50_ms={} p95_ms={} notes={}",
            target.name, target.p50_ms, target.p95_ms, target.notes
        )?;
    }
    for target in firkin::substrate::REQUIRED_FIRKIN_OVERHEAD_METRICS {
        writeln!(
            writer,
            "overhead={} p95_{}={} notes={}",
            target.name,
            benchmark_unit_label(target.unit),
            target.max_p95,
            target.notes
        )?;
    }
    Ok(())
}

fn benchmark_unit_label(unit: firkin::trace::BenchmarkUnit) -> &'static str {
    match unit {
        firkin::trace::BenchmarkUnit::Milliseconds => "ms",
        firkin::trace::BenchmarkUnit::Microseconds => "us",
        firkin::trace::BenchmarkUnit::Percent => "percent",
        firkin::trace::BenchmarkUnit::Mebibytes => "mib",
        firkin::trace::BenchmarkUnit::Hertz => "hz",
        firkin::trace::BenchmarkUnit::Bytes => "bytes",
        firkin::trace::BenchmarkUnit::Count => "count",
        firkin::trace::BenchmarkUnit::CountPerSecond => "count_per_sec",
        firkin::trace::BenchmarkUnit::OperationsPerSecond => "ops_per_sec",
        firkin::trace::BenchmarkUnit::BytesPerSecond => "bytes_per_sec",
        firkin::trace::BenchmarkUnit::MebibytesPerSecond => "mib_per_sec",
        firkin::trace::BenchmarkUnit::Iops => "iops",
        firkin::trace::BenchmarkUnit::Ratio => "ratio",
    }
}

fn write_substrate_acceptance_checklist(mut writer: impl Write) -> std::io::Result<()> {
    writeln!(
        writer,
        "manifest=production-apple-vz-substrate-acceptance-v1"
    )?;
    for check in ACCEPTANCE_CHECKS {
        writeln!(
            writer,
            "check={} status={} evidence={} notes={}",
            check.id, check.status, check.evidence, check.notes
        )?;
    }
    Ok(())
}

fn validate_lifecycle_slo(
    args: &ValidateLifecycleSloArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let report = firkin::evidence::BenchmarkEvidenceArtifact::read_json(&args.artifact)?;
    let gate = firkin::evidence::BenchmarkSloGateReport::from_lifecycle_report(
        &report,
        firkin::evidence::default_lifecycle_latency_slo_targets(args.min_samples),
    )?;
    writeln!(
        writer,
        "benchmark_slo_gate=passed kind=lifecycle artifact={} targets={} min_samples={}",
        args.artifact.display(),
        gate.passed_targets().len(),
        args.min_samples
    )?;
    Ok(())
}

fn validate_overhead_slo(
    args: &ValidateOverheadSloArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let report = firkin::evidence::BenchmarkOverheadEvidenceArtifact::read_json(&args.artifact)?;
    let gate = firkin::evidence::BenchmarkSloGateReport::from_overhead_report(
        &report,
        firkin::evidence::default_firkin_overhead_slo_targets(args.min_samples),
    )?;
    writeln!(
        writer,
        "benchmark_slo_gate=passed kind=overhead artifact={} targets={} min_samples={}",
        args.artifact.display(),
        gate.passed_targets().len(),
        args.min_samples
    )?;
    Ok(())
}

fn write_scorecard_artifact(
    args: &WriteScorecardArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let samples = read_benchmark_samples(&args.samples)?;
    let report = firkin::benchmark::RuntimeAgentScorecardEvidenceWriter::new(&args.artifact)
        .with_min_samples(args.min_samples)
        .write_samples(samples)?;
    writeln!(
        writer,
        "scorecard_artifact=written artifact={} required_metrics={} min_samples={}",
        args.artifact.display(),
        report.required_metrics().len(),
        args.min_samples
    )?;
    Ok(())
}

fn validate_scorecard(
    args: &ValidateScorecardArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let report = firkin::evidence::AgentBenchmarkScorecardArtifact::read_json(&args.artifact)?;
    report.validate_min_samples(args.min_samples)?;
    let snappy_target_misses = report.snappy_target_misses();
    writeln!(
        writer,
        "scorecard=valid artifact={} required_metrics={} min_samples={} snappy_target_status={} snappy_target_misses={} require_snappy={}",
        args.artifact.display(),
        report.required_metrics().len(),
        args.min_samples,
        snappy_target_status(&snappy_target_misses),
        snappy_target_misses.len(),
        args.require_snappy
    )?;
    write_snappy_target_misses(&snappy_target_misses, &mut writer)?;
    if args.require_snappy && !snappy_target_misses.is_empty() {
        return Err(format!(
            "agent scorecard is not snappy: {} target misses remain",
            snappy_target_misses.len()
        )
        .into());
    }
    Ok(())
}

fn write_autoscale_scorecard_artifact(
    args: &WriteAutoscaleScorecardArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let samples = read_benchmark_samples(&args.samples)?;
    let report = firkin::benchmark::RuntimeAutoscaleScorecardEvidenceWriter::new(&args.artifact)
        .with_min_samples(args.min_samples)
        .write_samples(samples)?;
    writeln!(
        writer,
        "autoscale_scorecard_artifact=written artifact={} required_metrics={} min_samples={}",
        args.artifact.display(),
        report.required_metrics().len(),
        args.min_samples
    )?;
    Ok(())
}

fn validate_autoscale_scorecard(
    args: &ValidateAutoscaleScorecardArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let report = firkin::evidence::AutoscaleEfficiencyScorecardArtifact::read_json(&args.artifact)?;
    report.validate_min_samples(args.min_samples)?;
    let snappy_target_misses = report.snappy_target_misses();
    writeln!(
        writer,
        "autoscale_scorecard=valid artifact={} required_metrics={} min_samples={} promotion_blockers={} require_promotable={} snappy_target_status={} snappy_target_misses={} require_snappy={}",
        args.artifact.display(),
        report.required_metrics().len(),
        args.min_samples,
        report.promotion_blockers().len(),
        args.require_promotable,
        snappy_target_status(&snappy_target_misses),
        snappy_target_misses.len(),
        args.require_snappy
    )?;
    write_autoscale_promotion_blockers(&report, &mut writer)?;
    write_snappy_target_misses(&snappy_target_misses, &mut writer)?;
    if args.require_promotable && !report.promotion_blockers().is_empty() {
        return Err(format!(
            "autoscale scorecard is not promotion-grade: {} promotion blockers remain",
            report.promotion_blockers().len()
        )
        .into());
    }
    if args.require_snappy && !snappy_target_misses.is_empty() {
        return Err(format!(
            "autoscale scorecard is not snappy: {} target misses remain",
            snappy_target_misses.len()
        )
        .into());
    }
    Ok(())
}

fn write_agent_computer_scorecard_artifact(
    args: &WriteAgentComputerScorecardArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let samples = read_benchmark_samples(&args.samples)?;
    let report =
        firkin::benchmark::RuntimeAgentComputerScorecardEvidenceWriter::new(&args.artifact)
            .with_min_samples(args.min_samples)
            .write_samples(samples)?;
    writeln!(
        writer,
        "agent_computer_scorecard_artifact=written artifact={} required_metrics={} min_samples={}",
        args.artifact.display(),
        report.required_metrics().len(),
        args.min_samples
    )?;
    Ok(())
}

fn validate_agent_computer_scorecard(
    args: &ValidateAgentComputerScorecardArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let report = firkin::evidence::AgentComputerScorecardArtifact::read_json(&args.artifact)?;
    report.validate_min_samples(args.min_samples)?;
    let snappy_target_misses = report.snappy_target_misses();
    writeln!(
        writer,
        "agent_computer_scorecard=valid artifact={} required_metrics={} min_samples={} promotion_blockers={} require_promotable={} snappy_target_status={} snappy_target_misses={} require_snappy={}",
        args.artifact.display(),
        report.required_metrics().len(),
        args.min_samples,
        report.promotion_blockers().len(),
        args.require_promotable,
        snappy_target_status(&snappy_target_misses),
        snappy_target_misses.len(),
        args.require_snappy
    )?;
    write_agent_computer_promotion_blockers(&report, &mut writer)?;
    write_snappy_target_misses(&snappy_target_misses, &mut writer)?;
    if args.require_promotable && !report.promotion_blockers().is_empty() {
        return Err(format!(
            "agent-computer scorecard is not promotion-grade: {} promotion blockers remain",
            report.promotion_blockers().len()
        )
        .into());
    }
    if args.require_snappy && !snappy_target_misses.is_empty() {
        return Err(format!(
            "agent-computer scorecard is not snappy: {} target misses remain",
            snappy_target_misses.len()
        )
        .into());
    }
    Ok(())
}

fn write_scorecard_report(
    args: &ReportScorecardArgs,
    writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let report = firkin::evidence::AgentBenchmarkScorecardArtifact::read_json(&args.artifact)?;
    report.validate_min_samples(1)?;
    write_scorecard_summaries(
        "scorecard_report",
        &args.artifact,
        report.summaries(),
        writer,
    )?;
    Ok(())
}

fn write_autoscale_scorecard_report(
    args: &ReportAutoscaleScorecardArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let report = firkin::evidence::AutoscaleEfficiencyScorecardArtifact::read_json(&args.artifact)?;
    report.validate_min_samples(1)?;
    write_scorecard_summaries(
        "autoscale_scorecard_report",
        &args.artifact,
        report.summaries(),
        &mut writer,
    )?;
    write_autoscale_promotion_blockers(&report, &mut writer)?;
    Ok(())
}

fn write_agent_computer_scorecard_report(
    args: &ReportAgentComputerScorecardArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let report = firkin::evidence::AgentComputerScorecardArtifact::read_json(&args.artifact)?;
    report.validate_min_samples(1)?;
    write_scorecard_summaries(
        "agent_computer_scorecard_report",
        &args.artifact,
        report.summaries(),
        &mut writer,
    )?;
    write_agent_computer_promotion_blockers(&report, &mut writer)?;
    Ok(())
}

fn write_agent_computer_trace_report(
    args: &ReportAgentComputerTracesArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let (artifact_kind, traces, explicit_samples) =
        read_agent_computer_trace_artifact(&args.artifact)?;
    let overflowed = traces
        .iter()
        .map(firkin::trace::SandboxEventTrace::overflowed)
        .sum::<u64>();
    let derived_samples =
        firkin::evidence::derive_available_product_autoscale_metric_samples(traces.clone());
    let explicit_sample_count = explicit_samples.len();
    let summaries = benchmark_summaries_from_samples(derived_samples)?;
    let explicit_summaries = benchmark_summaries_from_samples(explicit_samples.clone())?;
    writeln!(
        writer,
        "agent_computer_trace_report=summary artifact={} kind={} traces={} overflowed={} derived_metrics={} explicit_metrics={} explicit_samples={}",
        args.artifact.display(),
        artifact_kind.as_deref().unwrap_or("raw_trace_array"),
        traces.len(),
        overflowed,
        summaries.len(),
        explicit_summaries.len(),
        explicit_sample_count
    )?;
    write_scorecard_summaries(
        "agent_computer_trace_metric",
        &args.artifact,
        &summaries,
        &mut writer,
    )?;
    if !explicit_summaries.is_empty() {
        write_scorecard_summaries(
            "agent_computer_artifact_metric",
            &args.artifact,
            &explicit_summaries,
            &mut writer,
        )?;
    }
    for sample in &explicit_samples {
        writeln!(
            writer,
            "agent_computer_artifact_sample=tags artifact={} metric={} tags={}",
            args.artifact.display(),
            sample.metric(),
            sample_tags_report_value(sample)
        )?;
    }
    for (index, trace) in traces.iter().enumerate() {
        let Some(first) = trace.events().first() else {
            writeln!(
                writer,
                "trace={} events=0 overflowed={}",
                index,
                trace.overflowed()
            )?;
            continue;
        };
        let start = match first.lifecycle() {
            firkin::trace::LifecycleClass::Resumed => {
                firkin::trace::SandboxEventName::AgentComputerResumed
            }
            _ => firkin::trace::SandboxEventName::AgentComputerRequestStart,
        };
        let metric = agent_computer_trace_line_metric(first.lifecycle(), first.workload());
        writeln!(
            writer,
            "trace={} metric={} lifecycle={:?} workload={:?} profile={:?} events={} overflowed={} total_ms={} create_ms={} probe_ms={} cli_ms={} browser_ms={} database_ms={}",
            index,
            metric,
            first.lifecycle(),
            first.workload(),
            first.profile(),
            trace.events().len(),
            trace.overflowed(),
            format_trace_duration_ms(
                trace,
                start,
                firkin::trace::SandboxEventName::AgentComputerReady,
            ),
            format_trace_duration_ms(
                trace,
                start,
                firkin::trace::SandboxEventName::AgentComputerSandboxCreated,
            ),
            format_trace_duration_ms(
                trace,
                firkin::trace::SandboxEventName::AgentComputerProbeStart,
                firkin::trace::SandboxEventName::AgentComputerReady,
            ),
            format_trace_duration_ms(
                trace,
                start,
                firkin::trace::SandboxEventName::CliFirstUsefulStdout,
            ),
            format_trace_duration_ms(trace, start, firkin::trace::SandboxEventName::BrowserReady,),
            format_trace_duration_ms(trace, start, firkin::trace::SandboxEventName::DatabaseReady,)
        )?;
    }
    Ok(())
}

fn agent_computer_trace_line_metric(
    lifecycle: firkin::trace::LifecycleClass,
    workload: firkin::trace::WorkloadClass,
) -> &'static str {
    match (lifecycle, workload) {
        (firkin::trace::LifecycleClass::Resumed, firkin::trace::WorkloadClass::AgentComputer) => {
            "product.agent_computer_resume_ms"
        }
        (_, firkin::trace::WorkloadClass::AgentComputer) => "product.agent_computer_ready_ms",
        (_, firkin::trace::WorkloadClass::ConcurrentCreate) => {
            "debug.product.agent_computer_density_trace_ms"
        }
        _ => "debug.product.agent_computer_trace_ms",
    }
}

fn benchmark_summaries_from_samples(
    samples: Vec<firkin::trace::BenchmarkSample>,
) -> Result<Vec<firkin::evidence::BenchmarkSummary>, firkin::evidence::BenchmarkSummaryError> {
    let mut grouped = BTreeMap::<String, Vec<firkin::trace::BenchmarkSample>>::new();
    for sample in samples {
        grouped
            .entry(sample.metric().to_owned())
            .or_default()
            .push(sample);
    }
    grouped
        .into_iter()
        .map(|(metric, samples)| firkin::evidence::BenchmarkSummary::from_samples(metric, samples))
        .collect()
}

fn format_trace_duration_ms(
    trace: &firkin::trace::SandboxEventTrace,
    start: firkin::trace::SandboxEventName,
    end: firkin::trace::SandboxEventName,
) -> String {
    trace.duration_between(start, end).map_or_else(
        |_| "missing".to_owned(),
        |duration| (duration.as_secs_f64() * 1000.0).to_string(),
    )
}

fn write_agent_computer_promotion_blockers(
    report: &firkin::evidence::AgentComputerScorecardReport,
    mut writer: impl Write,
) -> std::io::Result<()> {
    for blocker in report.promotion_blockers() {
        writeln!(
            writer,
            "promotion_blocker metric={} blocker={} next_action={}",
            blocker.metric(),
            blocker.blocker(),
            blocker.next_action()
        )?;
    }
    Ok(())
}

fn write_autoscale_promotion_blockers(
    report: &firkin::evidence::AutoscaleEfficiencyScorecardReport,
    mut writer: impl Write,
) -> std::io::Result<()> {
    for blocker in report.promotion_blockers() {
        writeln!(
            writer,
            "promotion_blocker metric={} blocker={} next_action={}",
            blocker.metric(),
            blocker.blocker(),
            blocker.next_action()
        )?;
    }
    Ok(())
}

fn write_snappy_target_misses(
    misses: &[firkin::evidence::ScorecardSnappyTargetMiss],
    mut writer: impl Write,
) -> std::io::Result<()> {
    for miss in misses {
        writeln!(
            writer,
            "snappy_target_miss metric={} direction={} p95_threshold={} actual_p95={}",
            miss.metric(),
            miss.direction().as_str(),
            miss.p95_threshold(),
            miss.actual_p95()
        )?;
    }
    Ok(())
}

fn snappy_target_status(misses: &[firkin::evidence::ScorecardSnappyTargetMiss]) -> &'static str {
    if misses.is_empty() { "pass" } else { "miss" }
}

fn write_scorecard_summaries(
    report_label: &str,
    artifact: &Path,
    summaries: &[firkin::evidence::BenchmarkSummary],
    mut writer: impl Write,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "{report_label}=summary artifact={} metrics={}",
        artifact.display(),
        summaries.len()
    )?;
    for summary in summaries {
        let confidence = summary.percentile_availability();
        writeln!(
            writer,
            "metric={} kind={:?} unit={} count={} p50={} p90={} p95={} p99={} max={} confidence={} unstable_percentile={} p95_status={} p99_status={}",
            summary.metric(),
            summary.kind(),
            benchmark_unit_label(summary.unit()),
            summary.count(),
            summary.p50(),
            summary.p90(),
            summary.p95(),
            summary.p99(),
            summary.max(),
            confidence.as_str(),
            confidence.unstable_percentile(),
            confidence.p95_status(),
            confidence.p99_status()
        )?;
    }
    Ok(())
}

fn sample_tags_report_value(sample: &firkin::trace::BenchmarkSample) -> String {
    sample
        .tags()
        .iter()
        .map(|tag| format!("{}={}", tag.key(), tag.value()))
        .collect::<Vec<_>>()
        .join(",")
}

fn read_benchmark_samples(
    path: &Path,
) -> Result<Vec<firkin::trace::BenchmarkSample>, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

type AgentComputerTraceArtifact = (
    Option<String>,
    Vec<firkin::trace::SandboxEventTrace>,
    Vec<firkin::trace::BenchmarkSample>,
);

fn read_agent_computer_trace_artifact(
    path: &Path,
) -> Result<AgentComputerTraceArtifact, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    let value = serde_json::from_slice::<serde_json::Value>(&bytes)?;
    if value.is_array() {
        return Ok((None, serde_json::from_value(value)?, Vec::new()));
    }
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if let Some(traces) = value.get("traces") {
        let samples = value
            .get("samples")
            .map(|samples| serde_json::from_value(samples.clone()))
            .transpose()?
            .unwrap_or_default();
        return Ok((kind, serde_json::from_value(traces.clone())?, samples));
    }
    Err(format!(
        "agent-computer trace report expected a trace array or artifact with `traces`: {}",
        path.display()
    )
    .into())
}

fn write_benchmark_report(
    args: &BenchmarkReportArgs,
    writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    match args.kind {
        BenchmarkReportKind::Lifecycle => {
            if let Ok(report) =
                firkin::evidence::BenchmarkEvidenceArtifact::read_json(&args.artifact)
            {
                write_benchmark_summaries(args, "lifecycle", report.summaries(), writer)?;
            } else {
                let artifact = load_raw_sample_artifact_summaries(&args.artifact)?;
                if artifact.summaries.iter().any(|summary| {
                    summary.kind() != firkin::trace::BenchmarkMetricKind::LifecycleLatency
                }) {
                    return Err(format!(
                        "lifecycle report expected lifecycle-latency samples in {}",
                        args.artifact.display()
                    )
                    .into());
                }
                write_benchmark_summaries(args, artifact.kind, &artifact.summaries, writer)?;
            }
        }
        BenchmarkReportKind::Overhead => {
            let report =
                firkin::evidence::BenchmarkOverheadEvidenceArtifact::read_json(&args.artifact)?;
            write_benchmark_summaries(args, "overhead", report.summaries(), writer)?;
        }
        BenchmarkReportKind::Decision => {
            let artifact = load_benchmark_summaries(&args.artifact)?;
            write_decision_benchmark_report(args, &artifact, writer)?;
        }
    }
    Ok(())
}

fn write_benchmark_summaries(
    args: &BenchmarkReportArgs,
    kind: &str,
    summaries: &[firkin::evidence::BenchmarkSummary],
    mut writer: impl Write,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "benchmark_report=summary kind={} artifact={} metrics={}",
        kind,
        args.artifact.display(),
        summaries.len()
    )?;
    for summary in summaries {
        writeln!(
            writer,
            "metric={} kind={:?} unit={} count={} p50={} p90={} p95={} p99={} max={}",
            summary.metric(),
            summary.kind(),
            benchmark_unit_label(summary.unit()),
            summary.count(),
            summary.p50(),
            summary.p90(),
            summary.p95(),
            summary.p99(),
            summary.max()
        )?;
    }
    Ok(())
}

fn write_decision_benchmark_report(
    args: &BenchmarkReportArgs,
    artifact: &LoadedBenchmarkSummaries,
    mut writer: impl Write,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "benchmark_report=decision artifact={} artifact_kind={} metrics={}",
        args.artifact.display(),
        artifact.kind,
        artifact.summaries.len()
    )?;
    for summary in &artifact.summaries {
        let confidence = summary.percentile_availability();
        writeln!(
            writer,
            "metric={} kind={:?} unit={} count={} min={} mean={} mad={} cv={} p50={} p90={} p95={} p99={} max={} confidence={} unstable_percentile={} p95_status={} p99_status={}",
            summary.metric(),
            summary.kind(),
            benchmark_unit_label(summary.unit()),
            summary.count(),
            summary.min(),
            summary.mean(),
            summary.median_absolute_deviation(),
            summary.coefficient_of_variation(),
            summary.p50(),
            summary.p90(),
            summary.p95(),
            summary.p99(),
            summary.max(),
            confidence.as_str(),
            confidence.unstable_percentile(),
            confidence.p95_status(),
            confidence.p99_status(),
        )?;
    }
    Ok(())
}

fn run_benchmark_suite(
    args: &BenchmarkRunArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    if args.mode != BenchmarkMode::SignedLive {
        return Err(
            "benchmark run currently requires --mode signed-live for evidence generation".into(),
        );
    }
    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let repeats = benchmark_repeats_for_duration(args.duration);
    let mut command = ProcessCommand::new("scripts/run-signed-live-runtime-test.sh");
    if args.no_build {
        command.arg("--no-build");
    }
    if current_executable_is_release() {
        command.arg("--release");
    }
    configure_signed_live_benchmark_command(args, repeats, &mut command)?;
    let status = command.status()?;
    if !status.success() {
        return Err(format!("signed-live benchmark runner exited with {status}").into());
    }
    if !args.out.exists() {
        return Err(format!("benchmark runner did not write {}", args.out.display()).into());
    }
    writeln!(
        writer,
        "benchmark_run=passed suite={} mode={} duration_seconds={} repeats={} artifact={}",
        args.suite,
        args.mode.as_str(),
        args.duration.as_secs(),
        repeats,
        args.out.display()
    )?;
    Ok(())
}

fn configure_signed_live_benchmark_command(
    args: &BenchmarkRunArgs,
    repeats: u64,
    command: &mut ProcessCommand,
) -> Result<(), Box<dyn Error>> {
    match args.suite.as_str() {
        "agent-core" | "p0" | "overnight" => {
            let restore_timing = args.out.with_file_name("restore-timings.json");
            command
                .env("FIRKIN_LIVE_BENCHMARK_REPEATS", repeats.to_string())
                .env("FIRKIN_LIVE_BENCHMARK_ARTIFACT", &args.out)
                .env("FIRKIN_LIVE_RESTORE_TIMING_ARTIFACT", restore_timing)
                .arg("live_runtime_benchmark_evidence_writes_required_lifecycle_artifact");
        }
        "overhead" => {
            command
                .env("FIRKIN_LIVE_OVERHEAD_REPEATS", repeats.to_string())
                .env("FIRKIN_LIVE_OVERHEAD_ARTIFACT", &args.out)
                .arg("live_runtime_overhead_evidence_writes_required_overhead_artifact");
        }
        "agent-computer" => {
            command
                .env("FIRKIN_LIVE_AGENT_COMPUTER_REPEATS", repeats.to_string())
                .env("FIRKIN_LIVE_AGENT_COMPUTER_ARTIFACT", &args.out)
                .arg("live_runtime_agent_computer_scorecard_writes_product_path_artifact");
        }
        "autoscale" => {
            command
                .env("FIRKIN_LIVE_AUTOSCALE_REPEATS", repeats.to_string())
                .env("FIRKIN_LIVE_AUTOSCALE_ARTIFACT", &args.out)
                .arg("live_runtime_autoscale_scorecard_writes_product_path_artifact");
        }
        suite => {
            return Err(format!(
                "unsupported signed-live benchmark suite `{suite}`; expected agent-core, p0, overnight, overhead, agent-computer, or autoscale"
            )
            .into());
        }
    }
    Ok(())
}

fn benchmark_repeats_for_duration(duration: BenchmarkDuration) -> u64 {
    duration.as_secs().div_ceil(20).max(1)
}

fn current_executable_is_release() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.components()
                .rev()
                .nth(1)
                .map(|component| component.as_os_str() == "release")
        })
        .unwrap_or(false)
}

fn save_benchmark_baseline(
    args: &BenchmarkBaselineSaveArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let root = args
        .benchmark_root
        .clone()
        .unwrap_or_else(firkin_benchmark_root);
    let name = validate_baseline_name(&args.name)?;
    let destination = baseline_path(&root, name);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&args.artifact, &destination)?;
    writeln!(
        writer,
        "benchmark_baseline=saved name={} source={} path={}",
        name,
        args.artifact.display(),
        destination.display()
    )?;
    Ok(())
}

fn list_benchmark_baselines(
    args: &BenchmarkBaselineListArgs,
    mut writer: impl Write,
) -> std::io::Result<()> {
    let root = args
        .benchmark_root
        .clone()
        .unwrap_or_else(firkin_benchmark_root);
    let baseline_root = root.join("baselines");
    writeln!(
        writer,
        "benchmark_baselines=root path={}",
        baseline_root.display()
    )?;
    if !baseline_root.exists() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(&baseline_root)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        let size = std::fs::metadata(&path).map_or(0, |metadata| metadata.len());
        writeln!(
            writer,
            "baseline={} path={} bytes={}",
            path.file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("-"),
            path.display(),
            size
        )?;
    }
    Ok(())
}

fn validate_baseline_name(name: &str) -> Result<&str, Box<dyn Error>> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(
            format!("invalid baseline name `{name}`; use ASCII letters, numbers, - or _").into(),
        );
    }
    Ok(name)
}

fn baseline_path(root: &Path, name: &str) -> PathBuf {
    root.join("baselines").join(format!("{name}.json"))
}

fn compare_benchmark_artifacts(
    args: &BenchmarkCompareArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let (baseline_kind, current_kind, rows) =
        compare_benchmark_rows(&args.baseline, &args.current, args.rank)?;
    writeln!(
        writer,
        "benchmark_compare=summary rank={} baseline={} current={} baseline_kind={} current_kind={} matched_metrics={}",
        args.rank.as_str(),
        args.baseline.display(),
        args.current.display(),
        baseline_kind,
        current_kind,
        rows.len()
    )?;
    for row in rows.iter().take(10) {
        writeln!(
            writer,
            "metric={} phase={} owner={} next_action={} count={} baseline_count={} current_count={} baseline_p95={} current_p95={} delta_p95={} baseline_p99={} current_p99={} confidence={} unstable_percentile={} p95_status={} p99_status={}",
            row.metric,
            row.phase,
            row.owner,
            row.next_action,
            row.current_count,
            row.baseline_count,
            row.current_count,
            row.baseline_p95,
            row.current_p95,
            row.delta_p95(),
            row.baseline_p99,
            row.current_p99,
            row.confidence.as_str(),
            row.confidence.unstable_percentile(),
            row.confidence.p95_status(),
            row.confidence.p99_status()
        )?;
    }
    Ok(())
}

fn compare_benchmark_rows(
    baseline_path: &Path,
    current_path: &Path,
    rank: BenchmarkCompareRank,
) -> Result<(&'static str, &'static str, Vec<BenchmarkCompareRow>), Box<dyn Error>> {
    let baseline = load_benchmark_summaries(baseline_path)?;
    let current = load_benchmark_summaries(current_path)?;
    let baseline_by_metric = baseline
        .summaries
        .iter()
        .map(|summary| (summary.metric().to_owned(), summary))
        .collect::<BTreeMap<_, _>>();
    let mut rows = current
        .summaries
        .iter()
        .filter_map(|current| {
            let baseline = baseline_by_metric.get(current.metric())?;
            let owner = firkin::evidence::benchmark_metric_ownership(current.metric());
            let compare_confidence = min_percentile_availability(
                baseline.percentile_availability(),
                current.percentile_availability(),
            );
            Some(BenchmarkCompareRow {
                metric: current.metric().to_owned(),
                owner: owner.owner,
                phase: owner.phase_label,
                next_action: if compare_confidence.p95_status() == "decision_grade" {
                    owner.next_action_hint
                } else {
                    "collect_more_samples"
                },
                baseline_p95: baseline.p95(),
                current_p95: current.p95(),
                baseline_p99: baseline.p99(),
                current_p99: current.p99(),
                baseline_count: baseline.count(),
                current_count: current.count(),
                confidence: compare_confidence,
            })
        })
        .collect::<Vec<_>>();
    sort_compare_rows(&mut rows, rank);
    Ok((baseline.kind, current.kind, rows))
}

#[derive(Clone)]
struct LoadedBenchmarkSummaries {
    kind: &'static str,
    summaries: Vec<firkin::evidence::BenchmarkSummary>,
}

fn load_benchmark_summaries(path: &Path) -> Result<LoadedBenchmarkSummaries, Box<dyn Error>> {
    if let Ok(report) = firkin::evidence::AgentBenchmarkScorecardArtifact::read_json(path)
        && report.required_metrics() == firkin::evidence::P0_SCORECARD_METRICS
    {
        return Ok(LoadedBenchmarkSummaries {
            kind: "scorecard",
            summaries: report.summaries().to_vec(),
        });
    }
    if let Ok(report) = firkin::evidence::AutoscaleEfficiencyScorecardArtifact::read_json(path)
        && report.required_metrics() == firkin::evidence::AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
    {
        return Ok(LoadedBenchmarkSummaries {
            kind: "autoscale_scorecard",
            summaries: report.summaries().to_vec(),
        });
    }
    if let Ok(report) = firkin::evidence::AgentComputerScorecardArtifact::read_json(path)
        && report.required_metrics() == firkin::evidence::AGENT_COMPUTER_SCORECARD_METRICS
    {
        return Ok(LoadedBenchmarkSummaries {
            kind: "agent_computer_scorecard",
            summaries: report.summaries().to_vec(),
        });
    }
    if let Ok(report) = firkin::evidence::BenchmarkOverheadEvidenceArtifact::read_json(path) {
        let required_overhead = firkin::evidence::REQUIRED_FIRKIN_OVERHEAD_METRICS
            .iter()
            .map(|metric| metric.name)
            .collect::<Vec<_>>();
        if report.required_metrics() == required_overhead {
            return Ok(LoadedBenchmarkSummaries {
                kind: "overhead",
                summaries: report.summaries().to_vec(),
            });
        }
    }
    if let Ok(report) = firkin::evidence::BenchmarkEvidenceArtifact::read_json(path) {
        let kind =
            if report.required_metrics() == firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS {
                "lifecycle"
            } else {
                "benchmark"
            };
        return Ok(LoadedBenchmarkSummaries {
            kind,
            summaries: report.summaries().to_vec(),
        });
    }
    if let Ok(artifact) = load_raw_sample_artifact_summaries(path) {
        return Ok(artifact);
    }
    Err(format!("unsupported benchmark artifact {}", path.display()).into())
}

#[derive(serde::Deserialize)]
struct RawBenchmarkSampleArtifact {
    #[serde(default)]
    kind: String,
    samples: Vec<firkin::trace::BenchmarkSample>,
}

fn load_raw_sample_artifact_summaries(
    path: &Path,
) -> Result<LoadedBenchmarkSummaries, Box<dyn Error>> {
    let bytes = std::fs::read(path)?;
    let artifact = serde_json::from_slice::<RawBenchmarkSampleArtifact>(&bytes)?;
    if artifact.samples.is_empty() {
        return Err(format!(
            "benchmark sample artifact has no samples: {}",
            path.display()
        )
        .into());
    }
    let mut grouped = BTreeMap::<String, Vec<firkin::trace::BenchmarkSample>>::new();
    for sample in artifact.samples {
        grouped
            .entry(sample.metric().to_owned())
            .or_default()
            .push(sample);
    }
    let mut summaries = Vec::with_capacity(grouped.len());
    for (metric, samples) in grouped {
        summaries.push(firkin::evidence::BenchmarkSummary::from_samples(
            metric.clone(),
            samples,
        )?);
    }
    Ok(LoadedBenchmarkSummaries {
        kind: raw_sample_artifact_kind(&artifact.kind),
        summaries,
    })
}

fn raw_sample_artifact_kind(kind: &str) -> &'static str {
    match kind {
        "live_direct_exec_first_stdout" => "live_direct_exec_first_stdout",
        "live_resume_to_first_stdout" => "live_resume_to_first_stdout",
        "live_retained_shell_batch_100" => "live_retained_shell_batch_100",
        "live_retained_shell_density" => "live_retained_shell_density",
        "live_warm_to_first_stdout" => "live_warm_to_first_stdout",
        "live_product_pod_disk_reclaim" => "live_product_pod_disk_reclaim",
        _ => "live_sample_artifact",
    }
}

#[derive(Clone)]
struct BenchmarkCompareRow {
    metric: String,
    owner: &'static str,
    phase: &'static str,
    next_action: &'static str,
    baseline_p95: f64,
    current_p95: f64,
    baseline_p99: f64,
    current_p99: f64,
    baseline_count: usize,
    current_count: usize,
    confidence: firkin::evidence::PercentileAvailability,
}

impl BenchmarkCompareRow {
    fn delta_p95(&self) -> f64 {
        self.current_p95 - self.baseline_p95
    }
}

fn sort_compare_rows(rows: &mut [BenchmarkCompareRow], rank: BenchmarkCompareRank) {
    rows.sort_by(|left, right| {
        let left_value = match rank {
            BenchmarkCompareRank::Bottlenecks => left.current_p95,
            BenchmarkCompareRank::Regressions => left.delta_p95(),
            BenchmarkCompareRank::Improvements => -left.delta_p95(),
        };
        let right_value = match rank {
            BenchmarkCompareRank::Bottlenecks => right.current_p95,
            BenchmarkCompareRank::Regressions => right.delta_p95(),
            BenchmarkCompareRank::Improvements => -right.delta_p95(),
        };
        right_value.total_cmp(&left_value)
    });
}

fn min_percentile_availability(
    left: firkin::evidence::PercentileAvailability,
    right: firkin::evidence::PercentileAvailability,
) -> firkin::evidence::PercentileAvailability {
    if percentile_availability_rank(left) <= percentile_availability_rank(right) {
        left
    } else {
        right
    }
}

const fn percentile_availability_rank(
    availability: firkin::evidence::PercentileAvailability,
) -> u8 {
    match availability {
        firkin::evidence::PercentileAvailability::SmokeOnly => 0,
        firkin::evidence::PercentileAvailability::SuperfastIteration => 1,
        firkin::evidence::PercentileAvailability::FastIteration => 2,
        firkin::evidence::PercentileAvailability::BaselineCheckpoint => 3,
        firkin::evidence::PercentileAvailability::P50P90DecisionGrade => 4,
        firkin::evidence::PercentileAvailability::P95DecisionGrade => 5,
        firkin::evidence::PercentileAvailability::P99DecisionGrade => 6,
    }
}

fn write_benchmark_proof(
    args: &BenchmarkProofArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let source = std::fs::read_to_string(&args.source)?;
    let title = format!("Firkin Benchmark {} Proof", args.milestone.to_uppercase());
    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{}</title>
  <style>
    body {{ margin: 0; font: 15px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #f8f8f5; color: #17191c; }}
    main {{ max-width: 1080px; margin: 0 auto; padding: 36px 22px 60px; }}
    h1 {{ margin: 0 0 10px; font-size: 32px; }}
    .status {{ display: inline-block; padding: 3px 9px; border-radius: 999px; background: #0b7a3e; color: white; font-weight: 700; font-size: 12px; }}
    pre {{ white-space: pre-wrap; overflow-wrap: anywhere; padding: 14px; background: #eeeeea; border: 1px solid #d8d8d0; border-radius: 8px; }}
  </style>
</head>
<body>
<main>
  <h1>{}</h1>
  <p><span class="status">generated</span></p>
  <p>source: <code>{}</code></p>
  <h2>Evidence</h2>
  <pre>{}</pre>
</main>
</body>
</html>
"#,
        xml_escape(&title),
        xml_escape(&title),
        xml_escape(&args.source.display().to_string()),
        xml_escape(&source)
    );
    atomic_write_file(&args.out, html.as_bytes())?;
    writeln!(
        writer,
        "benchmark_proof=written milestone={} source={} out={}",
        args.milestone,
        args.source.display(),
        args.out.display()
    )?;
    Ok(())
}

fn write_benchmark_sprint_ready(
    args: &BenchmarkSprintReadyArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let root = args
        .benchmark_root
        .clone()
        .unwrap_or_else(firkin_benchmark_root);
    let baseline_name = validate_baseline_name(&args.baseline)?;
    let baseline = baseline_path(&root, baseline_name);
    if !baseline.exists() {
        return Err(format!("missing baseline {}", baseline.display()).into());
    }
    let doctor_args = BenchmarkDoctorArgs {
        mode: args.mode,
        state_root: None,
        cache_root: None,
        benchmark_root: Some(root.clone()),
        min_free_bytes: args.min_free_bytes,
    };
    let mut doctor = Vec::new();
    if let Err(error) = write_benchmark_doctor(&doctor_args, &mut doctor) {
        writeln!(writer, "{}", String::from_utf8_lossy(&doctor).trim_end())?;
        write_sprint_ready_blocked(args, &baseline, error.as_ref(), &mut writer)?;
        return Err(error);
    }
    let Some(current) = &args.current_artifact else {
        return Err("sprint-ready requires --current-artifact".into());
    };
    let Some(overhead) = &args.overhead_artifact else {
        return Err("sprint-ready requires --overhead-artifact".into());
    };
    let overhead_args = ValidateOverheadSloArgs {
        artifact: overhead.clone(),
        min_samples: 1,
    };
    let mut overhead_slo = Vec::new();
    validate_overhead_slo(&overhead_args, &mut overhead_slo)?;
    writeln!(
        writer,
        "{}",
        String::from_utf8_lossy(&overhead_slo).trim_end()
    )?;
    if let Some(scorecard) = &args.scorecard_artifact {
        let scorecard_args = ValidateScorecardArgs {
            artifact: scorecard.clone(),
            min_samples: 1,
            require_snappy: false,
        };
        let mut scorecard_output = Vec::new();
        validate_scorecard(&scorecard_args, &mut scorecard_output)?;
        writeln!(
            writer,
            "{}",
            String::from_utf8_lossy(&scorecard_output).trim_end()
        )?;
    }
    let mut coverage_artifacts = vec![current.clone(), overhead.clone()];
    if let Some(scorecard) = &args.scorecard_artifact {
        coverage_artifacts.push(scorecard.clone());
    }
    let coverage_args = BenchmarkCoverageArgs {
        strict: true,
        artifact: coverage_artifacts,
    };
    let mut coverage = Vec::new();
    if let Err(error) = write_benchmark_coverage(&coverage_args, &mut coverage) {
        writeln!(writer, "{}", String::from_utf8_lossy(&coverage).trim_end())?;
        write_sprint_ready_compare_or_blocked(&baseline, current, &mut writer)?;
        write_sprint_ready_blocked(args, &baseline, error.as_ref(), &mut writer)?;
        return Err(error);
    }
    writeln!(writer, "{}", String::from_utf8_lossy(&coverage).trim_end())?;
    let compare_args = BenchmarkCompareArgs {
        baseline: baseline.clone(),
        current: current.clone(),
        rank: BenchmarkCompareRank::Bottlenecks,
    };
    let mut compare = Vec::new();
    compare_benchmark_artifacts(&compare_args, &mut compare)?;
    writeln!(writer, "{}", String::from_utf8_lossy(&compare).trim_end())?;
    writeln!(
        writer,
        "sprint-ready=passed suite={} mode={} baseline={} first_command=\"fk benchmark run {} --mode {} --duration 30s --out target/firkin-live-evidence/current-30s.json\"",
        args.suite,
        args.mode.as_str(),
        baseline.display(),
        args.suite,
        args.mode.as_str()
    )?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn write_benchmark_sprint_record(
    args: &BenchmarkSprintRecordArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let root = args
        .benchmark_root
        .clone()
        .unwrap_or_else(firkin_benchmark_root);
    let baseline_name = validate_baseline_name(&args.baseline)?;
    let baseline = baseline_path(&root, baseline_name);
    require_existing_file("baseline", &baseline)?;
    require_existing_file("current artifact", &args.current_artifact)?;
    require_existing_file("overhead artifact", &args.overhead_artifact)?;
    if let Some(scorecard) = &args.scorecard_artifact {
        require_existing_file("scorecard artifact", scorecard)?;
    }

    let doctor_args = BenchmarkDoctorArgs {
        mode: args.mode,
        state_root: None,
        cache_root: None,
        benchmark_root: Some(root.clone()),
        min_free_bytes: args.min_free_bytes,
    };
    let (doctor_ok, doctor_output, doctor_error) =
        capture_benchmark_output(|output| write_benchmark_doctor(&doctor_args, output));

    let overhead_args = ValidateOverheadSloArgs {
        artifact: args.overhead_artifact.clone(),
        min_samples: 1,
    };
    let (overhead_ok, overhead_output, overhead_error) =
        capture_benchmark_output(|output| validate_overhead_slo(&overhead_args, output));

    let scorecard_result = args.scorecard_artifact.as_ref().map(|scorecard| {
        let scorecard_args = ValidateScorecardArgs {
            artifact: scorecard.clone(),
            min_samples: 1,
            require_snappy: false,
        };
        capture_benchmark_output(|output| validate_scorecard(&scorecard_args, output))
    });

    let mut coverage_artifacts = vec![
        args.current_artifact.clone(),
        args.overhead_artifact.clone(),
    ];
    if let Some(scorecard) = &args.scorecard_artifact {
        coverage_artifacts.push(scorecard.clone());
    }
    let coverage_args = BenchmarkCoverageArgs {
        strict: true,
        artifact: coverage_artifacts,
    };
    let (coverage_ok, coverage_output, coverage_error) =
        capture_benchmark_output(|output| write_benchmark_coverage(&coverage_args, output));

    let compare_args = BenchmarkCompareArgs {
        baseline: baseline.clone(),
        current: args.current_artifact.clone(),
        rank: BenchmarkCompareRank::Bottlenecks,
    };
    let (compare_ok, compare_output, compare_error) =
        capture_benchmark_output(|output| compare_benchmark_artifacts(&compare_args, output));
    let top_bottleneck = compare_benchmark_rows(
        &baseline,
        &args.current_artifact,
        BenchmarkCompareRank::Bottlenecks,
    )
    .ok()
    .and_then(|(_, _, rows)| rows.into_iter().next());

    let sprint_ready_args = BenchmarkSprintReadyArgs {
        suite: args.suite.clone(),
        baseline: args.baseline.clone(),
        mode: args.mode,
        current_artifact: Some(args.current_artifact.clone()),
        overhead_artifact: Some(args.overhead_artifact.clone()),
        scorecard_artifact: args.scorecard_artifact.clone(),
        benchmark_root: Some(root),
        min_free_bytes: args.min_free_bytes,
    };
    let (sprint_ready_ok, sprint_ready_output, sprint_ready_error) =
        capture_benchmark_output(|output| write_benchmark_sprint_ready(&sprint_ready_args, output));

    let status = if doctor_ok
        && overhead_ok
        && scorecard_result.as_ref().is_none_or(|(ok, _, _)| *ok)
        && coverage_ok
        && compare_ok
        && sprint_ready_ok
    {
        "passed"
    } else {
        "blocked"
    };
    let markdown = render_sprint_record_markdown(SprintRecordRender {
        args,
        baseline: &baseline,
        status,
        doctor_output: &doctor_output,
        doctor_error: doctor_error.as_deref(),
        overhead_output: &overhead_output,
        overhead_error: overhead_error.as_deref(),
        scorecard_result: scorecard_result.as_ref(),
        coverage_output: &coverage_output,
        coverage_error: coverage_error.as_deref(),
        compare_output: &compare_output,
        compare_error: compare_error.as_deref(),
        sprint_ready_output: &sprint_ready_output,
        sprint_ready_error: sprint_ready_error.as_deref(),
        top_bottleneck: top_bottleneck.as_ref(),
    });
    atomic_write_file(&args.out, markdown.as_bytes())?;
    writeln!(
        writer,
        "sprint_record=written status={} suite={} baseline={} out={}",
        status,
        args.suite,
        baseline.display(),
        args.out.display()
    )?;
    if status == "passed" {
        Ok(())
    } else {
        Err(sprint_record_error([
            doctor_error,
            overhead_error,
            scorecard_result.and_then(|(_, _, error)| error),
            coverage_error,
            compare_error,
            sprint_ready_error,
        ])
        .into())
    }
}

fn require_existing_file(label: &str, path: &Path) -> Result<(), Box<dyn Error>> {
    if path.is_file() {
        Ok(())
    } else {
        Err(format!("missing {label}: {}", path.display()).into())
    }
}

fn capture_benchmark_output(
    f: impl FnOnce(&mut Vec<u8>) -> Result<(), Box<dyn Error>>,
) -> (bool, String, Option<String>) {
    let mut output = Vec::new();
    match f(&mut output) {
        Ok(()) => (
            true,
            String::from_utf8_lossy(&output).trim_end().to_owned(),
            None,
        ),
        Err(error) => (
            false,
            String::from_utf8_lossy(&output).trim_end().to_owned(),
            Some(error.to_string()),
        ),
    }
}

struct SprintRecordRender<'a> {
    args: &'a BenchmarkSprintRecordArgs,
    baseline: &'a Path,
    status: &'a str,
    doctor_output: &'a str,
    doctor_error: Option<&'a str>,
    overhead_output: &'a str,
    overhead_error: Option<&'a str>,
    scorecard_result: Option<&'a (bool, String, Option<String>)>,
    coverage_output: &'a str,
    coverage_error: Option<&'a str>,
    compare_output: &'a str,
    compare_error: Option<&'a str>,
    sprint_ready_output: &'a str,
    sprint_ready_error: Option<&'a str>,
    top_bottleneck: Option<&'a BenchmarkCompareRow>,
}

#[allow(
    clippy::format_push_string,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]
fn render_sprint_record_markdown(input: SprintRecordRender<'_>) -> String {
    let next_command = sprint_next_30s_command(input.args);
    let top_bottleneck = input.top_bottleneck.map_or_else(
        || "none".to_owned(),
        |row| {
            format!(
                "{} phase={} owner={} current_p95={} delta_p95={} confidence={}",
                row.metric,
                row.phase,
                row.owner,
                row.current_p95,
                row.delta_p95(),
                row.confidence.as_str()
            )
        },
    );
    let confidence = input
        .top_bottleneck
        .map_or("none", |row| row.confidence.as_str());
    let residual_risks = sprint_residual_risks(input.status, input.top_bottleneck);
    let mut markdown = String::new();
    markdown.push_str("# Firkin P0 Sprint Record\n\n");
    markdown.push_str(&format!("status: {}\n", input.status));
    markdown.push_str(&format!("suite: {}\n", input.args.suite));
    markdown.push_str(&format!("mode: {}\n", input.args.mode.as_str()));
    markdown.push_str(&format!("baseline: {}\n", input.baseline.display()));
    markdown.push_str(&format!(
        "current_artifact: {}\n",
        input.args.current_artifact.display()
    ));
    markdown.push_str(&format!(
        "overhead_artifact: {}\n",
        input.args.overhead_artifact.display()
    ));
    markdown.push_str(&format!(
        "scorecard_artifact: {}\n",
        input
            .args
            .scorecard_artifact
            .as_ref()
            .map_or_else(|| "-".to_owned(), |path| path.display().to_string())
    ));
    markdown.push_str(&format!("top_bottleneck: {top_bottleneck}\n"));
    markdown.push_str(&format!("confidence: {confidence}\n"));
    markdown.push_str(&format!("residual_risks: {residual_risks}\n"));
    markdown.push_str(&format!("next_30s_command: `{next_command}`\n\n"));
    markdown.push_str("## Exact Commands\n\n");
    markdown.push_str(&format!(
        "doctor: `{}`\n",
        sprint_record_doctor_command(input.args)
    ));
    markdown.push_str(&format!(
        "overhead_slo: `cargo run -q -p firkin-cli -- benchmark validate-overhead-slo {} --min-samples 1`\n",
        input.args.overhead_artifact.display()
    ));
    if let Some(scorecard) = &input.args.scorecard_artifact {
        markdown.push_str(&format!(
            "scorecard: `cargo run -q -p firkin-cli -- benchmark validate-scorecard {} --min-samples 1`\n",
            scorecard.display()
        ));
    }
    markdown.push_str(&format!(
        "strict_coverage: `{}`\n",
        sprint_record_coverage_command(input.args)
    ));
    markdown.push_str(&format!(
        "compare: `cargo run -q -p firkin-cli -- benchmark compare {} {} --rank bottlenecks`\n",
        input.baseline.display(),
        input.args.current_artifact.display()
    ));
    markdown.push_str(&format!(
        "sprint_ready: `{}`\n\n",
        sprint_record_ready_command(input.args)
    ));
    push_record_section(
        &mut markdown,
        "Doctor",
        input.doctor_output,
        input.doctor_error,
    );
    push_record_section(
        &mut markdown,
        "Overhead SLO",
        input.overhead_output,
        input.overhead_error,
    );
    if let Some((_, output, error)) = input.scorecard_result {
        push_record_section(&mut markdown, "Scorecard", output, error.as_deref());
    }
    push_record_section(
        &mut markdown,
        "Strict Coverage",
        input.coverage_output,
        input.coverage_error,
    );
    push_record_section(
        &mut markdown,
        "Compare",
        input.compare_output,
        input.compare_error,
    );
    push_record_section(
        &mut markdown,
        "Sprint Ready",
        input.sprint_ready_output,
        input.sprint_ready_error,
    );
    while markdown.ends_with("\n\n") {
        markdown.pop();
    }
    markdown
}

#[allow(clippy::format_push_string)]
fn push_record_section(markdown: &mut String, title: &str, output: &str, error: Option<&str>) {
    markdown.push_str(&format!("## {title}\n\n"));
    if let Some(error) = error {
        markdown.push_str(&format!("failure: {error}\n\n"));
    }
    markdown.push_str("```text\n");
    markdown.push_str(output);
    if !output.ends_with('\n') {
        markdown.push('\n');
    }
    markdown.push_str("```\n\n");
}

fn sprint_record_doctor_command(args: &BenchmarkSprintRecordArgs) -> String {
    format!(
        "cargo run -q -p firkin-cli -- benchmark doctor --mode {} --min-free-bytes {}",
        args.mode.as_str(),
        args.min_free_bytes
    )
}

#[allow(clippy::format_push_string)]
fn sprint_record_coverage_command(args: &BenchmarkSprintRecordArgs) -> String {
    let mut command = format!(
        "cargo run -q -p firkin-cli -- benchmark coverage --strict --artifact {} --artifact {}",
        args.current_artifact.display(),
        args.overhead_artifact.display()
    );
    if let Some(scorecard) = &args.scorecard_artifact {
        command.push_str(&format!(" --artifact {}", scorecard.display()));
    }
    command
}

#[allow(clippy::format_push_string)]
fn sprint_record_ready_command(args: &BenchmarkSprintRecordArgs) -> String {
    let mut command = format!(
        "cargo run -q -p firkin-cli -- benchmark sprint-ready --suite {} --baseline {} --mode {} --current-artifact {} --overhead-artifact {} --min-free-bytes {}",
        args.suite,
        args.baseline,
        args.mode.as_str(),
        args.current_artifact.display(),
        args.overhead_artifact.display(),
        args.min_free_bytes
    );
    if let Some(scorecard) = &args.scorecard_artifact {
        command.push_str(&format!(" --scorecard-artifact {}", scorecard.display()));
    }
    command
}

fn sprint_next_30s_command(args: &BenchmarkSprintRecordArgs) -> String {
    format!(
        "fk benchmark run {} --mode {} --duration 30s --out target/firkin-live-evidence/current-30s.json",
        args.suite,
        args.mode.as_str()
    )
}

fn sprint_residual_risks(status: &str, top_bottleneck: Option<&BenchmarkCompareRow>) -> String {
    if status != "passed" {
        return "blocked gate output above is current source of truth".to_owned();
    }
    match top_bottleneck.map(|row| row.confidence) {
        Some(firkin::evidence::PercentileAvailability::P99DecisionGrade) => {
            "live Apple/VZ availability and hardware variance still apply".to_owned()
        }
        Some(confidence) => {
            format!(
                "{}; increase sample count before treating p99 as optimization truth",
                confidence.as_str()
            )
        }
        None => "no matched compare row; establish comparable baseline/current metrics".to_owned(),
    }
}

fn sprint_record_error(errors: impl IntoIterator<Item = Option<String>>) -> String {
    let failures = errors.into_iter().flatten().collect::<Vec<_>>();
    if failures.is_empty() {
        "sprint record blocked".to_owned()
    } else {
        format!("sprint record blocked: {}", failures.join("; "))
    }
}

fn write_sprint_ready_compare_or_blocked(
    baseline: &Path,
    current: &Path,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let compare_args = BenchmarkCompareArgs {
        baseline: baseline.to_path_buf(),
        current: current.to_path_buf(),
        rank: BenchmarkCompareRank::Bottlenecks,
    };
    let mut compare = Vec::new();
    match compare_benchmark_artifacts(&compare_args, &mut compare) {
        Ok(()) => writeln!(writer, "{}", String::from_utf8_lossy(&compare).trim_end())?,
        Err(compare_error) => writeln!(writer, "benchmark_compare=blocked error={compare_error}")?,
    }
    Ok(())
}

fn write_sprint_ready_blocked(
    args: &BenchmarkSprintReadyArgs,
    baseline: &Path,
    error: &dyn Error,
    mut writer: impl Write,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "sprint-ready=blocked suite={} mode={} baseline={} first_command=\"fk benchmark run {} --mode {} --duration 30s --out target/firkin-live-evidence/current-30s.json\" reason=\"{}\"",
        args.suite,
        args.mode.as_str(),
        baseline.display(),
        args.suite,
        args.mode.as_str(),
        error
    )
}

fn validate_soak(args: &ValidateSoakArgs, mut writer: impl Write) -> Result<(), Box<dyn Error>> {
    let report = firkin::substrate::SoakEvidenceArtifact::read_json(&args.artifact)?;
    let gate = report.validate_production()?;
    let benchmark_artifact = PathBuf::from(gate.benchmark_artifact());
    if !benchmark_artifact.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "soak benchmark artifact missing: {}",
                benchmark_artifact.display()
            ),
        )
        .into());
    }
    let _benchmark = firkin::evidence::BenchmarkEvidenceArtifact::read_json(&benchmark_artifact)?;
    writeln!(
        writer,
        "soak_gate=passed artifact={} duration_seconds={} steps={}",
        args.artifact.display(),
        report.duration().as_secs(),
        gate.covered_steps().len()
    )?;
    Ok(())
}

fn write_substrate_snapshot_sidecars(
    args: &SubstrateSnapshotSidecarsArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let manifest = match args.kind.manifest_kind() {
        firkin::substrate::SnapshotArtifactKind::BaseTemplate => {
            firkin::substrate::SnapshotArtifactManifest::base(&args.logical_id, &args.artifact)
        }
        firkin::substrate::SnapshotArtifactKind::Continuation => {
            firkin::substrate::SnapshotArtifactManifest::continuation(
                &args.logical_id,
                &args.artifact,
            )
        }
    };
    let integrity = firkin::substrate::SnapshotArtifactIntegrity::from_file(&manifest)?;
    let manifest_path =
        firkin::substrate::SnapshotArtifactManifest::sidecar_path_for_artifact(&args.artifact);
    let integrity_path =
        firkin::substrate::SnapshotArtifactIntegrity::sidecar_path_for_artifact(&args.artifact);

    manifest.write_json(&manifest_path)?;
    integrity.write_json(&integrity_path)?;
    writeln!(
        writer,
        "snapshot_sidecars=written artifact={} logical_id={} kind={} manifest={} integrity={} size_bytes={} sha256={}",
        args.artifact.display(),
        args.logical_id,
        args.kind.output_label(),
        manifest_path.display(),
        integrity_path.display(),
        integrity.size_bytes(),
        integrity.sha256_hex(),
    )?;
    Ok(())
}

fn run_substrate_hygiene_once(
    args: &SubstrateHygieneOnceArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let maintenance = substrate_hygiene_maintenance(
        &args.snapshot_root,
        &args.log_root,
        args.manifest_root.as_deref(),
        args.max_log_bytes,
        args.gzip_logs,
        Duration::ZERO,
    );
    let report = maintenance.tick()?;
    writeln!(
        writer,
        "hygiene_tick=passed snapshot_root={} log_root={} manifest_root={} artifact_deleted={} log_rotated={} gzip_logs={}",
        args.snapshot_root.display(),
        args.log_root.display(),
        args.manifest_root
            .as_ref()
            .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
        report.artifact_gc().deleted_count(),
        report.log_rotation().rotated_count(),
        args.gzip_logs
    )?;
    Ok(())
}

async fn run_substrate_hygiene_daemon(
    args: &SubstrateHygieneDaemonArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let interval = Duration::from_secs(args.interval_seconds);
    let maintenance = substrate_hygiene_maintenance(
        &args.snapshot_root,
        &args.log_root,
        args.manifest_root.as_deref(),
        args.max_log_bytes,
        args.gzip_logs,
        interval,
    );
    let handle = maintenance.spawn();
    writeln!(
        writer,
        "hygiene_daemon=started snapshot_root={} log_root={} manifest_root={} interval_seconds={} max_log_bytes={} gzip_logs={}",
        args.snapshot_root.display(),
        args.log_root.display(),
        args.manifest_root
            .as_ref()
            .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
        args.interval_seconds,
        args.max_log_bytes,
        args.gzip_logs
    )?;
    tokio::signal::ctrl_c().await?;
    handle.shutdown().await?;
    writeln!(writer, "hygiene_daemon=stopped")?;
    Ok(())
}

fn substrate_hygiene_maintenance(
    snapshot_root: &Path,
    log_root: &Path,
    manifest_root: Option<&Path>,
    max_log_bytes: u64,
    gzip_logs: bool,
    interval: Duration,
) -> firkin_runtime::RuntimeHygieneMaintenance {
    let mut maintenance = firkin_runtime::RuntimeHygieneMaintenance::new(
        snapshot_root,
        [],
        log_root,
        max_log_bytes,
        interval,
    );
    if let Some(manifest_root) = manifest_root {
        maintenance = maintenance.with_manifest_dir(manifest_root);
    }
    if gzip_logs {
        maintenance = maintenance.with_gzip_log_compression();
    }
    maintenance
}

fn write_substrate_hygiene_launchd_plist(
    args: &SubstrateHygieneLaunchdPlistArgs,
    mut writer: impl Write,
) -> std::io::Result<()> {
    let mut program_arguments = vec![
        args.fk_bin.display().to_string(),
        "substrate".to_owned(),
        "hygiene-daemon".to_owned(),
        "--snapshot-root".to_owned(),
        args.snapshot_root.display().to_string(),
        "--log-root".to_owned(),
        args.log_root.display().to_string(),
        "--max-log-bytes".to_owned(),
        args.max_log_bytes.to_string(),
        "--interval-seconds".to_owned(),
        args.interval_seconds.to_string(),
    ];
    if let Some(manifest_root) = &args.manifest_root {
        program_arguments.push("--manifest-root".to_owned());
        program_arguments.push(manifest_root.display().to_string());
    }
    if args.gzip_logs {
        program_arguments.push("--gzip-logs".to_owned());
    }

    writeln!(writer, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(
        writer,
        r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#
    )?;
    writeln!(writer, r#"<plist version="1.0">"#)?;
    writeln!(writer, "<dict>")?;
    writeln!(writer, "  <key>Label</key>")?;
    writeln!(writer, "  <string>{}</string>", xml_escape(&args.label))?;
    writeln!(writer, "  <key>ProgramArguments</key>")?;
    writeln!(writer, "  <array>")?;
    for argument in &program_arguments {
        writeln!(writer, "    <string>{}</string>", xml_escape(argument))?;
    }
    writeln!(writer, "  </array>")?;
    writeln!(writer, "  <key>RunAtLoad</key>")?;
    writeln!(writer, "  <true/>")?;
    writeln!(writer, "  <key>KeepAlive</key>")?;
    writeln!(writer, "  <true/>")?;
    if let Some(path) = &args.standard_out_path {
        writeln!(writer, "  <key>StandardOutPath</key>")?;
        writeln!(
            writer,
            "  <string>{}</string>",
            xml_escape(&path.display().to_string())
        )?;
    }
    if let Some(path) = &args.standard_error_path {
        writeln!(writer, "  <key>StandardErrorPath</key>")?;
        writeln!(
            writer,
            "  <string>{}</string>",
            xml_escape(&path.display().to_string())
        )?;
    }
    writeln!(writer, "</dict>")?;
    writeln!(writer, "</plist>")?;
    Ok(())
}

fn write_substrate_reconcile_launchd_plist(
    args: &SubstrateReconcileLaunchdPlistArgs,
    mut writer: impl Write,
) -> std::io::Result<()> {
    let program_arguments = vec![
        args.fk_bin.display().to_string(),
        "substrate".to_owned(),
        "reconcile-once".to_owned(),
        "--active-vm-root".to_owned(),
        args.active_vm_root.display().to_string(),
        "--snapshot-root".to_owned(),
        args.snapshot_root.display().to_string(),
        "--log-root".to_owned(),
        args.log_root.display().to_string(),
        "--process-root".to_owned(),
        args.process_root.display().to_string(),
        "--quarantine-root".to_owned(),
        args.quarantine_root.display().to_string(),
        "--heartbeat-timeout-seconds".to_owned(),
        args.heartbeat_timeout_seconds.to_string(),
    ];

    writeln!(writer, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(
        writer,
        r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#
    )?;
    writeln!(writer, r#"<plist version="1.0">"#)?;
    writeln!(writer, "<dict>")?;
    writeln!(writer, "  <key>Label</key>")?;
    writeln!(writer, "  <string>{}</string>", xml_escape(&args.label))?;
    writeln!(writer, "  <key>ProgramArguments</key>")?;
    writeln!(writer, "  <array>")?;
    for argument in &program_arguments {
        writeln!(writer, "    <string>{}</string>", xml_escape(argument))?;
    }
    writeln!(writer, "  </array>")?;
    writeln!(writer, "  <key>RunAtLoad</key>")?;
    writeln!(writer, "  <true/>")?;
    writeln!(writer, "  <key>StartInterval</key>")?;
    writeln!(writer, "  <integer>{}</integer>", args.interval_seconds)?;
    if let Some(path) = &args.standard_out_path {
        writeln!(writer, "  <key>StandardOutPath</key>")?;
        writeln!(
            writer,
            "  <string>{}</string>",
            xml_escape(&path.display().to_string())
        )?;
    }
    if let Some(path) = &args.standard_error_path {
        writeln!(writer, "  <key>StandardErrorPath</key>")?;
        writeln!(
            writer,
            "  <string>{}</string>",
            xml_escape(&path.display().to_string())
        )?;
    }
    writeln!(writer, "</dict>")?;
    writeln!(writer, "</plist>")?;
    Ok(())
}

fn install_substrate_reconcile_launchd_plist(
    args: &SubstrateReconcileLaunchdInstallArgs,
    mut writer: impl Write,
) -> std::io::Result<()> {
    let mut plist = Vec::new();
    write_substrate_reconcile_launchd_plist(&args.launchd, &mut plist)?;
    atomic_write_file(&args.plist_path, &plist)?;
    writeln!(
        writer,
        "reconcile_launchd_plist=installed path={} label={}",
        args.plist_path.display(),
        args.launchd.label
    )
}

fn install_substrate_hygiene_launchd_plist(
    args: &SubstrateHygieneLaunchdInstallArgs,
    mut writer: impl Write,
) -> std::io::Result<()> {
    let mut plist = Vec::new();
    write_substrate_hygiene_launchd_plist(&args.launchd, &mut plist)?;
    atomic_write_file(&args.plist_path, &plist)?;
    writeln!(
        writer,
        "hygiene_launchd_plist=installed path={} label={}",
        args.plist_path.display(),
        args.launchd.label
    )?;
    Ok(())
}

fn run_substrate_hygiene_launchd_bootstrap(
    args: &SubstrateHygieneLaunchdBootstrapArgs,
    mut writer: impl Write,
) -> std::io::Result<()> {
    for command in hygiene_launchd_bootstrap_commands(args) {
        run_launchctl(&command)?;
    }
    writeln!(
        writer,
        "hygiene_launchd=bootstrapped domain={} label={} plist_path={}",
        args.domain,
        args.label,
        args.plist_path.display()
    )?;
    Ok(())
}

fn hygiene_launchd_bootstrap_commands(
    args: &SubstrateHygieneLaunchdBootstrapArgs,
) -> Vec<Vec<String>> {
    launchd_bootstrap_commands(&args.domain, &args.label, &args.plist_path)
}

fn run_substrate_reconcile_launchd_bootstrap(
    args: &SubstrateReconcileLaunchdBootstrapArgs,
    mut writer: impl Write,
) -> std::io::Result<()> {
    for command in reconcile_launchd_bootstrap_commands(args) {
        run_launchctl(&command)?;
    }
    writeln!(
        writer,
        "reconcile_launchd=bootstrapped domain={} label={} plist_path={}",
        args.domain,
        args.label,
        args.plist_path.display()
    )?;
    Ok(())
}

fn reconcile_launchd_bootstrap_commands(
    args: &SubstrateReconcileLaunchdBootstrapArgs,
) -> Vec<Vec<String>> {
    launchd_bootstrap_commands(&args.domain, &args.label, &args.plist_path)
}

fn launchd_bootstrap_commands(domain: &str, label: &str, plist_path: &Path) -> Vec<Vec<String>> {
    vec![
        vec![
            "bootstrap".to_owned(),
            domain.to_owned(),
            plist_path.display().to_string(),
        ],
        vec![
            "kickstart".to_owned(),
            "-k".to_owned(),
            format!("{domain}/{label}"),
        ],
    ]
}

fn run_substrate_hygiene_launchd_status(
    args: &SubstrateHygieneLaunchdStatusArgs,
) -> std::io::Result<()> {
    run_launchctl(&hygiene_launchd_status_command(args))
}

fn hygiene_launchd_status_command(args: &SubstrateHygieneLaunchdStatusArgs) -> Vec<String> {
    launchd_status_command(&args.domain, &args.label)
}

fn run_substrate_reconcile_launchd_status(
    args: &SubstrateReconcileLaunchdStatusArgs,
) -> std::io::Result<()> {
    run_launchctl(&reconcile_launchd_status_command(args))
}

fn reconcile_launchd_status_command(args: &SubstrateReconcileLaunchdStatusArgs) -> Vec<String> {
    launchd_status_command(&args.domain, &args.label)
}

fn launchd_status_command(domain: &str, label: &str) -> Vec<String> {
    vec!["print".to_owned(), format!("{domain}/{label}")]
}

fn run_launchctl(args: &[String]) -> std::io::Result<()> {
    let status = ProcessCommand::new("launchctl").args(args).status()?;
    if status.success() {
        return Ok(());
    }
    Err(std::io::Error::other(format!(
        "launchctl {} exited with {status}",
        args.join(" ")
    )))
}

fn atomic_write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension().and_then(|ext| ext.to_str()).unwrap_or(""),
        std::process::id()
    ));
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

fn write_substrate_stuck_vm_plan(
    args: &SubstrateStuckVmPlanArgs,
    mut writer: impl Write,
) -> std::io::Result<()> {
    let timeout = Duration::from_secs(args.heartbeat_timeout_seconds);
    let observations = args.vms.iter().map(|vm| {
        firkin::substrate::StuckVmObservation::new(
            &vm.id,
            Duration::from_secs(vm.heartbeat_age_seconds),
        )
    });
    let plan = firkin::substrate::StuckVmCleanupPlan::from_observations(observations, timeout);
    writeln!(
        writer,
        "stuck_vm_plan=ok heartbeat_timeout_seconds={} decisions={}",
        args.heartbeat_timeout_seconds,
        plan.decisions().len()
    )?;
    for entry in plan.decisions() {
        let observation = entry.observation();
        writeln!(
            writer,
            "vm_id={} heartbeat_age_seconds={} decision={}",
            printable_vm_id(observation.id()),
            observation.heartbeat_age().as_secs(),
            stuck_vm_decision_name(entry.decision())
        )?;
    }
    Ok(())
}

fn write_substrate_host_scan(
    args: &SubstrateHostScanArgs,
    mut writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    let scan = firkin_runtime::RuntimeHostScanner::new(
        &args.active_vm_root,
        &args.snapshot_root,
        &args.log_root,
        &args.process_root,
    )
    .scan()?;
    let reconciliation = scan.reconciliation_plan();
    let stuck_vms = scan.stuck_vm_cleanup_plan(Duration::from_secs(args.heartbeat_timeout_seconds));
    let restart_decisions = reconciliation
        .decisions()
        .iter()
        .map(|entry| {
            let record = entry.record();
            SubstrateRestartDecisionReport {
                id: record.id(),
                kind: restart_resource_kind_name(record.kind()),
                decision: reconciliation_decision_name(entry.action()),
            }
        })
        .collect::<Vec<_>>();
    let stuck_vm_decisions = stuck_vms
        .decisions()
        .iter()
        .map(|entry| {
            let observation = entry.observation();
            SubstrateStuckVmDecisionReport {
                id: observation.id(),
                heartbeat_age_seconds: observation.heartbeat_age().as_secs(),
                runtime_pid: observation.runtime_pid(),
                decision: stuck_vm_decision_name(entry.decision()),
            }
        })
        .collect::<Vec<_>>();
    let report = SubstrateHostScanReport {
        host_scan: "ok",
        heartbeat_timeout_seconds: args.heartbeat_timeout_seconds,
        restart_decision_count: restart_decisions.len(),
        stuck_vm_decision_count: stuck_vm_decisions.len(),
        restart_decisions,
        stuck_vm_decisions,
    };
    serde_json::to_writer(&mut writer, &report)?;
    writeln!(writer)?;
    Ok(())
}

fn run_substrate_reconcile_once(
    args: &SubstrateReconcileOnceArgs,
    writer: impl Write,
) -> Result<(), Box<dyn Error>> {
    run_substrate_reconcile_once_with_terminator(
        args,
        writer,
        firkin_runtime::CommandHostProcessTerminator,
    )
}

fn run_substrate_reconcile_once_with_terminator<T>(
    args: &SubstrateReconcileOnceArgs,
    mut writer: impl Write,
    mut terminator: T,
) -> Result<(), Box<dyn Error>>
where
    T: firkin_runtime::HostProcessTerminator,
    T::Error: Error + Send + Sync + 'static,
{
    let recovery = firkin_runtime::RuntimeRestartRecovery::new(
        &args.active_vm_root,
        &args.snapshot_root,
        &args.log_root,
        &args.process_root,
        &args.quarantine_root,
        Duration::from_secs(args.heartbeat_timeout_seconds),
    );
    let recovery_report = recovery.execute_with_terminator(&mut terminator)?;
    let restart_report = recovery_report.restart();
    let stuck_vm_report = recovery_report.stuck_vm();
    let report = SubstrateReconcileOnceReport {
        reconcile_once: "ok",
        restart: SubstrateRestartReconcileCounts {
            recovered: restart_report.recovered_count(),
            cleaned: restart_report.cleaned_count(),
            quarantined: restart_report.quarantined_count(),
        },
        stuck_vm: SubstrateStuckVmReconcileCounts {
            preserved: stuck_vm_report.preserved_count(),
            cleaned: stuck_vm_report.cleaned_count(),
            quarantined: stuck_vm_report.quarantined_count(),
        },
    };
    serde_json::to_writer(&mut writer, &report)?;
    writeln!(writer)?;
    Ok(())
}

fn reconciliation_decision_name(
    decision: firkin::substrate::ReconciliationDecision,
) -> &'static str {
    match decision {
        firkin::substrate::ReconciliationDecision::Recover => "recover",
        firkin::substrate::ReconciliationDecision::Cleanup => "cleanup",
        firkin::substrate::ReconciliationDecision::Quarantine => "quarantine",
    }
}

fn restart_resource_kind_name(kind: firkin::substrate::RestartResourceKind) -> &'static str {
    match kind {
        firkin::substrate::RestartResourceKind::ActiveVm => "active_vm",
        firkin::substrate::RestartResourceKind::SnapshotArtifact => "snapshot_artifact",
        firkin::substrate::RestartResourceKind::LogStream => "log_stream",
        firkin::substrate::RestartResourceKind::StaleRuntimeProcess => "stale_runtime_process",
    }
}

fn stuck_vm_decision_name(decision: firkin::substrate::StuckVmCleanupDecision) -> &'static str {
    match decision {
        firkin::substrate::StuckVmCleanupDecision::Preserve => "preserve",
        firkin::substrate::StuckVmCleanupDecision::Cleanup => "cleanup",
        firkin::substrate::StuckVmCleanupDecision::Quarantine => "quarantine",
    }
}

fn printable_vm_id(vm_id: &str) -> &str {
    if vm_id.is_empty() { "-" } else { vm_id }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}

struct AcceptanceCheck {
    id: &'static str,
    status: &'static str,
    evidence: &'static str,
    notes: &'static str,
}

const ACCEPTANCE_CHECKS: &[AcceptanceCheck] = &[
    AcceptanceCheck {
        id: "template_build_snapshot",
        status: "signed_live_vz_template_snapshot_proven",
        evidence: "firkin-template-executor-tests+firkin-runtime-template-build-tests+firkin-runtime-snapshot-feature-check+signed-live-vz-template-build-snapshot-smoke+signed-live-cubeapi-classic-template-route-smoke",
        notes: "runtime-template-command-path-core-runner-live-vz-clone-setup-cache-warm-snapshot-save-manifest-and-integrity-sidecar-write-restore-proven-cubeapi-classic-create-rebuild-status-logs-list-detail-delete-unit-and-signed-live-product-route-proven",
    },
    AcceptanceCheck {
        id: "snapshot_restore_default_create",
        status: "core_live_restore_proven",
        evidence: "firkin-runtime-create-from-snapshot-tests+firkin-runtime-e2b-adapter-tests+signed-live-core-snapshot-restore-smoke+signed-live-sdk-domain-proxy-process-smoke+signed-live-sdk-domain-proxy-concurrent-retained-stdin-smoke",
        notes: "capacity-admitted-async-snapshot-restore-records-latency-cube-create-wrapper-localruntimebackend-create-sdk-domain-proxy-process-route-reusable-core-launcher-and-private-restored-rootfs-stage-proven",
    },
    AcceptanceCheck {
        id: "warm_pool_lifecycle",
        status: "signed_live_product_route_proven",
        evidence: "firkin-substrate-warm-pool-tests+firkin-runtime-warm-pool-tests+firkin-runtime-warm-pool-service-tests+firkin-runtime-e2b-adapter-warm-checkout-test+firkin-runtime-e2b-adapter-warm-target-maintain-test+firkin-runtime-e2b-adapter-same-template-depth-test+firkin-runtime-e2b-adapter-warm-template-maintainer-tests+firkin-runtime-e2b-adapter-backend-ready-template-maintainer-test+firkin-runtime-e2b-adapter-clean-prewarm-policy-test+signed-live-vz-warm-pool-checkout-smoke+signed-live-sdk-domain-proxy-prewarmed-template-smoke",
        notes: "retained-session-pool-maintain-checkout-capacity-latency-replenishment-policy-runtime-pass-bounded-supervisor-loop-service-owner-spawned-task-shutdown-firkin-runtime-adapter-prewarm-template-product-create-checkout-idempotent-warm-template-target-maintenance-pass-same-template-warm-depth-spawned-background-refill-owner-and-localruntimebackend-ready-template-target-derivation-clean-prewarm-policy-retains-restored-session-without-readiness-probe-or-command-side-effects-until-checkout-signed-live-vz-product-route-runs-command",
    },
    AcceptanceCheck {
        id: "freshness_sync_readonly_then_writable",
        status: "signed_live_product_route_proven",
        evidence: "firkin-template-freshness-sync-tests+firkin-runtime-e2b-adapter-freshness-sync-write-gate-test+firkin-runtime-e2b-adapter-guest-freshness-sync-test+firkin-runtime-e2b-adapter-automatic-guest-sync-test+signed-live-vz-freshness-sync-smoke+signed-live-vz-freshness-product-route-smoke+containerization-netlink-route-add-default-replace-test",
        notes: "runtime-create-metadata-installs-syncing-gate-filesystem-reads-continue-filesystem-writes-block-runtime-session-git-fetch-checkout-reset-is-spawned-after-restore-and-unlocks-writes-signed-live-vmnet-restore-public-repo-fast-forward-proof-passes-through-direct-adapter-and-cubeapi-e2b-post-sandboxes-product-route-vminitd-default-route-replace-keeps-restored-network-setup-idempotent",
    },
    AcceptanceCheck {
        id: "continuation_snapshot_resume",
        status: "signed_live_product_route_proven",
        evidence: "firkin-substrate-continuation-snapshot-tests+firkin-runtime-continuation-snapshot-tests+firkin-e2b-followup-route-tests+firkin-runtime-e2b-adapter-snapshot-route-tests+firkin-runtime-e2b-adapter-followup-route-tests+firkin-runtime-e2b-adapter-delete-snapshot-test+signed-live-vz-continuation-capture-restore-smoke+signed-live-vz-create-snapshot-product-route-smoke+signed-live-vz-followup-product-route-smoke",
        notes: "follow-up-snapshot-capture-writes-manifest-and-integrity-sidecars-runtime-restore-cube-followup-wrapper-runtime-adapter-create-snapshot-route-followup-product-route-and-firkin-continuation-artifact-delete-sidecar-cleanup-proven-signed-live-vz-create-snapshot-product-route-captures-restorable-guest-state-needs-soak-coverage",
    },
    AcceptanceCheck {
        id: "reserved_port_routing",
        status: "code_interpreter_python_context_smoke_proven",
        evidence: "firkin-runtime-e2b-adapter-tests+domain-proxy-code-interpreter-execute-test+host-backed-python-context-test+signed-live-sdk-domain-proxy-code-interpreter-probe-smoke+signed-live-vz-code-interpreter-execute-smoke+signed-live-vz-concurrent-code-interpreter-execute-smoke+signed-live-vz-python-context-smoke+domain-proxy-mcp-connect-tunnel-test",
        notes: "envd-and-code-interpreter-reserved-ports-route-to-runtime-owned-http-services-signed-live-vz-code-interpreter-probe-through-product-domain-proxy-passes-code-interpreter-execute-bash-protocol-runs-through-runtime-envd-adapter-and-product-domain-proxy-with-single-and-two-active-sandbox-signed-live-vz-proofs-python-context-id-persists-pickleable-namespace-state-across-execute-requests-with-host-backed-and-signed-live-vz-proofs-mcp-reserved-port-connect-tunnels-through-firkin-port-router-full-jupyter-kernel-parity-and-guest-mcp-service-semantics-deferred-to-v2",
    },
    AcceptanceCheck {
        id: "envd_filesystem_operations",
        status: "signed_live_sdk_domain_proxy_filesystem_proven",
        evidence: "firkin-runtime-e2b-adapter-tests+envd-http-server-file-read-write-proof+envd-http-server-grpc-web-text-watch-streaming-proof+signed-live-sdk-domain-proxy-filesystem-smoke+signed-live-sdk-domain-proxy-concurrent-filesystem-smoke+domain-proxy-two-active-filesystem-route-test",
        notes: "read-write-stat-list-mkdir-move-remove-watch-finite-run-through-active-restored-session-command-runner-and-vendored-sdk-domain-proxy-write-read-stat-list-remove-missing-exists-and-concurrent-filesystem-smokes-pass-two-active-sandbox-filesystem-read-write-stat-list-remove-routing-passes-grpc-web-text-watch-streams-start-and-filesystem-events-before-watch-end",
    },
    AcceptanceCheck {
        id: "envd_process_records",
        status: "signed_live_sdk_domain_proxy_process_proven",
        evidence: "firkin-runtime-e2b-adapter-tests+envd-http-server-grpc-web-list-proof+signed-live-runtime-snapshot-process-smokes+signed-live-vm-envd-process-http-smoke+signed-live-sdk-domain-proxy-process-smoke+signed-live-sdk-domain-proxy-concurrent-command-smoke+signed-live-sdk-domain-proxy-retained-stdin-smoke+signed-live-sdk-domain-proxy-retained-pty-smoke+signed-live-sdk-domain-proxy-concurrent-retained-stdin-smoke+domain-proxy-two-active-process-route-test+domain-proxy-two-active-retained-process-route-test",
        notes: "finite-start-list-connect-http-list-and-retained-interactive-stdin-signal-pty-input-resize-output-buffering-defined-sandbox-scoped-process-records-preserved-when-one-active-sandbox-stops-signed-live-vz-command-stdin-pty-envd-http-vendored-sdk-domain-proxy-process-concurrent-finite-command-retained-stdin-retained-pty-input-resize-connect-signal-and-concurrent-retained-stdin-smokes-pass-two-active-sandbox-process-and-retained-process-connect-routing-passes",
    },
    AcceptanceCheck {
        id: "stop_lifecycle",
        status: "signed_live_sdk_kill_delete_proven",
        evidence: "firkin-runtime-e2b-adapter-tests+signed-live-sdk-domain-proxy-process-smoke",
        notes: "vendored-sdk-kill-through-domain-proxy-stops-restored-vz-session-releases-active-capacity-and-records-kill-delete-latency",
    },
    AcceptanceCheck {
        id: "latency_benchmarks",
        status: "representative_slo_gate_proven",
        evidence: "fk-benchmark-targets+fk-benchmark-validate-lifecycle-slo+fk-benchmark-report-lifecycle+firkin-substrate-benchmark-evidence-tests+firkin-substrate-benchmark-slo-gate-tests+firkin-runtime-benchmark-evidence-writer-tests+signed-live-vz-required-lifecycle-benchmark-artifact-smoke+just-live-runtime-benchmark-slo-gate+just-live-runtime-benchmark-representative",
        notes: "required-lifecycle-metric-validator-json-artifact-runtime-writer-shared-configured-slo-p95-sample-count-gate-cli-and-signed-live-vz-smoke-cover-cold-template-build-warm-restore-command-start-first-stdout-ready-probe-snapshot-save-kill-delete-warm-pool-checkout-concurrent-create-representative-signed-live-min-samples-3-passes-cold-template-build-163ms-warm-snapshot-restore-173ms-command-start-36ms-first-stdout-byte-43ms-ready-probe-sub-ms-snapshot-save-100ms-kill-delete-2ms-warm-pool-checkout-sub-ms-concurrent-create-2527ms",
    },
    AcceptanceCheck {
        id: "overhead_benchmarks",
        status: "representative_slo_gate_proven",
        evidence: "fk-benchmark-targets+fk-benchmark-validate-overhead-slo+fk-benchmark-report-overhead+firkin-substrate-benchmark-schema-tests+firkin-substrate-overhead-evidence-tests+firkin-substrate-benchmark-slo-gate-tests+firkin-runtime-overhead-evidence-writer-tests+signed-live-vz-required-overhead-artifact-smoke+just-live-runtime-overhead-slo-gate+just-live-runtime-overhead-representative",
        notes: "firkin-tax-shape-required-overhead-validator-runtime-writer-shared-json-artifact-slo-p95-sample-count-gate-cli-and-signed-live-vz-smoke-cover-control-plane-cpu-idle-control-plane-rss-idle-per-sandbox-host-rss-disk-metadata-growth-idle-wakeup-rate-representative-signed-live-min-samples-3-passes-cpu-idle-0-percent-rss-idle-33-99mib-per-sandbox-host-rss-14-04mib-disk-metadata-4096b-idle-wakeup-0hz",
    },
    AcceptanceCheck {
        id: "runtime_preflight",
        status: "product_and_adapter_preflight_wired",
        evidence: "fk-debug-preflight+firkin-runtime-preflight-tests+firkin-cli-e2b-host-preflight-tests+firkin-runtime-adapter-preflight-tests+firkin-runtime-managed-roots-tests",
        notes: "vmm-capability-signing-preflight-exists-e2b-host-creates-runtime-roots-and-checks-required-sandbox-log-roots-and-10gib-host-disk-floor-before-serving-control-and-proxy-firkin-runtime-adapter-checks-configured-runtime-preflight-before-start-and-prewarm-restore-work-managed-runtime-roots-helper-wires-snapshot-log-preflight-and-active-vm-marker-root-together-for-production-composition",
    },
    AcceptanceCheck {
        id: "restart_reconciliation",
        status: "signed_live_vz_marker_host_scan_proven",
        evidence: "firkin-substrate-reconciliation-tests+firkin-runtime-reconciliation-tests+firkin-runtime-filesystem-reconciler-tests+firkin-runtime-host-scan-tests+firkin-runtime-active-vm-marker-test+firkin-runtime-host-process-stuck-vm-cleaner-test+firkin-runtime-managed-roots-tests+fk-substrate-host-scan+fk-substrate-reconcile-once+fk-substrate-reconcile-launchd-plist+fk-substrate-reconcile-launchd-install+fk-substrate-reconcile-launchd-bootstrap+fk-substrate-reconcile-launchd-status+signed-live-vz-active-marker-host-scan-smoke",
        notes: "filesystem-host-scan-feeds-recover-cleanup-quarantine-runtime-executor-json-host-scan-output-includes-stuck-vm-runtime-pid-for-service-consumption-runtime-restart-recovery-owner-scans-reconciles-and-runs-host-process-stuck-vm-cleanup-filesystem-marker-cleanup-quarantine-adapter-runtime-adapter-active-heartbeat-timestamp-plus-runtime-pid-and-executable-directory-marker-publish-refresh-remove-managed-runtime-roots-helper-wires-active-vm-marker-root-with-runtime-preflight-one-shot-reconcile-command-treats-missing-marker-roots-as-empty-and-uses-executable-checked-host-process-terminator-for-stuck-active-vm-cleanup-startinterval-launchd-plist-atomic-plist-install-launchctl-bootstrap-kickstart-and-status-signed-live-running-vz-session-marker-host-scan-recover-decision-proven-unmanaged-external-vz-process-enumeration-out-of-scope",
    },
    AcceptanceCheck {
        id: "snapshot_artifact_gc",
        status: "signed_live_hygiene_pressure_proven",
        evidence: "firkin-substrate-artifact-gc-tests+firkin-substrate-snapshot-manifest-sidecar-tests+firkin-runtime-artifact-gc-tests+firkin-runtime-hygiene-maintenance-tests+fk-substrate-hygiene-once+fk-substrate-hygiene-daemon+fk-substrate-hygiene-launchd-plist+fk-substrate-hygiene-launchd-install+fk-substrate-hygiene-launchd-bootstrap+fk-substrate-hygiene-launchd-status+signed-live-vz-hygiene-pressure-smoke",
        notes: "unreferenced-file-and-directory-gc-executes-through-runtime-wrapper-for-snapshot-root-with-age-based-retention-and-periodic-hygiene-owner-manifest-json-sidecars-persist-discover-feed-runtime-gc-and-can-be-read-each-maintenance-tick-fk-substrate-hygiene-once-runs-operator-schedulable-sidecar-backed-gc-pass-fk-substrate-hygiene-daemon-runs-the-periodic-owner-until-interrupted-launchd-plist-rendering-is-available-install-writes-the-plist-atomically-bootstrap-runs-launchctl-bootstrap-plus-kickstart-and-status-runs-launchctl-print-signed-live-vz-snapshot-artifact-preserved-and-stale-snapshot-directory-reclaimed-under-hygiene-pressure",
    },
    AcceptanceCheck {
        id: "snapshot_artifact_integrity",
        status: "signed_live_integrity_reject_proven",
        evidence: "firkin-substrate-artifact-integrity-tests+firkin-substrate-snapshot-manifest-sidecar-tests+firkin-runtime-create-from-snapshot-integrity-tests+firkin-runtime-e2b-adapter-integrity-tests+firkin-e2b-state-persistence-test+fk-substrate-snapshot-sidecars+signed-live-vz-mutated-snapshot-integrity-reject-smoke",
        notes: "snapshot-size-and-sha256-verify-runtime-verified-restore-checks-integrity-before-disk-probe-capacity-launch-adapter-cold-start-warm-prewarm-product-followup-restore-and-local-state-json-persistence-manifest-json-sidecars-persist-discover-and-are-written-by-template-and-continuation-snapshot-capture-integrity-json-sidecars-persist-are-written-by-capture-can-be-consumed-by-restore-and-can-fill-missing-prepared-template-integrity-fk-substrate-snapshot-sidecars-writes-operator-import-manifest-and-integrity-sidecars-for-existing-artifacts-signed-live-vz-snapshot-mutated-after-capture-is-rejected-before-restore",
    },
    AcceptanceCheck {
        id: "log_rotation",
        status: "signed_live_hygiene_pressure_proven",
        evidence: "firkin-substrate-log-rotation-tests+firkin-runtime-log-rotation-tests+firkin-runtime-hygiene-maintenance-tests+firkin-runtime-gzip-log-rotation-tests+fk-substrate-hygiene-once+fk-substrate-hygiene-daemon+fk-substrate-hygiene-launchd-plist+fk-substrate-hygiene-launchd-install+fk-substrate-hygiene-launchd-bootstrap+fk-substrate-hygiene-launchd-status+signed-live-vz-hygiene-pressure-smoke",
        notes: "oversized-log-rotation-executes-through-runtime-wrapper-for-log-root-with-bounded-generation-retention-optional-gzip-compression-periodic-hygiene-owner-fk-substrate-hygiene-once-hook-fk-substrate-hygiene-daemon-periodic-entrypoint-launchd-plist-rendering-atomic-plist-install-launchctl-bootstrap-plus-kickstart-and-launchctl-print-status-signed-live-vz-hygiene-pressure-smoke-rotates-oversized-runtime-log-in-same-maintenance-tick-as-snapshot-gc",
    },
    AcceptanceCheck {
        id: "stuck_vm_cleanup",
        status: "signed_live_host_process_cleanup_proven",
        evidence: "firkin-substrate-stuck-vm-cleanup-tests+firkin-runtime-stuck-vm-cleanup-tests+firkin-runtime-host-process-stuck-vm-cleaner-test+firkin-runtime-filesystem-reconciler-tests+firkin-runtime-active-vm-marker-test+firkin-runtime-managed-roots-tests+fk-substrate-stuck-vm-plan+fk-substrate-host-scan+fk-substrate-reconcile-once+fk-substrate-reconcile-launchd-plist+fk-substrate-reconcile-launchd-install+fk-substrate-reconcile-launchd-bootstrap+fk-substrate-reconcile-launchd-status+signed-live-vz-active-marker-host-scan-smoke+signed-live-host-process-stuck-vm-cleanup-smoke",
        notes: "filesystem-host-scan-heartbeat-and-runtime-pid-input-feeds-preserve-cleanup-quarantine-runtime-executor-operator-visible-plan-output-json-host-scan-output-filesystem-active-vm-marker-cleanup-quarantine-adapter-runtime-adapter-active-heartbeat-runtime-pid-and-executable-directory-marker-publish-refresh-remove-managed-runtime-roots-helper-wires-production-active-marker-root-one-shot-reconcile-command-treats-missing-marker-roots-as-empty-and-terminates-executable-matched-marked-host-process-before-active-vm-marker-cleanup-startinterval-launchd-plist-atomic-plist-install-launchctl-bootstrap-kickstart-and-status-signed-live-running-vz-session-marker-host-scan-preserve-decision-proven-signed-live-stale-marker-host-process-termination-and-marker-cleanup-proven-unmanaged-external-vz-process-enumeration-out-of-scope",
    },
    AcceptanceCheck {
        id: "capacity_scheduler_pressure",
        status: "runtime_active_queue_backpressure_wired",
        evidence: "firkin-substrate-capacity-ledger-tests+firkin-substrate-active-backpressure-tests+firkin-runtime-create-from-snapshot-tests+firkin-runtime-disk-pressure-tests+firkin-runtime-warm-pool-pressure-tests+firkin-runtime-template-build-disk-pressure-tests+firkin-runtime-continuation-snapshot-disk-pressure-tests+firkin-runtime-e2b-adapter-warm-disk-floor-test+firkin-runtime-e2b-adapter-active-queue-test+firkin-runtime-e2b-adapter-followup-queue-test",
        notes: "cpu-ram-disk-accounting-bounded-active-queue-backpressure-policy-df-backed-10gib-host-free-space-floor-and-firkin-runtime-adapter-cold-create-plus-followup-queue-owner-enforced-before-snapshot-restore-capacity-launch-template-snapshot-save-and-continuation-snapshot-capture-warm-pool-refill-and-adapter-prewarm-stop-at-20gib-to-avoid-starving-active-sandboxes",
    },
    AcceptanceCheck {
        id: "multi_container_vm_substrate",
        status: "core_live_smoke_proven",
        evidence: "signed-core-builder-live-two-containers-one-vm-smoke+cubeapi-firkin-create-restore-single-vm-backed-container-tests",
        notes: "substrate-live-smoke-passes-with-signed-harness-cube-default-mapping-create-and-snapshot-restore-tests-assert-single-vm-backed-container-runtime-mode-one-cube-sandbox-per-firkin-vm-backed-container-advanced-pod-design-recorded-in-docs-plans-2026-05-04-firkin-pod-support-design-md-but-firkin-core-pod-api-runtime-block-device-attach-pod-aware-manifest-restore-cube-post-pods-route-and-signed-live-pod-smokes-not-landed",
    },
    AcceptanceCheck {
        id: "network_policy_hard_fail",
        status: "final_adapter_path_proven",
        evidence: "cubeapi-host-and-firkin-runtime-adapter-tests",
        notes: "restrictive-policy-hard-fails-and-firkin-session-rolls-back-until-real-enforcement-exists",
    },
    AcceptanceCheck {
        id: "single_node_24h_soak",
        status: "runner_smoke_proven",
        evidence: "firkin-substrate-soak-scenario-tests+firkin-substrate-soak-evidence-artifact-tests+firkin-runtime-product-soak-runner-test+signed-live-vz-product-soak-smoke+fk-substrate-validate-soak+live-runtime-soak-24h-recipe",
        notes: "inspect-like-loop-runner-drives-product-create-command-file-snapshot-followup-cleanup-and-writes-json-artifact-signed-live-1s-smoke-passes-validator-requires-24h-duration-readable-benchmark-artifact-cleanup-evidence-zero-orphans-zero-failures-actual-24h-run-artifact-missing",
    },
];

async fn debug_boot(args: DebugBootArgs) -> Result<(), Box<dyn Error>> {
    if VMINITD_AARCH64.is_empty() || VMEXEC_AARCH64.is_empty() {
        return Err("debug boot requires embedded vminitd and vmexec bytes; rebuild without firkin-vminitd-bytes/runtime-download".into());
    }

    let init_block = firkin::ext4::init_block::synthesize(VMINITD_AARCH64, VMEXEC_AARCH64)?;
    let kernel = resolve_kernel(args.kernel)?;
    let boot_log = args.boot_log.map_or(BootLog::None, BootLog::File);
    let network = args
        .vmnet_subnet
        .map_or(Network::Nat, Network::vmnet_shared_subnet);

    let config = VmConfig::builder()
        .kernel(KernelImage::from_file(kernel))
        .init_block(init_block)
        .boot_log(boot_log)
        .networks([network])
        .build()?;
    let vm = VirtualMachine::new(config).boot().await?;

    println!("vm_id={}", vm.id());
    println!("state={:?}", vm.state());
    for interface in vm.network_interfaces() {
        println!(
            "interface={} address={}/{} gateway={}",
            interface.name(),
            interface.ipv4_address(),
            interface.prefix(),
            interface.gateway()
        );
    }

    if args.hold_secs > 0 {
        tokio::time::sleep(Duration::from_secs(args.hold_secs)).await;
    }

    vm.stop().await?;
    Ok(())
}

async fn e2b_host(args: E2bHostArgs) -> Result<(), Box<dyn Error>> {
    let domain: Hostname = args.domain.parse()?;
    let state_dir = default_e2b_state_dir();
    let root = args.root.unwrap_or_else(|| state_dir.join("sandboxes"));
    let state_path = args.state.unwrap_or_else(|| state_dir.join("state.json"));
    let log_root = e2b_log_root(&state_path, &state_dir);
    if let Some(parent) = state_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::create_dir_all(&root)?;
    std::fs::create_dir_all(&log_root)?;
    preflight_e2b_host_runtime_roots(&root, &log_root)?;

    let runtime = HostRuntimeAdapter::new(&root, domain.clone());
    let backend = if state_path.exists() {
        LocalRuntimeBackend::load_state_json(runtime.clone(), &state_path)?
    } else {
        LocalRuntimeBackend::new(runtime.clone(), SystemLifecycleClock.now_rfc3339())
    };
    runtime.restore_from_state(&backend.export_state()).await?;

    let control_listener = tokio::net::TcpListener::bind(args.control_addr).await?;
    let proxy_listener = tokio::net::TcpListener::bind(args.proxy_addr).await?;
    let control_addr = control_listener.local_addr()?;
    let proxy_addr = proxy_listener.local_addr()?;
    if !args.skip_domain_preflight {
        preflight_e2b_proxy_domain(&domain, proxy_addr)?;
    }
    let proxy_tls = match (
        args.proxy_tls_cert.as_deref(),
        args.proxy_tls_key.as_deref(),
    ) {
        (Some(cert), Some(key)) => Some(DomainProxyTlsIdentity::from_pem_files(cert, key)?),
        (None, None) => None,
        _ => return Err("--proxy-tls-cert and --proxy-tls-key must be passed together".into()),
    };
    let mut control = ControlPlaneHttpServer::new_persistent(backend, &state_path);
    if let Some(api_key) = args.api_key.as_deref() {
        control = control.with_required_api_key(api_key);
    }
    let proxy = firkin::e2b::DomainProxyHttpServer::from_control_plane(&control, domain.clone());

    println!("e2b_control_url=http://{control_addr}/");
    let proxy_scheme = if proxy_tls.is_some() { "https" } else { "http" };
    println!("e2b_proxy_url={proxy_scheme}://{proxy_addr}/");
    println!("e2b_domain={domain}");
    println!("e2b_domain_probe_host=49983-sbx_probe.{domain}");
    println!("export E2B_API_URL=http://{control_addr}/");
    println!("export E2B_DOMAIN={domain}");
    if let Some(cert_path) = args.proxy_tls_cert.as_deref() {
        println!("export E2B_CA_CERT_FILE={}", cert_path.display());
        println!("export E2B_SANDBOX_RESOLVE_ADDR={proxy_addr}");
    } else {
        println!("export E2B_SANDBOX_URL=http://{proxy_addr}/");
    }
    if args.api_key.is_some() {
        println!("e2b_api_key_required=true");
    }
    println!("e2b_state_path={}", state_path.display());
    println!("e2b_sandbox_root={}", root.display());
    println!("e2b_log_root={}", log_root.display());
    println!(
        "e2b_lifecycle_interval_seconds={}",
        args.lifecycle_interval_seconds
    );

    let lifecycle_task = control
        .lifecycle_scheduler(Duration::from_secs(args.lifecycle_interval_seconds))
        .spawn();
    let serve_result = if let Some(identity) = proxy_tls {
        tokio::try_join!(
            async {
                control
                    .serve(control_listener)
                    .await
                    .map_err(|error| -> Box<dyn Error> { Box::new(error) })
            },
            async {
                proxy
                    .serve_tls(proxy_listener, identity)
                    .await
                    .map_err(|error| -> Box<dyn Error> { Box::new(error) })
            }
        )
        .map(|_| ())
    } else {
        tokio::try_join!(control.serve(control_listener), proxy.serve(proxy_listener))
            .map(|_| ())
            .map_err(|error| -> Box<dyn Error> { Box::new(error) })
    };
    lifecycle_task.abort();
    serve_result
}

fn e2b_log_root(state_path: &Path, fallback_state_dir: &Path) -> PathBuf {
    state_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(fallback_state_dir)
        .join("logs")
}

fn preflight_e2b_host_runtime_roots(
    sandbox_root: &Path,
    log_root: &Path,
) -> Result<(), Box<dyn Error>> {
    let mut probe = firkin_runtime::HostDiskPressureProbe::new();
    preflight_e2b_host_runtime_roots_with_disk_probe(sandbox_root, log_root, &mut probe)
}

fn preflight_e2b_host_runtime_roots_with_disk_probe<P>(
    sandbox_root: &Path,
    log_root: &Path,
    probe: &mut P,
) -> Result<(), Box<dyn Error>>
where
    P: firkin_runtime::DiskPressureProbe,
    P::Error: Display,
{
    firkin_runtime::RuntimePreflight::new(sandbox_root, log_root, firkin::types::Size::gib(10))
        .check_with_disk_probe(probe)?;
    Ok(())
}

fn preflight_e2b_proxy_domain(
    domain: &Hostname,
    proxy_addr: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    let probe_host = format!("49983-sbx_probe.{domain}");
    let resolved = (probe_host.as_str(), proxy_addr.port())
        .to_socket_addrs()
        .map_err(|error| {
            format!(
                "E2B proxy domain `{domain}` is not resolvable as `{probe_host}`: {error}; use a *.localhost domain or configure local DNS before starting"
            )
        })?
        .collect::<Vec<_>>();
    let proxy_ips = loopback_equivalent_ips(proxy_addr.ip());
    if resolved
        .iter()
        .any(|addr| proxy_ips.contains(&addr.ip()) && addr.port() == proxy_addr.port())
    {
        Ok(())
    } else {
        Err(format!(
            "E2B proxy domain `{domain}` resolves `{probe_host}` to {resolved:?}, not proxy listener {proxy_addr}; configure local DNS or pass --skip-domain-preflight for a deliberately external proxy"
        )
        .into())
    }
}

fn loopback_equivalent_ips(ip: IpAddr) -> Vec<IpAddr> {
    if ip.is_loopback() {
        vec![
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        ]
    } else {
        vec![ip]
    }
}

fn default_e2b_state_dir() -> PathBuf {
    firkin_runtime::FirkinStorageConfig::from_env().e2b_state_dir()
}

fn client(platform: Option<&str>) -> Result<Client, Box<dyn Error>> {
    let Some(platform) = platform else {
        return Ok(Client::default());
    };

    let platform = match platform {
        "linux/arm64" => Platform::linux_arm64(),
        "linux/arm64/v8" => Platform::linux_arm64_v8(),
        "linux/amd64" => Platform::linux_amd64(),
        _ => {
            return Err(format!(
                "unsupported platform {platform:?}; expected linux/arm64, linux/arm64/v8, or linux/amd64"
            )
            .into());
        }
    };

    Ok(Client::builder().platform(platform).build()?)
}

fn resolve_kernel(explicit: Option<PathBuf>) -> Result<PathBuf, Box<dyn Error>> {
    let path = explicit
        .or_else(|| std::env::var_os("FIRKIN_KERNEL_PATH").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../bin/vmlinux"));
    Ok(path.canonicalize().map_err(|error| {
        format!(
            "kernel path {} is not readable; pass --kernel or set FIRKIN_KERNEL_PATH: {error}",
            path.display()
        )
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn output_metric_names(output: &str) -> std::collections::BTreeSet<&str> {
        output
            .lines()
            .filter_map(|line| line.strip_prefix("metric="))
            .filter_map(|line| line.split_once(' ').map(|(metric, _)| metric))
            .collect()
    }

    #[test]
    fn clear_roots_include_explicit_scopes_and_legacy_tmp_roots() {
        let storage = firkin_runtime::FirkinStorageConfig::from_roots(
            "/var/firkin/state",
            "/var/firkin/cache",
        );
        let roots = clear_roots(
            Path::new("/tmp"),
            &storage,
            Path::new("/var/firkin/benchmarks"),
            ClearSelection {
                state: true,
                cache: false,
                benchmarks: true,
                legacy_tmp: true,
            },
        )
        .expect("clear roots resolve");

        assert_eq!(
            roots,
            vec![
                ClearRoot {
                    label: "legacy_tmp_firkin_cache",
                    path: PathBuf::from("/tmp/firkin"),
                    kind: ClearRootKind::Cache,
                },
                ClearRoot {
                    label: "legacy_tmp_runtime_continuations",
                    path: PathBuf::from("/tmp/firkin-runtime-continuations"),
                    kind: ClearRootKind::LegacyTemp,
                },
                ClearRoot {
                    label: "legacy_tmp_runtime_restore_staging",
                    path: PathBuf::from("/tmp/firkin-runtime-restore-staging"),
                    kind: ClearRootKind::LegacyTemp,
                },
                ClearRoot {
                    label: "legacy_tmp_single_node_snapshots",
                    path: PathBuf::from("/tmp/firkin-single-node-snapshots"),
                    kind: ClearRootKind::LegacyTemp,
                },
                ClearRoot {
                    label: "firkin_benchmarks",
                    path: PathBuf::from("/var/firkin/benchmarks"),
                    kind: ClearRootKind::Benchmarks,
                },
                ClearRoot {
                    label: "firkin_state",
                    path: PathBuf::from("/var/firkin/state"),
                    kind: ClearRootKind::State,
                },
            ]
        );
    }

    #[test]
    fn clear_deletes_configured_state_cache_and_legacy_tmp_roots() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let state_root = tempdir.path().join("state");
        let cache_root = tempdir.path().join("cache");
        let tmp_root = tempdir.path().join("tmp");
        let state_continuations = state_root.join("runtime/continuations");
        let tmp_restore_staging = tmp_root.join("firkin-runtime-restore-staging");

        std::fs::create_dir_all(&state_continuations).expect("state continuations dir");
        std::fs::write(state_continuations.join("snapshot.vz"), b"snapshot")
            .expect("snapshot file");
        std::fs::create_dir_all(&tmp_restore_staging).expect("tmp restore dir");
        std::fs::write(tmp_restore_staging.join("restore.vz"), b"restore").expect("restore file");
        std::fs::create_dir_all(&cache_root).expect("cache dir");
        std::fs::write(cache_root.join("layer"), b"cache").expect("cache file");

        let args = ClearArgs {
            state: true,
            cache: true,
            benchmarks: false,
            all: false,
            legacy_tmp: true,
            dry_run: false,
            yes: true,
            state_root: Some(state_root.clone()),
            cache_root: Some(cache_root.clone()),
            benchmark_root: None,
            tmp_root: Some(tmp_root.clone()),
            older_than: None,
        };
        let mut output = Vec::new();

        run_clear(&args, &mut output).expect("clear runs");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("clear=deleted"));
        assert!(output.contains("root=firkin_state kind=state"));
        assert!(output.contains("root=firkin_cache kind=cache"));
        assert!(!state_continuations.exists());
        assert!(!tmp_restore_staging.exists());
        assert!(!cache_root.exists());
    }

    #[test]
    fn clear_refuses_dangerous_roots() {
        let root = ClearRoot {
            label: "unsafe",
            path: PathBuf::from("/"),
            kind: ClearRootKind::State,
        };

        let error = clear_root_report(&root, false, None).expect_err("unsafe root refused");

        assert!(
            error
                .to_string()
                .contains("refusing to clear unsafe Firkin root")
        );
    }

    #[test]
    fn benchmark_doctor_host_only_checks_writable_roots() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let args = BenchmarkDoctorArgs {
            mode: BenchmarkMode::HostOnly,
            state_root: Some(tempdir.path().join("state")),
            cache_root: Some(tempdir.path().join("cache")),
            benchmark_root: Some(tempdir.path().join("benchmarks")),
            min_free_bytes: 0,
        };
        let mut output = Vec::new();

        write_benchmark_doctor(&args, &mut output).expect("doctor passes");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("benchmark_doctor=started mode=host-only"));
        assert!(output.contains("check=state_root"));
        assert!(output.contains("check=cache_root"));
        assert!(output.contains("check=benchmark_root"));
        assert!(output.contains("benchmark_doctor=passed mode=host-only"));
    }

    fn heartbeat_seconds_ago(age_seconds: u64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("now after epoch")
            .as_secs();
        now.saturating_sub(age_seconds).to_string()
    }

    fn write_active_vm_marker(root: &Path, id: &str, age_seconds: u64, pid: u32) {
        let marker = root.join(id);
        std::fs::create_dir_all(&marker).expect("active vm marker");
        std::fs::write(marker.join("heartbeat"), heartbeat_seconds_ago(age_seconds))
            .expect("heartbeat");
        std::fs::write(marker.join("runtime.pid"), pid.to_string()).expect("pid");
        std::fs::write(marker.join("runtime.executable"), "/bin/fk").expect("executable");
    }

    #[derive(Clone)]
    struct RecordingHostProcessTerminator {
        terminated: Arc<Mutex<Vec<u32>>>,
    }

    impl firkin_runtime::HostProcessTerminator for RecordingHostProcessTerminator {
        type Error = std::convert::Infallible;

        fn terminate_process(
            &mut self,
            request: &firkin_runtime::HostProcessTerminationRequest,
        ) -> Result<(), Self::Error> {
            self.terminated
                .lock()
                .expect("terminated lock")
                .push(request.pid());
            Ok(())
        }
    }

    #[test]
    fn parses_run_command_with_trailing_args() {
        let cli = Cli::parse_from(["fk", "run", "busybox", "--", "/bin/echo", "hi"]);
        let Command::Run(args) = cli.command else {
            panic!("expected run command");
        };

        assert_eq!(args.image, "busybox");
        assert_eq!(args.command, ["/bin/echo", "hi"]);
    }

    #[test]
    fn parses_debug_preflight() {
        let cli = Cli::parse_from(["fk", "debug", "preflight"]);
        assert!(matches!(
            cli.command,
            Command::Debug {
                command: DebugCommand::Preflight
            }
        ));
    }

    #[test]
    fn formats_runtime_capabilities_for_preflight() {
        let mut output = Vec::new();
        write_runtime_capabilities(&mut output, firkin::apple_local_runtime_capabilities())
            .expect("capabilities");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("runtime_backend=apple-vz"));
        assert!(output.contains("supported_capability=create-linux-container"));
        assert!(output.contains("unsupported_capability=e2b-network-policy reason="));
        assert!(output.contains("unsupported_capability=domain-host-proxy reason="));
    }

    #[test]
    fn parses_debug_boot() {
        let cli = Cli::parse_from([
            "fk",
            "debug",
            "boot",
            "--kernel",
            "/tmp/vmlinux",
            "--boot-log",
            "/tmp/fk.log",
            "--vmnet-subnet",
            "192.168.126.0/24",
            "--hold-secs",
            "1",
        ]);
        let Command::Debug {
            command: DebugCommand::Boot(args),
        } = cli.command
        else {
            panic!("expected debug boot command");
        };

        assert_eq!(args.kernel, Some(PathBuf::from("/tmp/vmlinux")));
        assert_eq!(args.boot_log, Some(PathBuf::from("/tmp/fk.log")));
        assert_eq!(args.vmnet_subnet.as_deref(), Some("192.168.126.0/24"));
        assert_eq!(args.hold_secs, 1);
    }

    #[test]
    fn parses_e2b_host_with_explicit_state() {
        let cli = Cli::parse_from([
            "fk",
            "e2b",
            "host",
            "--control-addr",
            "127.0.0.1:4100",
            "--proxy-addr",
            "127.0.0.1:4101",
            "--domain",
            "cube.localhost",
            "--root",
            "/tmp/firkin-e2b/root",
            "--state",
            "/tmp/firkin-e2b/state.json",
            "--skip-domain-preflight",
            "--api-key",
            "local-key",
            "--lifecycle-interval-seconds",
            "15",
            "--proxy-tls-cert",
            "/tmp/firkin-e2b/cert.pem",
            "--proxy-tls-key",
            "/tmp/firkin-e2b/key.pem",
        ]);
        let Command::E2b {
            command: E2bCommand::Host(args),
        } = cli.command
        else {
            panic!("expected e2b host command");
        };

        assert_eq!(
            args.control_addr,
            "127.0.0.1:4100".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            args.proxy_addr,
            "127.0.0.1:4101".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(args.domain, "cube.localhost");
        assert_eq!(args.root, Some(PathBuf::from("/tmp/firkin-e2b/root")));
        assert_eq!(
            args.state,
            Some(PathBuf::from("/tmp/firkin-e2b/state.json"))
        );
        assert!(args.skip_domain_preflight);
        assert_eq!(args.api_key.as_deref(), Some("local-key"));
        assert_eq!(args.lifecycle_interval_seconds, 15);
        assert_eq!(
            args.proxy_tls_cert,
            Some(PathBuf::from("/tmp/firkin-e2b/cert.pem"))
        );
        assert_eq!(
            args.proxy_tls_key,
            Some(PathBuf::from("/tmp/firkin-e2b/key.pem"))
        );
    }

    struct RecordingDiskProbe {
        available: firkin::types::Size,
        probed_paths: Vec<PathBuf>,
    }

    impl firkin_runtime::DiskPressureProbe for RecordingDiskProbe {
        type Error = &'static str;

        fn available_disk(
            &mut self,
            path: &std::path::Path,
        ) -> Result<firkin::types::Size, Self::Error> {
            self.probed_paths.push(path.to_path_buf());
            Ok(self.available)
        }
    }

    #[test]
    fn derives_e2b_log_root_next_to_state_file() {
        let fallback = PathBuf::from("/tmp/firkin/e2b");

        assert_eq!(
            e2b_log_root(
                std::path::Path::new("/tmp/firkin/e2b/state.json"),
                &fallback
            ),
            PathBuf::from("/tmp/firkin/e2b/logs")
        );
        assert_eq!(
            e2b_log_root(std::path::Path::new("state.json"), &fallback),
            PathBuf::from("/tmp/firkin/e2b/logs")
        );
    }

    #[test]
    fn e2b_host_runtime_preflight_checks_roots_and_disk_floor() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let root = tempdir.path().join("sandboxes");
        let log_root = tempdir.path().join("logs");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&log_root).expect("logs");
        let mut probe = RecordingDiskProbe {
            available: firkin::types::Size::gib(32),
            probed_paths: Vec::new(),
        };

        preflight_e2b_host_runtime_roots_with_disk_probe(&root, &log_root, &mut probe)
            .expect("runtime preflight");

        assert_eq!(probe.probed_paths, vec![root, log_root]);
    }

    #[test]
    fn parses_benchmark_targets() {
        let cli = Cli::parse_from(["fk", "benchmark", "targets"]);
        assert!(matches!(
            cli.command,
            Command::Benchmark {
                command: BenchmarkCommand::Targets
            }
        ));
    }

    #[test]
    fn parses_benchmark_catalog() {
        let cli = Cli::parse_from(["fk", "benchmark", "catalog"]);
        assert!(matches!(
            cli.command,
            Command::Benchmark {
                command: BenchmarkCommand::Catalog
            }
        ));
    }

    #[test]
    fn parses_benchmark_autoscale_contract() {
        let cli = Cli::parse_from(["fk", "benchmark", "autoscale-contract"]);
        assert!(matches!(
            cli.command,
            Command::Benchmark {
                command: BenchmarkCommand::AutoscaleContract
            }
        ));
    }

    #[test]
    fn parses_benchmark_coverage() {
        let cli = Cli::parse_from(["fk", "benchmark", "coverage"]);
        assert!(matches!(
            cli.command,
            Command::Benchmark {
                command: BenchmarkCommand::Coverage(_)
            }
        ));
    }

    #[test]
    fn parses_benchmark_memory_attribution() {
        let cli = Cli::parse_from(["fk", "benchmark", "memory-attribution"]);
        assert!(matches!(
            cli.command,
            Command::Benchmark {
                command: BenchmarkCommand::MemoryAttribution
            }
        ));
    }

    #[test]
    fn parses_config_show() {
        let cli = Cli::parse_from(["fk", "config", "show", "--state-root", "/tmp/firkin-state"]);
        let Command::Config {
            command: ConfigCommand::Show(args),
        } = cli.command
        else {
            panic!("expected config show command");
        };

        assert_eq!(args.state_root, Some(PathBuf::from("/tmp/firkin-state")));
    }

    #[test]
    fn formats_config_show_with_explicit_roots() {
        let args = ConfigShowArgs {
            state_root: Some(PathBuf::from("/tmp/firkin-state")),
            cache_root: Some(PathBuf::from("/tmp/firkin-cache")),
            benchmark_root: Some(PathBuf::from("/tmp/firkin-benchmarks")),
        };
        let mut output = Vec::new();

        write_config_show(&args, &mut output).expect("config show");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("config=firkin-storage-v1"));
        assert!(output.contains("state_root=/tmp/firkin-state"));
        assert!(output.contains("cache_root=/tmp/firkin-cache"));
        assert!(output.contains("benchmark_root=/tmp/firkin-benchmarks"));
    }

    #[test]
    fn parses_clear_explicit_scopes() {
        let cli = Cli::parse_from([
            "fk",
            "clear",
            "--state",
            "--cache",
            "--benchmarks",
            "--dry-run",
            "--older-than",
            "24h",
        ]);
        let Command::Clear(args) = cli.command else {
            panic!("expected clear command");
        };

        assert!(args.state);
        assert!(args.cache);
        assert!(args.benchmarks);
        assert!(args.dry_run);
        assert_eq!(
            args.older_than.map(ClearOlderThan::as_duration),
            Some(Duration::from_hours(24))
        );
    }

    #[test]
    fn parses_benchmark_run_baseline_compare_proof_and_sprint_ready() {
        let run = Cli::parse_from([
            "fk",
            "benchmark",
            "run",
            "agent-core",
            "--mode",
            "signed-live",
            "--duration",
            "30s",
            "--out",
            "/tmp/current.json",
            "--no-build",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::Run(run_args),
        } = run.command
        else {
            panic!("expected benchmark run command");
        };
        assert!(run_args.no_build);

        let baseline = Cli::parse_from([
            "fk",
            "benchmark",
            "baseline",
            "save",
            "/tmp/current.json",
            "--name",
            "local-agent-core",
        ]);
        assert!(matches!(
            baseline.command,
            Command::Benchmark {
                command: BenchmarkCommand::Baseline { .. }
            }
        ));

        let compare = Cli::parse_from([
            "fk",
            "benchmark",
            "compare",
            "/tmp/baseline.json",
            "/tmp/current.json",
            "--rank",
            "bottlenecks",
        ]);
        assert!(matches!(
            compare.command,
            Command::Benchmark {
                command: BenchmarkCommand::Compare(_)
            }
        ));

        let proof = Cli::parse_from([
            "fk",
            "benchmark",
            "proof",
            "m1",
            "--from",
            "/tmp/proof.txt",
            "--out",
            "/tmp/proof.html",
        ]);
        assert!(matches!(
            proof.command,
            Command::Benchmark {
                command: BenchmarkCommand::Proof(_)
            }
        ));

        let ready = Cli::parse_from([
            "fk",
            "benchmark",
            "sprint-ready",
            "--suite",
            "agent-core",
            "--baseline",
            "local-agent-core",
            "--current-artifact",
            "/tmp/current.json",
            "--overhead-artifact",
            "/tmp/overhead.json",
            "--scorecard-artifact",
            "/tmp/scorecard.json",
        ]);
        assert!(matches!(
            ready.command,
            Command::Benchmark {
                command: BenchmarkCommand::SprintReady(_)
            }
        ));
    }

    #[test]
    fn parses_benchmark_iteration_commands() {
        let p0_contract = Cli::parse_from(["fk", "benchmark", "p0-contract"]);
        assert!(matches!(
            p0_contract.command,
            Command::Benchmark {
                command: BenchmarkCommand::P0Contract
            }
        ));

        let phase_owners = Cli::parse_from(["fk", "benchmark", "phase-owners"]);
        assert!(matches!(
            phase_owners.command,
            Command::Benchmark {
                command: BenchmarkCommand::PhaseOwners
            }
        ));

        let metric_contract = Cli::parse_from(["fk", "benchmark", "metric-contract"]);
        assert!(matches!(
            metric_contract.command,
            Command::Benchmark {
                command: BenchmarkCommand::MetricContract
            }
        ));

        let sprint_record = Cli::parse_from([
            "fk",
            "benchmark",
            "sprint-record",
            "--suite",
            "agent-core",
            "--baseline",
            "local-agent-core",
            "--current-artifact",
            "/tmp/current.json",
            "--overhead-artifact",
            "/tmp/overhead.json",
            "--scorecard-artifact",
            "/tmp/scorecard.json",
            "--out",
            "/tmp/sprint.md",
        ]);
        assert!(matches!(
            sprint_record.command,
            Command::Benchmark {
                command: BenchmarkCommand::SprintRecord(_)
            }
        ));
    }

    #[test]
    fn parses_benchmark_validate_soak() {
        let cli = Cli::parse_from(["fk", "benchmark", "validate-soak", "/tmp/soak.json"]);
        let Command::Benchmark {
            command: BenchmarkCommand::ValidateSoak(args),
        } = cli.command
        else {
            panic!("expected benchmark validate soak command");
        };

        assert_eq!(args.artifact, PathBuf::from("/tmp/soak.json"));
    }

    #[test]
    fn parses_benchmark_suites_with_optional_suite_id() {
        let cli = Cli::parse_from(["fk", "benchmark", "suites", "agent-core"]);
        let Command::Benchmark {
            command: BenchmarkCommand::Suites(args),
        } = cli.command
        else {
            panic!("expected benchmark suites command");
        };

        assert_eq!(args.suite.as_deref(), Some("agent-core"));
    }

    #[test]
    fn formats_benchmark_catalog() {
        let mut output = Vec::new();
        write_benchmark_catalog(&mut output).expect("benchmark catalog");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("manifest=firkin-benchmark-metric-catalog-v1"));
        assert!(output.contains("metric=start.agent_task_ready_ms group=agent_task"));
        assert!(output.contains("metric=product.agent_computer_ready_ms group=product"));
        assert!(output.contains("metric=disk.sparse_bloat_after_trim group=disk"));
        assert!(output.contains("requirement=p0_dashboard"));
        assert!(output.contains("requirement=autoscale_dashboard"));
    }

    #[test]
    fn p0_contract_matches_scorecard_and_agent_core_suite() {
        let mut output = Vec::new();
        write_benchmark_p0_contract(&mut output).expect("p0 contract");
        let output = String::from_utf8(output).expect("utf8");
        let output_metrics = output_metric_names(&output);
        let p0_metrics = firkin::evidence::P0_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        let suite_metrics = firkin::benchmark::benchmark_suite("agent-core")
            .expect("agent-core suite")
            .cases
            .iter()
            .map(|case| case.metric)
            .collect::<std::collections::BTreeSet<_>>();

        assert!(output.contains("manifest=firkin-benchmark-p0-contract-v1"));
        assert_eq!(output_metrics, p0_metrics);
        assert_eq!(suite_metrics, p0_metrics);
        assert!(output.contains("metric=start.agent_task_ready_ms group=agent_task"));
        assert!(output.contains("case=agent_task_ready"));
        assert!(output.contains("source=event_trace:start.agent_task_ready_ms"));
    }

    #[test]
    fn autoscale_contract_matches_scorecard_and_coverage() {
        let mut output = Vec::new();
        write_benchmark_autoscale_contract(&mut output).expect("autoscale contract");
        let output = String::from_utf8(output).expect("utf8");
        let output_metrics = output_metric_names(&output);
        let autoscale_metrics = firkin::evidence::AUTOSCALE_EFFICIENCY_SCORECARD_METRICS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();

        assert!(output.contains("manifest=firkin-benchmark-autoscale-contract-v1"));
        assert_eq!(output_metrics, autoscale_metrics);
        assert!(output.contains("metric=product.agent_computer_ready_ms group=product"));
        assert!(output.contains("requirement=autoscale_dashboard"));
        assert!(output.contains("source=autoscale_trace:product.agent_computer_ready_ms"));
        assert!(output.contains("status=unit_validated_only"));
        assert!(output.contains("metric=autoscale.ready_queue_hit_rate_pct group=autoscale"));
        assert!(!output.contains("status=needs_live_harness"));
        assert!(output.contains("owner=firkin-benchmark/firkin-runtime"));
        assert!(output.contains("metric=cleanup.leftover_bytes group=cleanup"));
        assert!(output.contains("status=signed_live_exact"));
    }

    #[test]
    fn benchmark_run_wires_autoscale_signed_live_harness() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let args = BenchmarkRunArgs {
            suite: "autoscale".to_owned(),
            mode: BenchmarkMode::SignedLive,
            duration: BenchmarkDuration(Duration::from_secs(30)),
            out: tempdir.path().join("autoscale.json"),
            no_build: false,
        };
        let mut command = ProcessCommand::new("scripts/run-signed-live-runtime-test.sh");

        configure_signed_live_benchmark_command(&args, 2, &mut command).expect("autoscale command");
        let command_args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let command_env = command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect::<std::collections::BTreeMap<_, _>>();

        assert_eq!(
            command_args,
            vec!["live_runtime_autoscale_scorecard_writes_product_path_artifact"]
        );
        assert_eq!(
            command_env.get("FIRKIN_LIVE_AUTOSCALE_REPEATS"),
            Some(&"2".to_owned())
        );
        assert_eq!(
            command_env.get("FIRKIN_LIVE_AUTOSCALE_ARTIFACT"),
            Some(&args.out.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn metric_contract_prints_decision_grade_endpoints_without_legacy_names() {
        let mut output = Vec::new();
        write_benchmark_metric_contract(&mut output).expect("metric contract");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("manifest=firkin-decision-grade-metric-contract-v1"));
        assert!(output.contains("metric=start.hot_to_first_stdout_ms"));
        assert!(output.contains("start=PoolLeaseAcquired"));
        assert!(output.contains("end=FirstStdoutByte"));
        assert!(output.contains("lifecycle=hot"));
        assert!(output.contains("workload=tiny_exec"));
        assert!(output.contains("p95_min_samples=100"));
        assert!(output.contains("p99_min_samples=500"));

        for legacy in [
            "sandbox.start.hot_pool_checkout_ms",
            "command_start",
            "first_stdout_byte",
            "ready_probe",
            "warm_pool_checkout",
            "sandbox.density.max_active_before_p95_doubles",
        ] {
            assert!(
                !output
                    .lines()
                    .any(|line| line.strip_prefix("metric=").is_some_and(|rest| {
                        rest.split_once(' ').map_or(rest, |(metric, _)| metric) == legacy
                    })),
                "legacy metric leaked from metric contract: {legacy}"
            );
        }
    }

    #[test]
    fn phase_owners_print_exact_prefix_and_fallback_rules() {
        let mut output = Vec::new();
        write_benchmark_phase_owners(&mut output).expect("phase owners");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("manifest=firkin-benchmark-phase-owners-v1"));
        assert!(output.contains("match=agent_task_ready_ms phase=agent_task_ready"));
        assert!(output.contains("match=sandbox.disk. phase=disk"));
        assert!(output.contains("match=* phase=benchmark"));
        assert_eq!(
            firkin::evidence::benchmark_metric_ownership("agent_task_ready_ms").phase_label,
            "agent_task_ready"
        );
        assert_eq!(
            firkin::evidence::benchmark_metric_ownership("sandbox.disk.fsync_p99_us").owner,
            "firkin-oci/firkin-template/firkin-core"
        );
        assert_eq!(
            firkin::evidence::benchmark_metric_ownership("unknown.metric").phase_label,
            "benchmark"
        );
    }

    #[test]
    fn formats_benchmark_coverage() {
        let mut output = Vec::new();
        write_benchmark_coverage(
            &BenchmarkCoverageArgs {
                strict: false,
                artifact: Vec::new(),
            },
            &mut output,
        )
        .expect("benchmark coverage");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("manifest=firkin-benchmark-measurement-coverage-v1"));
        assert!(output.contains("p0_metrics=13"));
        assert!(output.contains("metric=start.hot_to_first_stdout_ms status=signed_live_exact"));
        assert!(
            output.contains("metric=exec.direct_first_stdout_byte_ms status=signed_live_exact")
        );
        assert!(output.contains("metric=disk.sparse_bloat_after_trim status=signed_live_exact"));
    }

    #[test]
    fn formats_benchmark_memory_attribution_blocker() {
        let mut output = Vec::new();
        write_benchmark_memory_attribution(&mut output).expect("memory attribution");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("manifest=firkin-benchmark-memory-attribution-v1"));
        assert!(output.contains("status=blocked"));
        assert!(output.contains(firkin::benchmark::EXACT_MEMORY_ATTRIBUTION_COLLECTOR_REQUIREMENT));
        assert!(output.contains("no per-VM resident footprint"));
        assert!(output.contains("exclusive com.apple.Virtualization.VirtualMachine task set"));
        assert!(output.contains("metric=sandbox.mem.idle_host_footprint_bytes"));
    }

    #[test]
    fn strict_benchmark_coverage_fails_until_p0_metrics_are_exact() {
        let mut output = Vec::new();
        let error = write_benchmark_coverage(
            &BenchmarkCoverageArgs {
                strict: true,
                artifact: Vec::new(),
            },
            &mut output,
        )
        .expect_err("strict coverage should fail");
        let output = String::from_utf8(output).expect("utf8");

        assert!(
            error
                .to_string()
                .contains("strict benchmark coverage failed"),
            "unexpected error: {error}; output:\n{output}"
        );
        assert!(output.contains("strict=true artifact=- artifact_kind=-"));
    }

    #[test]
    fn benchmark_coverage_reads_source_metrics_across_artifacts() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let lifecycle = tempdir.path().join("current.json");
        let overhead = tempdir.path().join("overhead.json");
        let scorecard = tempdir.path().join("scorecard.json");
        write_lifecycle_artifact_with_value(&lifecycle, 1.0);
        write_overhead_artifact_with_value(&overhead, 0.1);
        write_scorecard_artifact_with_values(&scorecard, [1.0, 2.0]);
        let mut output = Vec::new();

        write_benchmark_coverage(
            &BenchmarkCoverageArgs {
                strict: false,
                artifact: vec![lifecycle, overhead, scorecard],
            },
            &mut output,
        )
        .expect("coverage");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("artifact_kind=lifecycle,overhead,scorecard"));
        assert!(output.contains("metric=start.agent_task_ready_ms status=signed_live_exact"));
        assert!(output.contains(
            "artifact_metric=start.agent_task_ready_ms artifact_status=present artifact_count=2 artifact_kind=scorecard"
        ));
        assert!(
            output.contains("metric=exec.direct_first_stdout_byte_ms status=signed_live_exact")
        );
    }

    #[test]
    fn benchmark_coverage_reads_event_trace_metrics_from_scorecard_artifacts() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let lifecycle = tempdir.path().join("current.json");
        let overhead = tempdir.path().join("overhead.json");
        let scorecard = tempdir.path().join("scorecard.json");
        write_lifecycle_artifact_with_value(&lifecycle, 1.0);
        write_overhead_artifact_with_value(&overhead, 0.1);
        write_scorecard_artifact_with_values(&scorecard, [0.25]);
        let mut output = Vec::new();

        write_benchmark_coverage(
            &BenchmarkCoverageArgs {
                strict: false,
                artifact: vec![lifecycle, overhead, scorecard],
            },
            &mut output,
        )
        .expect("coverage");
        let output = String::from_utf8(output).expect("utf8");

        assert!(
            output.contains(
                "metric=start.hot_to_first_stdout_ms status=signed_live_exact source=event_trace:start.hot_to_first_stdout_ms artifact_metric=start.hot_to_first_stdout_ms artifact_status=present artifact_count=1 artifact_kind=lifecycle"
            ),
            "unexpected output:\n{output}"
        );
        assert!(
            output.contains(
                "metric=disk.sparse_bloat_after_trim status=signed_live_exact source=event_trace:disk.sparse_bloat_after_trim artifact_metric=disk.sparse_bloat_after_trim artifact_status=present artifact_count=1 artifact_kind=scorecard"
            )
        );
    }

    #[test]
    fn benchmark_coverage_reads_shared_guardrails_from_autoscale_scorecard_artifacts() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let lifecycle = tempdir.path().join("current.json");
        let overhead = tempdir.path().join("overhead.json");
        let autoscale_scorecard = tempdir.path().join("autoscale-scorecard.json");
        write_lifecycle_artifact_with_value(&lifecycle, 1.0);
        write_overhead_artifact_with_value(&overhead, 0.1);
        write_autoscale_scorecard_artifact_with_values(&autoscale_scorecard, [0.0, 0.0]);
        let mut output = Vec::new();

        write_benchmark_coverage(
            &BenchmarkCoverageArgs {
                strict: false,
                artifact: vec![lifecycle, overhead, autoscale_scorecard],
            },
            &mut output,
        )
        .expect("coverage");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("artifact_kind=lifecycle,overhead,autoscale_scorecard"));
        assert!(output.contains(
            "metric=cleanup.leftover_bytes status=signed_live_exact source=event_trace:cleanup.leftover_bytes artifact_metric=cleanup.leftover_bytes artifact_status=present artifact_count=2 artifact_kind=autoscale_scorecard"
        ));
        assert!(output.contains(
            "metric=reliability.unknown_failure_rate status=signed_live_exact source=event_trace:reliability.unknown_failure_rate artifact_metric=reliability.unknown_failure_rate artifact_status=present artifact_count=2 artifact_kind=autoscale_scorecard"
        ));
    }

    #[test]
    fn strict_benchmark_coverage_rejects_low_sample_p95_artifacts() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let lifecycle = tempdir.path().join("current.json");
        let overhead = tempdir.path().join("overhead.json");
        let scorecard = tempdir.path().join("scorecard.json");
        write_lifecycle_artifact_with_values(&lifecycle, [1.0, 2.0, 3.0]);
        write_overhead_artifact_with_value(&overhead, 0.1);
        write_scorecard_artifact_with_values(&scorecard, [1.0, 2.0, 3.0]);
        let mut output = Vec::new();

        let error = write_benchmark_coverage(
            &BenchmarkCoverageArgs {
                strict: true,
                artifact: vec![lifecycle, overhead, scorecard],
            },
            &mut output,
        )
        .expect_err("strict coverage must reject unstable p95 samples");
        let output = String::from_utf8(output).expect("utf8");

        assert!(
            error
                .to_string()
                .contains("strict benchmark coverage failed"),
            "unexpected error: {error}; output:\n{output}"
        );
        assert!(output.contains("metric=start.hot_to_first_stdout_ms"));
        assert!(output.contains("confidence=superfast_iteration"));
        assert!(output.contains("p95_min_samples=100"));
        assert!(output.contains("p95_status=collect_more_samples"));
    }

    #[test]
    fn benchmark_sprint_ready_fails_honestly_when_required_p0_is_missing_or_proxy() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let baseline_artifact = tempdir.path().join("baseline.json");
        let current_artifact = tempdir.path().join("current.json");
        let overhead_artifact = tempdir.path().join("overhead.json");
        let benchmark_root = tempdir.path().join("benchmarks");
        write_lifecycle_artifact_with_value(&baseline_artifact, 10.0);
        write_lifecycle_artifact_with_value(&current_artifact, 10.0);
        write_overhead_artifact_with_value(&overhead_artifact, 0.1);
        save_benchmark_baseline(
            &BenchmarkBaselineSaveArgs {
                artifact: baseline_artifact,
                name: "local_agent_core".to_owned(),
                benchmark_root: Some(benchmark_root.clone()),
            },
            Vec::new(),
        )
        .expect("save baseline");
        let mut output = Vec::new();

        let error = write_benchmark_sprint_ready(
            &BenchmarkSprintReadyArgs {
                suite: "agent-core".to_owned(),
                baseline: "local_agent_core".to_owned(),
                mode: BenchmarkMode::HostOnly,
                current_artifact: Some(current_artifact),
                overhead_artifact: Some(overhead_artifact),
                scorecard_artifact: None,
                benchmark_root: Some(benchmark_root),
                min_free_bytes: 0,
            },
            &mut output,
        )
        .expect_err("sprint-ready must fail until exact P0 coverage exists");
        let output = String::from_utf8(output).expect("utf8");

        assert!(
            error
                .to_string()
                .contains("strict benchmark coverage failed"),
            "unexpected error: {error}; output:\n{output}"
        );
        assert!(output.contains("benchmark_slo_gate=passed kind=overhead"));
        assert!(output.contains("strict=true"));
        assert!(output.contains("artifact_kind=lifecycle,overhead"));
        assert!(output.contains("start.hot_to_first_stdout_ms"));
        assert!(output.contains("disk.sparse_bloat_after_trim"));
        assert!(output.contains("benchmark_compare=summary rank=bottlenecks"));
        assert!(output.contains("sprint-ready=blocked suite=agent-core"));
        assert!(output.contains("first_command=\"fk benchmark run agent-core --mode host-only --duration 30s --out target/firkin-live-evidence/current-30s.json\""));
        assert!(!output.contains("sprint-ready=passed"));
    }

    #[test]
    fn benchmark_sprint_ready_reports_doctor_blockers_with_next_command() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let baseline_artifact = tempdir.path().join("baseline.json");
        let benchmark_root = tempdir.path().join("benchmarks");
        write_lifecycle_artifact_with_value(&baseline_artifact, 10.0);
        save_benchmark_baseline(
            &BenchmarkBaselineSaveArgs {
                artifact: baseline_artifact,
                name: "local_agent_core".to_owned(),
                benchmark_root: Some(benchmark_root.clone()),
            },
            Vec::new(),
        )
        .expect("save baseline");
        let mut output = Vec::new();

        let error = write_benchmark_sprint_ready(
            &BenchmarkSprintReadyArgs {
                suite: "agent-core".to_owned(),
                baseline: "local_agent_core".to_owned(),
                mode: BenchmarkMode::HostOnly,
                current_artifact: None,
                overhead_artifact: None,
                scorecard_artifact: None,
                benchmark_root: Some(benchmark_root),
                min_free_bytes: u64::MAX,
            },
            &mut output,
        )
        .expect_err("sprint-ready must fail when doctor fails");
        let output = String::from_utf8(output).expect("utf8");

        assert!(
            error.to_string().contains("benchmark doctor failed"),
            "unexpected error: {error}; output:\n{output}"
        );
        assert!(output.contains("check=disk_free"));
        assert!(output.contains("ok=false"));
        assert!(output.contains("sprint-ready=blocked suite=agent-core"));
        assert!(output.contains("first_command=\"fk benchmark run agent-core --mode host-only --duration 30s --out target/firkin-live-evidence/current-30s.json\""));
        assert!(!output.contains("sprint-ready=passed"));
    }

    #[test]
    fn benchmark_sprint_record_writes_markdown_for_passing_loop() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let baseline_artifact = tempdir.path().join("baseline.json");
        let current_artifact = tempdir.path().join("current.json");
        let overhead_artifact = tempdir.path().join("overhead.json");
        let scorecard_artifact = tempdir.path().join("scorecard.json");
        let out = tempdir.path().join("sprint.md");
        let benchmark_root = tempdir.path().join("benchmarks");
        write_lifecycle_artifact_with_values(&baseline_artifact, (1..=100_u32).map(f64::from));
        write_lifecycle_artifact_with_values(&current_artifact, (2..=101_u32).map(f64::from));
        write_overhead_artifact_with_value(&overhead_artifact, 0.1);
        write_scorecard_artifact_with_values(&scorecard_artifact, (1..=100_u32).map(f64::from));
        save_benchmark_baseline(
            &BenchmarkBaselineSaveArgs {
                artifact: baseline_artifact,
                name: "local_agent_core".to_owned(),
                benchmark_root: Some(benchmark_root.clone()),
            },
            Vec::new(),
        )
        .expect("save baseline");
        let mut output = Vec::new();

        write_benchmark_sprint_record(
            &BenchmarkSprintRecordArgs {
                suite: "agent-core".to_owned(),
                baseline: "local_agent_core".to_owned(),
                mode: BenchmarkMode::HostOnly,
                current_artifact: current_artifact.clone(),
                overhead_artifact: overhead_artifact.clone(),
                scorecard_artifact: Some(scorecard_artifact.clone()),
                out: out.clone(),
                benchmark_root: Some(benchmark_root),
                min_free_bytes: 0,
            },
            &mut output,
        )
        .expect("sprint record");
        let output = String::from_utf8(output).expect("utf8");
        let markdown = std::fs::read_to_string(out).expect("markdown");

        assert!(output.contains("sprint_record=written status=passed"));
        assert!(markdown.contains("status: passed"));
        assert!(markdown.contains(
            "strict_coverage: `cargo run -q -p firkin-cli -- benchmark coverage --strict"
        ));
        assert!(markdown.contains("## Strict Coverage"));
        assert!(markdown.contains("## Compare"));
        assert!(markdown.contains("## Sprint Ready"));
        assert!(markdown.contains("sprint-ready=passed suite=agent-core"));
        assert!(markdown.contains("top_bottleneck:"));
        assert!(markdown.contains("confidence:"));
        assert!(markdown.contains("residual_risks:"));
        assert!(markdown.contains("next_30s_command: `fk benchmark run agent-core --mode host-only --duration 30s --out target/firkin-live-evidence/current-30s.json`"));
        assert!(markdown.contains(&current_artifact.display().to_string()));
        assert!(markdown.contains(&overhead_artifact.display().to_string()));
        assert!(markdown.contains(&scorecard_artifact.display().to_string()));
    }

    #[test]
    fn benchmark_sprint_record_rejects_missing_required_artifact_before_write() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let baseline_artifact = tempdir.path().join("baseline.json");
        let overhead_artifact = tempdir.path().join("overhead.json");
        let benchmark_root = tempdir.path().join("benchmarks");
        let out = tempdir.path().join("sprint.md");
        write_lifecycle_artifact_with_value(&baseline_artifact, 5.0);
        write_overhead_artifact_with_value(&overhead_artifact, 0.1);
        save_benchmark_baseline(
            &BenchmarkBaselineSaveArgs {
                artifact: baseline_artifact,
                name: "local_agent_core".to_owned(),
                benchmark_root: Some(benchmark_root.clone()),
            },
            Vec::new(),
        )
        .expect("save baseline");

        let error = write_benchmark_sprint_record(
            &BenchmarkSprintRecordArgs {
                suite: "agent-core".to_owned(),
                baseline: "local_agent_core".to_owned(),
                mode: BenchmarkMode::HostOnly,
                current_artifact: tempdir.path().join("missing-current.json"),
                overhead_artifact,
                scorecard_artifact: None,
                out: out.clone(),
                benchmark_root: Some(benchmark_root),
                min_free_bytes: 0,
            },
            Vec::new(),
        )
        .expect_err("missing current artifact must fail");

        assert!(error.to_string().contains("missing current artifact"));
        assert!(!out.exists());
    }

    #[test]
    fn formats_benchmark_suites() {
        let mut output = Vec::new();
        let args = BenchmarkSuitesArgs {
            suite: Some("agent-core".to_owned()),
        };
        write_benchmark_suites(&args, &mut output).expect("benchmark suites");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("manifest=firkin-benchmark-suites-v1 suites=1"));
        assert!(output.contains("suite=agent-core"));
        assert!(output.contains("case=agent_task_ready suite=agent-core"));
        assert!(output.contains("metric=start.agent_task_ready_ms"));
    }

    #[test]
    fn formats_benchmark_targets_manifest() {
        let mut output = Vec::new();
        write_benchmark_targets(&mut output).expect("benchmark targets");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("manifest=firkin-benchmark-targets-v1"));
        assert!(output.contains("target=start.hot_to_first_stdout_ms p50_ms=50 p95_ms=75"));
        assert!(output.contains("target=exec.direct_first_stdout_byte_ms p50_ms=20 p95_ms=35"));
        assert!(output.contains("target=start.resume_to_first_stdout_ms p50_ms=35 p95_ms=50"));
        assert!(output.contains("target=exec.batch_100_small_commands_ms"));
        for metric in firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS {
            assert!(output.contains(&format!("target={metric}")));
        }
        assert!(output.contains("overhead=control_plane_cpu_idle p95_percent=1"));
        assert!(output.contains("overhead=control_plane_rss_idle p95_mib=256"));
        assert!(output.contains("overhead=disk_metadata_growth p95_bytes=1048576"));
    }

    #[test]
    fn parses_benchmark_validate_lifecycle_slo() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "validate-lifecycle-slo",
            "/tmp/live-benchmark-evidence.json",
            "--min-samples",
            "3",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::ValidateLifecycleSlo(args),
        } = cli.command
        else {
            panic!("expected benchmark validate lifecycle slo command");
        };

        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/live-benchmark-evidence.json")
        );
        assert_eq!(args.min_samples, 3);
    }

    #[test]
    fn parses_benchmark_validate_overhead_slo() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "validate-overhead-slo",
            "/tmp/live-overhead-evidence.json",
            "--min-samples",
            "3",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::ValidateOverheadSlo(args),
        } = cli.command
        else {
            panic!("expected benchmark validate overhead slo command");
        };

        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/live-overhead-evidence.json")
        );
        assert_eq!(args.min_samples, 3);
    }

    #[test]
    fn parses_benchmark_write_scorecard() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "write-scorecard",
            "/tmp/samples.json",
            "/tmp/scorecard.json",
            "--min-samples",
            "3",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::WriteScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark write-scorecard command");
        };

        assert_eq!(args.samples, PathBuf::from("/tmp/samples.json"));
        assert_eq!(args.artifact, PathBuf::from("/tmp/scorecard.json"));
        assert_eq!(args.min_samples, 3);
    }

    #[test]
    fn parses_benchmark_write_autoscale_scorecard() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "write-autoscale-scorecard",
            "/tmp/autoscale-samples.json",
            "/tmp/autoscale-scorecard.json",
            "--min-samples",
            "3",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::WriteAutoscaleScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark write-autoscale-scorecard command");
        };

        assert_eq!(args.samples, PathBuf::from("/tmp/autoscale-samples.json"));
        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/autoscale-scorecard.json")
        );
        assert_eq!(args.min_samples, 3);
    }

    #[test]
    fn parses_benchmark_write_agent_computer_scorecard() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "write-agent-computer-scorecard",
            "/tmp/agent-computer-samples.json",
            "/tmp/agent-computer-scorecard.json",
            "--min-samples",
            "3",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::WriteAgentComputerScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark write-agent-computer-scorecard command");
        };

        assert_eq!(
            args.samples,
            PathBuf::from("/tmp/agent-computer-samples.json")
        );
        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/agent-computer-scorecard.json")
        );
        assert_eq!(args.min_samples, 3);
    }

    #[test]
    fn parses_benchmark_validate_scorecard() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "validate-scorecard",
            "/tmp/scorecard.json",
            "--min-samples",
            "3",
            "--require-snappy",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::ValidateScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark validate-scorecard command");
        };

        assert_eq!(args.artifact, PathBuf::from("/tmp/scorecard.json"));
        assert_eq!(args.min_samples, 3);
        assert!(args.require_snappy);
    }

    #[test]
    fn parses_benchmark_validate_autoscale_scorecard() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "validate-autoscale-scorecard",
            "/tmp/autoscale-scorecard.json",
            "--min-samples",
            "3",
            "--require-promotable",
            "--require-snappy",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::ValidateAutoscaleScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark validate-autoscale-scorecard command");
        };

        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/autoscale-scorecard.json")
        );
        assert_eq!(args.min_samples, 3);
        assert!(args.require_promotable);
        assert!(args.require_snappy);
    }

    #[test]
    fn parses_benchmark_validate_agent_computer_scorecard() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "validate-agent-computer-scorecard",
            "/tmp/agent-computer-scorecard.json",
            "--min-samples",
            "3",
            "--require-promotable",
            "--require-snappy",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::ValidateAgentComputerScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark validate-agent-computer-scorecard command");
        };

        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/agent-computer-scorecard.json")
        );
        assert_eq!(args.min_samples, 3);
        assert!(args.require_promotable);
        assert!(args.require_snappy);
    }

    #[test]
    fn parses_benchmark_report_scorecard() {
        let cli = Cli::parse_from(["fk", "benchmark", "report-scorecard", "/tmp/scorecard.json"]);
        let Command::Benchmark {
            command: BenchmarkCommand::ReportScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark report-scorecard command");
        };

        assert_eq!(args.artifact, PathBuf::from("/tmp/scorecard.json"));
    }

    #[test]
    fn parses_benchmark_report_autoscale_scorecard() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "report-autoscale-scorecard",
            "/tmp/autoscale-scorecard.json",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::ReportAutoscaleScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark report-autoscale-scorecard command");
        };

        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/autoscale-scorecard.json")
        );
    }

    #[test]
    fn parses_benchmark_report_agent_computer_scorecard() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "report-agent-computer-scorecard",
            "/tmp/agent-computer-scorecard.json",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::ReportAgentComputerScorecard(args),
        } = cli.command
        else {
            panic!("expected benchmark report-agent-computer-scorecard command");
        };

        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/agent-computer-scorecard.json")
        );
    }

    #[test]
    fn parses_benchmark_report_agent_computer_traces() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "report-agent-computer-traces",
            "/tmp/agent-computer-scorecard.traces.json",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::ReportAgentComputerTraces(args),
        } = cli.command
        else {
            panic!("expected benchmark report-agent-computer-traces command");
        };

        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/agent-computer-scorecard.traces.json")
        );
    }

    #[test]
    fn parses_benchmark_report() {
        let cli = Cli::parse_from([
            "fk",
            "benchmark",
            "report",
            "lifecycle",
            "/tmp/live-benchmark-evidence.json",
        ]);
        let Command::Benchmark {
            command: BenchmarkCommand::Report(args),
        } = cli.command
        else {
            panic!("expected benchmark report command");
        };

        assert_eq!(args.kind, BenchmarkReportKind::Lifecycle);
        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/live-benchmark-evidence.json")
        );

        let cli = Cli::parse_from(["fk", "benchmark", "report", "decision", "/tmp/current.json"]);
        let Command::Benchmark {
            command: BenchmarkCommand::Report(args),
        } = cli.command
        else {
            panic!("expected benchmark report command");
        };

        assert_eq!(args.kind, BenchmarkReportKind::Decision);
        assert_eq!(args.artifact, PathBuf::from("/tmp/current.json"));
    }

    #[test]
    fn parses_substrate_acceptance_checklist() {
        let cli = Cli::parse_from(["fk", "substrate", "acceptance-checklist"]);
        assert!(matches!(
            cli.command,
            Command::Substrate {
                command: SubstrateCommand::AcceptanceChecklist
            }
        ));
    }

    #[test]
    fn parses_substrate_validate_soak() {
        let cli = Cli::parse_from(["fk", "substrate", "validate-soak", "/tmp/soak.json"]);
        let Command::Substrate {
            command: SubstrateCommand::ValidateSoak(args),
        } = cli.command
        else {
            panic!("expected validate soak command");
        };

        assert_eq!(args.artifact, PathBuf::from("/tmp/soak.json"));
    }

    #[test]
    fn parses_substrate_snapshot_sidecars() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "snapshot-sidecars",
            "--artifact",
            "/tmp/firkin/snapshots/repo-main.vz",
            "--logical-id",
            "repo-main",
            "--kind",
            "base-template",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::SnapshotSidecars(args),
        } = cli.command
        else {
            panic!("expected snapshot sidecars command");
        };

        assert_eq!(
            args.artifact,
            PathBuf::from("/tmp/firkin/snapshots/repo-main.vz")
        );
        assert_eq!(args.logical_id, "repo-main");
        assert_eq!(args.kind, SnapshotSidecarKind::BaseTemplate);
    }

    #[test]
    fn substrate_snapshot_sidecars_write_manifest_and_integrity() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let artifact = tempdir.path().join("repo-main.vz");
        std::fs::write(&artifact, b"snapshot-bytes").expect("artifact");
        let args = SubstrateSnapshotSidecarsArgs {
            artifact: artifact.clone(),
            logical_id: "repo-main".to_owned(),
            kind: SnapshotSidecarKind::BaseTemplate,
        };
        let mut output = Vec::new();

        write_substrate_snapshot_sidecars(&args, &mut output).expect("snapshot sidecars");

        let manifest_path =
            firkin::substrate::SnapshotArtifactManifest::sidecar_path_for_artifact(&artifact);
        let integrity_path =
            firkin::substrate::SnapshotArtifactIntegrity::sidecar_path_for_artifact(&artifact);
        let manifest = firkin::substrate::SnapshotArtifactManifest::read_json(&manifest_path)
            .expect("manifest sidecar");
        let integrity = firkin::substrate::SnapshotArtifactIntegrity::read_json(&integrity_path)
            .expect("integrity sidecar");
        assert_eq!(
            manifest.kind(),
            firkin::substrate::SnapshotArtifactKind::BaseTemplate
        );
        assert_eq!(manifest.logical_id(), "repo-main");
        assert_eq!(manifest.path(), artifact.as_path());
        integrity.verify(&manifest).expect("integrity verifies");

        let output = String::from_utf8(output).expect("utf8");
        assert!(output.contains("snapshot_sidecars=written"));
        assert!(output.contains("kind=base_template"));
        assert!(output.contains("logical_id=repo-main"));
    }

    #[test]
    fn parses_substrate_hygiene_once() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "hygiene-once",
            "--snapshot-root",
            "/tmp/firkin/snapshots",
            "--log-root",
            "/tmp/firkin/logs",
            "--manifest-root",
            "/tmp/firkin/manifests",
            "--max-log-bytes",
            "4096",
            "--gzip-logs",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::HygieneOnce(args),
        } = cli.command
        else {
            panic!("expected hygiene-once command");
        };

        assert_eq!(args.snapshot_root, PathBuf::from("/tmp/firkin/snapshots"));
        assert_eq!(args.log_root, PathBuf::from("/tmp/firkin/logs"));
        assert_eq!(
            args.manifest_root,
            Some(PathBuf::from("/tmp/firkin/manifests"))
        );
        assert_eq!(args.max_log_bytes, 4096);
        assert!(args.gzip_logs);
    }

    #[test]
    fn parses_substrate_hygiene_daemon() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "hygiene-daemon",
            "--snapshot-root",
            "/tmp/firkin/snapshots",
            "--log-root",
            "/tmp/firkin/logs",
            "--manifest-root",
            "/tmp/firkin/manifests",
            "--max-log-bytes",
            "4096",
            "--interval-seconds",
            "30",
            "--gzip-logs",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::HygieneDaemon(args),
        } = cli.command
        else {
            panic!("expected hygiene-daemon command");
        };

        assert_eq!(args.snapshot_root, PathBuf::from("/tmp/firkin/snapshots"));
        assert_eq!(args.log_root, PathBuf::from("/tmp/firkin/logs"));
        assert_eq!(
            args.manifest_root,
            Some(PathBuf::from("/tmp/firkin/manifests"))
        );
        assert_eq!(args.max_log_bytes, 4096);
        assert_eq!(args.interval_seconds, 30);
        assert!(args.gzip_logs);
    }

    #[test]
    fn substrate_hygiene_launchd_plist_renders_daemon_arguments() {
        let args = SubstrateHygieneLaunchdPlistArgs {
            label: "com.firkin.substrate.hygiene".to_owned(),
            fk_bin: PathBuf::from("/opt/firkin/bin/fk"),
            snapshot_root: PathBuf::from("/var/firkin/snapshots"),
            log_root: PathBuf::from("/var/log/firkin"),
            manifest_root: Some(PathBuf::from("/var/firkin/snapshots")),
            max_log_bytes: 4096,
            interval_seconds: 30,
            gzip_logs: true,
            standard_out_path: Some(PathBuf::from("/var/log/firkin/hygiene.out.log")),
            standard_error_path: Some(PathBuf::from("/var/log/firkin/hygiene.err.log")),
        };
        let mut output = Vec::new();

        write_substrate_hygiene_launchd_plist(&args, &mut output).expect("plist renders");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("<key>Label</key>"));
        assert!(output.contains("<string>com.firkin.substrate.hygiene</string>"));
        assert!(output.contains("<string>/opt/firkin/bin/fk</string>"));
        assert!(output.contains("<string>hygiene-daemon</string>"));
        assert!(output.contains("<string>--snapshot-root</string>"));
        assert!(output.contains("<string>/var/firkin/snapshots</string>"));
        assert!(output.contains("<string>--interval-seconds</string>"));
        assert!(output.contains("<string>30</string>"));
        assert!(output.contains("<string>--gzip-logs</string>"));
        assert!(output.contains("<key>RunAtLoad</key>"));
        assert!(output.contains("<true/>"));
        assert!(output.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn parses_substrate_hygiene_launchd_plist() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "hygiene-launchd-plist",
            "--fk-bin",
            "/opt/firkin/bin/fk",
            "--snapshot-root",
            "/var/firkin/snapshots",
            "--log-root",
            "/var/log/firkin",
            "--standard-out-path",
            "/var/log/firkin/hygiene.out.log",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::HygieneLaunchdPlist(args),
        } = cli.command
        else {
            panic!("expected hygiene-launchd-plist command");
        };

        assert_eq!(args.label, "com.firkin.substrate.hygiene");
        assert_eq!(args.fk_bin, PathBuf::from("/opt/firkin/bin/fk"));
        assert_eq!(args.snapshot_root, PathBuf::from("/var/firkin/snapshots"));
        assert_eq!(args.log_root, PathBuf::from("/var/log/firkin"));
        assert_eq!(args.interval_seconds, 300);
        assert_eq!(
            args.standard_out_path,
            Some(PathBuf::from("/var/log/firkin/hygiene.out.log"))
        );
    }

    #[test]
    fn substrate_reconcile_launchd_plist_renders_start_interval_job() {
        let args = SubstrateReconcileLaunchdPlistArgs {
            label: "com.firkin.substrate.reconcile".to_owned(),
            fk_bin: PathBuf::from("/opt/firkin/bin/fk"),
            active_vm_root: PathBuf::from("/var/firkin/active-vms"),
            snapshot_root: PathBuf::from("/var/firkin/snapshots"),
            log_root: PathBuf::from("/var/log/firkin"),
            process_root: PathBuf::from("/var/firkin/processes"),
            quarantine_root: PathBuf::from("/var/firkin/quarantine"),
            heartbeat_timeout_seconds: 300,
            interval_seconds: 60,
            standard_out_path: Some(PathBuf::from("/var/log/firkin/reconcile.out.log")),
            standard_error_path: Some(PathBuf::from("/var/log/firkin/reconcile.err.log")),
        };
        let mut output = Vec::new();

        write_substrate_reconcile_launchd_plist(&args, &mut output).expect("plist renders");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("<string>com.firkin.substrate.reconcile</string>"));
        assert!(output.contains("<string>/opt/firkin/bin/fk</string>"));
        assert!(output.contains("<string>reconcile-once</string>"));
        assert!(output.contains("<string>--active-vm-root</string>"));
        assert!(output.contains("<string>/var/firkin/active-vms</string>"));
        assert!(output.contains("<string>--quarantine-root</string>"));
        assert!(output.contains("<string>/var/firkin/quarantine</string>"));
        assert!(output.contains("<key>RunAtLoad</key>"));
        assert!(output.contains("<key>StartInterval</key>"));
        assert!(output.contains("<integer>60</integer>"));
        assert!(!output.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn parses_substrate_reconcile_launchd_plist() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "reconcile-launchd-plist",
            "--fk-bin",
            "/opt/firkin/bin/fk",
            "--active-vm-root",
            "/var/firkin/active-vms",
            "--snapshot-root",
            "/var/firkin/snapshots",
            "--log-root",
            "/var/log/firkin",
            "--process-root",
            "/var/firkin/processes",
            "--quarantine-root",
            "/var/firkin/quarantine",
            "--interval-seconds",
            "60",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::ReconcileLaunchdPlist(args),
        } = cli.command
        else {
            panic!("expected reconcile-launchd-plist command");
        };

        assert_eq!(args.label, "com.firkin.substrate.reconcile");
        assert_eq!(args.fk_bin, PathBuf::from("/opt/firkin/bin/fk"));
        assert_eq!(args.active_vm_root, PathBuf::from("/var/firkin/active-vms"));
        assert_eq!(args.snapshot_root, PathBuf::from("/var/firkin/snapshots"));
        assert_eq!(args.log_root, PathBuf::from("/var/log/firkin"));
        assert_eq!(args.process_root, PathBuf::from("/var/firkin/processes"));
        assert_eq!(
            args.quarantine_root,
            PathBuf::from("/var/firkin/quarantine")
        );
        assert_eq!(args.interval_seconds, 60);
        assert_eq!(args.heartbeat_timeout_seconds, 300);
    }

    #[test]
    fn substrate_reconcile_launchd_install_writes_plist() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let plist_path = tempdir
            .path()
            .join("LaunchAgents")
            .join("com.firkin.substrate.reconcile.plist");
        let args = SubstrateReconcileLaunchdInstallArgs {
            plist_path: plist_path.clone(),
            launchd: SubstrateReconcileLaunchdPlistArgs {
                label: "com.firkin.substrate.reconcile".to_owned(),
                fk_bin: PathBuf::from("/opt/firkin/bin/fk"),
                active_vm_root: PathBuf::from("/var/firkin/active-vms"),
                snapshot_root: PathBuf::from("/var/firkin/snapshots"),
                log_root: PathBuf::from("/var/log/firkin"),
                process_root: PathBuf::from("/var/firkin/processes"),
                quarantine_root: PathBuf::from("/var/firkin/quarantine"),
                heartbeat_timeout_seconds: 300,
                interval_seconds: 60,
                standard_out_path: None,
                standard_error_path: None,
            },
        };
        let mut output = Vec::new();

        install_substrate_reconcile_launchd_plist(&args, &mut output).expect("install plist");
        let output = String::from_utf8(output).expect("utf8");
        let plist = std::fs::read_to_string(&plist_path).expect("plist");

        assert!(plist.contains("<string>/opt/firkin/bin/fk</string>"));
        assert!(plist.contains("<string>reconcile-once</string>"));
        assert!(plist.contains("<key>StartInterval</key>"));
        assert!(output.contains("reconcile_launchd_plist=installed"));
        assert!(output.contains(&plist_path.display().to_string()));
    }

    #[test]
    fn parses_substrate_reconcile_launchd_install() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "reconcile-launchd-install",
            "--plist-path",
            "/Users/test/Library/LaunchAgents/com.firkin.substrate.reconcile.plist",
            "--fk-bin",
            "/opt/firkin/bin/fk",
            "--active-vm-root",
            "/var/firkin/active-vms",
            "--snapshot-root",
            "/var/firkin/snapshots",
            "--log-root",
            "/var/log/firkin",
            "--process-root",
            "/var/firkin/processes",
            "--quarantine-root",
            "/var/firkin/quarantine",
            "--interval-seconds",
            "60",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::ReconcileLaunchdInstall(args),
        } = cli.command
        else {
            panic!("expected reconcile-launchd-install command");
        };

        assert_eq!(
            args.plist_path,
            PathBuf::from("/Users/test/Library/LaunchAgents/com.firkin.substrate.reconcile.plist")
        );
        assert_eq!(args.launchd.label, "com.firkin.substrate.reconcile");
        assert_eq!(args.launchd.fk_bin, PathBuf::from("/opt/firkin/bin/fk"));
        assert_eq!(
            args.launchd.active_vm_root,
            PathBuf::from("/var/firkin/active-vms")
        );
        assert_eq!(args.launchd.interval_seconds, 60);
    }

    #[test]
    fn substrate_hygiene_launchd_install_writes_plist() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let plist_path = tempdir
            .path()
            .join("LaunchAgents")
            .join("com.firkin.substrate.hygiene.plist");
        let args = SubstrateHygieneLaunchdInstallArgs {
            plist_path: plist_path.clone(),
            launchd: SubstrateHygieneLaunchdPlistArgs {
                label: "com.firkin.substrate.hygiene".to_owned(),
                fk_bin: PathBuf::from("/opt/firkin/bin/fk"),
                snapshot_root: PathBuf::from("/var/firkin/snapshots"),
                log_root: PathBuf::from("/var/log/firkin"),
                manifest_root: None,
                max_log_bytes: 4096,
                interval_seconds: 30,
                gzip_logs: false,
                standard_out_path: None,
                standard_error_path: None,
            },
        };
        let mut output = Vec::new();

        install_substrate_hygiene_launchd_plist(&args, &mut output).expect("install plist");
        let output = String::from_utf8(output).expect("utf8");
        let plist = std::fs::read_to_string(&plist_path).expect("plist");

        assert!(plist.contains("<string>/opt/firkin/bin/fk</string>"));
        assert!(plist.contains("<string>hygiene-daemon</string>"));
        assert!(output.contains("hygiene_launchd_plist=installed"));
        assert!(output.contains(&plist_path.display().to_string()));
    }

    #[test]
    fn parses_substrate_hygiene_launchd_install() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "hygiene-launchd-install",
            "--plist-path",
            "/Users/test/Library/LaunchAgents/com.firkin.substrate.hygiene.plist",
            "--fk-bin",
            "/opt/firkin/bin/fk",
            "--snapshot-root",
            "/var/firkin/snapshots",
            "--log-root",
            "/var/log/firkin",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::HygieneLaunchdInstall(args),
        } = cli.command
        else {
            panic!("expected hygiene-launchd-install command");
        };

        assert_eq!(
            args.plist_path,
            PathBuf::from("/Users/test/Library/LaunchAgents/com.firkin.substrate.hygiene.plist")
        );
        assert_eq!(args.launchd.fk_bin, PathBuf::from("/opt/firkin/bin/fk"));
        assert_eq!(
            args.launchd.snapshot_root,
            PathBuf::from("/var/firkin/snapshots")
        );
    }

    #[test]
    fn substrate_hygiene_launchd_bootstrap_builds_launchctl_commands() {
        let args = SubstrateHygieneLaunchdBootstrapArgs {
            domain: "gui/501".to_owned(),
            label: "com.firkin.substrate.hygiene".to_owned(),
            plist_path: PathBuf::from(
                "/Users/test/Library/LaunchAgents/com.firkin.substrate.hygiene.plist",
            ),
        };

        let commands = hygiene_launchd_bootstrap_commands(&args);

        assert_eq!(
            commands,
            vec![
                vec![
                    "bootstrap".to_owned(),
                    "gui/501".to_owned(),
                    "/Users/test/Library/LaunchAgents/com.firkin.substrate.hygiene.plist"
                        .to_owned(),
                ],
                vec![
                    "kickstart".to_owned(),
                    "-k".to_owned(),
                    "gui/501/com.firkin.substrate.hygiene".to_owned(),
                ],
            ]
        );
    }

    #[test]
    fn parses_substrate_hygiene_launchd_bootstrap() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "hygiene-launchd-bootstrap",
            "--domain",
            "gui/501",
            "--label",
            "com.firkin.substrate.hygiene",
            "--plist-path",
            "/Users/test/Library/LaunchAgents/com.firkin.substrate.hygiene.plist",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::HygieneLaunchdBootstrap(args),
        } = cli.command
        else {
            panic!("expected hygiene-launchd-bootstrap command");
        };

        assert_eq!(args.domain, "gui/501");
        assert_eq!(args.label, "com.firkin.substrate.hygiene");
        assert_eq!(
            args.plist_path,
            PathBuf::from("/Users/test/Library/LaunchAgents/com.firkin.substrate.hygiene.plist")
        );
    }

    #[test]
    fn substrate_hygiene_launchd_status_builds_launchctl_command() {
        let args = SubstrateHygieneLaunchdStatusArgs {
            domain: "gui/501".to_owned(),
            label: "com.firkin.substrate.hygiene".to_owned(),
        };

        let command = hygiene_launchd_status_command(&args);

        assert_eq!(
            command,
            vec![
                "print".to_owned(),
                "gui/501/com.firkin.substrate.hygiene".to_owned()
            ]
        );
    }

    #[test]
    fn parses_substrate_hygiene_launchd_status() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "hygiene-launchd-status",
            "--domain",
            "gui/501",
            "--label",
            "com.firkin.substrate.hygiene",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::HygieneLaunchdStatus(args),
        } = cli.command
        else {
            panic!("expected hygiene-launchd-status command");
        };

        assert_eq!(args.domain, "gui/501");
        assert_eq!(args.label, "com.firkin.substrate.hygiene");
    }

    #[test]
    fn substrate_reconcile_launchd_bootstrap_builds_launchctl_commands() {
        let args = SubstrateReconcileLaunchdBootstrapArgs {
            domain: "gui/501".to_owned(),
            label: "com.firkin.substrate.reconcile".to_owned(),
            plist_path: PathBuf::from(
                "/Users/test/Library/LaunchAgents/com.firkin.substrate.reconcile.plist",
            ),
        };

        let commands = reconcile_launchd_bootstrap_commands(&args);

        assert_eq!(
            commands,
            vec![
                vec![
                    "bootstrap".to_owned(),
                    "gui/501".to_owned(),
                    "/Users/test/Library/LaunchAgents/com.firkin.substrate.reconcile.plist"
                        .to_owned(),
                ],
                vec![
                    "kickstart".to_owned(),
                    "-k".to_owned(),
                    "gui/501/com.firkin.substrate.reconcile".to_owned(),
                ],
            ]
        );
    }

    #[test]
    fn parses_substrate_reconcile_launchd_bootstrap() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "reconcile-launchd-bootstrap",
            "--domain",
            "gui/501",
            "--label",
            "com.firkin.substrate.reconcile",
            "--plist-path",
            "/Users/test/Library/LaunchAgents/com.firkin.substrate.reconcile.plist",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::ReconcileLaunchdBootstrap(args),
        } = cli.command
        else {
            panic!("expected reconcile-launchd-bootstrap command");
        };

        assert_eq!(args.domain, "gui/501");
        assert_eq!(args.label, "com.firkin.substrate.reconcile");
        assert_eq!(
            args.plist_path,
            PathBuf::from("/Users/test/Library/LaunchAgents/com.firkin.substrate.reconcile.plist")
        );
    }

    #[test]
    fn substrate_reconcile_launchd_status_builds_launchctl_command() {
        let args = SubstrateReconcileLaunchdStatusArgs {
            domain: "gui/501".to_owned(),
            label: "com.firkin.substrate.reconcile".to_owned(),
        };

        let command = reconcile_launchd_status_command(&args);

        assert_eq!(
            command,
            vec![
                "print".to_owned(),
                "gui/501/com.firkin.substrate.reconcile".to_owned()
            ]
        );
    }

    #[test]
    fn parses_substrate_reconcile_launchd_status() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "reconcile-launchd-status",
            "--domain",
            "gui/501",
            "--label",
            "com.firkin.substrate.reconcile",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::ReconcileLaunchdStatus(args),
        } = cli.command
        else {
            panic!("expected reconcile-launchd-status command");
        };

        assert_eq!(args.domain, "gui/501");
        assert_eq!(args.label, "com.firkin.substrate.reconcile");
    }

    #[test]
    fn substrate_stuck_vm_plan_formats_cleanup_decisions() {
        let args = SubstrateStuckVmPlanArgs {
            heartbeat_timeout_seconds: 300,
            vms: vec![
                StuckVmCliObservation {
                    id: "vm-old".to_owned(),
                    heartbeat_age_seconds: 600,
                },
                StuckVmCliObservation {
                    id: "vm-recent".to_owned(),
                    heartbeat_age_seconds: 10,
                },
                StuckVmCliObservation {
                    id: String::new(),
                    heartbeat_age_seconds: 600,
                },
            ],
        };
        let mut output = Vec::new();

        write_substrate_stuck_vm_plan(&args, &mut output).expect("plan");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("stuck_vm_plan=ok heartbeat_timeout_seconds=300 decisions=3"));
        assert!(output.contains("vm_id=vm-old heartbeat_age_seconds=600 decision=cleanup"));
        assert!(output.contains("vm_id=vm-recent heartbeat_age_seconds=10 decision=preserve"));
        assert!(output.contains("vm_id=- heartbeat_age_seconds=600 decision=quarantine"));
    }

    #[test]
    fn parses_substrate_stuck_vm_plan() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "stuck-vm-plan",
            "--heartbeat-timeout-seconds",
            "120",
            "--vm",
            "vm-old=600",
            "--vm",
            "vm-recent=10",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::StuckVmPlan(args),
        } = cli.command
        else {
            panic!("expected stuck-vm-plan command");
        };

        assert_eq!(args.heartbeat_timeout_seconds, 120);
        assert_eq!(
            args.vms,
            vec![
                StuckVmCliObservation {
                    id: "vm-old".to_owned(),
                    heartbeat_age_seconds: 600,
                },
                StuckVmCliObservation {
                    id: "vm-recent".to_owned(),
                    heartbeat_age_seconds: 10,
                },
            ]
        );
    }

    #[test]
    fn substrate_host_scan_formats_json_reconciliation_and_stuck_vm_decisions() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let active_vms = tempdir.path().join("active-vms");
        let snapshots = tempdir.path().join("snapshots");
        let logs = tempdir.path().join("logs");
        let processes = tempdir.path().join("processes");
        std::fs::create_dir_all(&active_vms).expect("active vms");
        std::fs::create_dir_all(&snapshots).expect("snapshots");
        std::fs::create_dir_all(&logs).expect("logs");
        std::fs::create_dir_all(&processes).expect("processes");
        write_active_vm_marker(&active_vms, "vm-old", 600, 41);
        write_active_vm_marker(&active_vms, "vm-recent", 10, 42);
        std::fs::write(snapshots.join("snapshot-1"), b"").expect("snapshot marker");
        std::fs::write(logs.join("runtime.log"), b"").expect("log marker");
        std::fs::write(processes.join("pid-123"), b"").expect("process marker");
        let args = SubstrateHostScanArgs {
            active_vm_root: active_vms,
            snapshot_root: snapshots,
            log_root: logs,
            process_root: processes,
            heartbeat_timeout_seconds: 300,
        };
        let mut output = Vec::new();

        write_substrate_host_scan(&args, &mut output).expect("host scan");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.trim_start().starts_with('{'));
        assert!(output.contains(r#""host_scan":"ok""#));
        assert!(output.contains(r#""restart_decision_count":5"#));
        assert!(output.contains(r#""stuck_vm_decision_count":2"#));
        assert!(output.contains(
            r#""restart_decisions":[{"id":"vm-old","kind":"active_vm","decision":"recover""#
        ));
        assert!(output.contains(
            r#""stuck_vm_decisions":[{"id":"vm-old","heartbeat_age_seconds":600,"runtime_pid":41,"decision":"cleanup""#
        ));
    }

    #[test]
    fn parses_substrate_host_scan() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "host-scan",
            "--active-vm-root",
            "/var/firkin/active-vms",
            "--snapshot-root",
            "/var/firkin/snapshots",
            "--log-root",
            "/var/log/firkin",
            "--process-root",
            "/var/firkin/processes",
            "--heartbeat-timeout-seconds",
            "120",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::HostScan(args),
        } = cli.command
        else {
            panic!("expected host-scan command");
        };

        assert_eq!(args.active_vm_root, PathBuf::from("/var/firkin/active-vms"));
        assert_eq!(args.snapshot_root, PathBuf::from("/var/firkin/snapshots"));
        assert_eq!(args.log_root, PathBuf::from("/var/log/firkin"));
        assert_eq!(args.process_root, PathBuf::from("/var/firkin/processes"));
        assert_eq!(args.heartbeat_timeout_seconds, 120);
    }

    #[test]
    fn substrate_reconcile_once_applies_filesystem_reconciliation_and_stuck_vm_cleanup() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let active_vms = tempdir.path().join("active-vms");
        let snapshots = tempdir.path().join("snapshots");
        let logs = tempdir.path().join("logs");
        let processes = tempdir.path().join("processes");
        let quarantine = tempdir.path().join("quarantine");
        std::fs::create_dir_all(&active_vms).expect("active vms");
        std::fs::create_dir_all(&snapshots).expect("snapshots");
        std::fs::create_dir_all(&logs).expect("logs");
        std::fs::create_dir_all(&processes).expect("processes");
        write_active_vm_marker(&active_vms, "vm-old", 600, 41);
        write_active_vm_marker(&active_vms, "vm-recent", 10, 42);
        std::fs::write(logs.join("runtime.log"), b"").expect("log marker");
        std::fs::write(processes.join("pid-123"), b"").expect("process marker");
        let args = SubstrateReconcileOnceArgs {
            active_vm_root: active_vms.clone(),
            snapshot_root: snapshots,
            log_root: logs.clone(),
            process_root: processes.clone(),
            quarantine_root: quarantine,
            heartbeat_timeout_seconds: 300,
        };
        let mut output = Vec::new();
        let terminated = Arc::new(Mutex::new(Vec::new()));
        let terminator = RecordingHostProcessTerminator {
            terminated: Arc::clone(&terminated),
        };

        run_substrate_reconcile_once_with_terminator(&args, &mut output, terminator)
            .expect("reconcile once");
        let output = String::from_utf8(output).expect("utf8");

        assert!(!active_vms.join("vm-old").exists());
        assert!(active_vms.join("vm-recent").exists());
        assert_eq!(*terminated.lock().expect("terminated lock"), vec![41]);
        assert!(!logs.join("runtime.log").exists());
        assert!(!processes.join("pid-123").exists());
        assert!(output.contains(r#""reconcile_once":"ok""#));
        assert!(output.contains(r#""restart":{"recovered":2,"cleaned":2,"quarantined":0}"#));
        assert!(output.contains(r#""stuck_vm":{"preserved":1,"cleaned":1,"quarantined":0}"#));
    }

    #[test]
    fn substrate_reconcile_once_treats_missing_marker_roots_as_empty() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let args = SubstrateReconcileOnceArgs {
            active_vm_root: tempdir.path().join("missing-active-vms"),
            snapshot_root: tempdir.path().join("missing-snapshots"),
            log_root: tempdir.path().join("missing-logs"),
            process_root: tempdir.path().join("missing-processes"),
            quarantine_root: tempdir.path().join("quarantine"),
            heartbeat_timeout_seconds: 300,
        };
        let mut output = Vec::new();

        run_substrate_reconcile_once(&args, &mut output).expect("reconcile once");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains(r#""reconcile_once":"ok""#));
        assert!(output.contains(r#""restart":{"recovered":0,"cleaned":0,"quarantined":0}"#));
        assert!(output.contains(r#""stuck_vm":{"preserved":0,"cleaned":0,"quarantined":0}"#));
    }

    #[test]
    fn parses_substrate_reconcile_once() {
        let cli = Cli::parse_from([
            "fk",
            "substrate",
            "reconcile-once",
            "--active-vm-root",
            "/var/firkin/active-vms",
            "--snapshot-root",
            "/var/firkin/snapshots",
            "--log-root",
            "/var/log/firkin",
            "--process-root",
            "/var/firkin/processes",
            "--quarantine-root",
            "/var/firkin/quarantine",
            "--heartbeat-timeout-seconds",
            "120",
        ]);
        let Command::Substrate {
            command: SubstrateCommand::ReconcileOnce(args),
        } = cli.command
        else {
            panic!("expected reconcile-once command");
        };

        assert_eq!(args.active_vm_root, PathBuf::from("/var/firkin/active-vms"));
        assert_eq!(args.snapshot_root, PathBuf::from("/var/firkin/snapshots"));
        assert_eq!(args.log_root, PathBuf::from("/var/log/firkin"));
        assert_eq!(args.process_root, PathBuf::from("/var/firkin/processes"));
        assert_eq!(
            args.quarantine_root,
            PathBuf::from("/var/firkin/quarantine")
        );
        assert_eq!(args.heartbeat_timeout_seconds, 120);
    }

    #[test]
    fn substrate_hygiene_once_reads_sidecars_and_rotates_logs() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let snapshot_root = tempdir.path().join("snapshots");
        let log_root = tempdir.path().join("logs");
        std::fs::create_dir_all(&snapshot_root).expect("snapshots");
        std::fs::create_dir_all(&log_root).expect("logs");
        let keep = snapshot_root.join("keep.vzstate");
        let delete = snapshot_root.join("delete.vzstate");
        let log = log_root.join("runtime.log");
        std::fs::write(&keep, b"keep").expect("keep");
        std::fs::write(&delete, b"delete").expect("delete");
        std::fs::write(&log, b"0123456789").expect("log");
        firkin::substrate::SnapshotArtifactManifest::base("repo-main", &keep)
            .write_json(snapshot_root.join("keep.manifest.json"))
            .expect("manifest");
        let args = SubstrateHygieneOnceArgs {
            snapshot_root: snapshot_root.clone(),
            log_root: log_root.clone(),
            manifest_root: Some(snapshot_root.clone()),
            max_log_bytes: 4,
            gzip_logs: true,
        };
        let mut output = Vec::new();

        run_substrate_hygiene_once(&args, &mut output).expect("hygiene succeeds");
        let output = String::from_utf8(output).expect("utf8");

        assert!(keep.exists());
        assert!(!delete.exists());
        assert!(log_root.join("runtime.log.1.gz").exists());
        assert!(output.contains("hygiene_tick=passed"));
        assert!(output.contains("artifact_deleted=1"));
        assert!(output.contains("log_rotated=1"));
        assert!(output.contains("gzip_logs=true"));
    }

    #[test]
    fn validates_lifecycle_slo_artifact_against_targets() {
        let artifact = std::env::temp_dir().join(format!(
            "firkin-cli-benchmark-evidence-{}.json",
            std::process::id()
        ));
        let samples = firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS
            .iter()
            .flat_map(|metric| {
                (0..100).map(move |_| {
                    firkin::trace::BenchmarkSample::new(
                        *metric,
                        firkin::trace::BenchmarkMetricKind::LifecycleLatency,
                        firkin::trace::BenchmarkUnit::Milliseconds,
                        1.0,
                    )
                })
            })
            .collect::<Vec<_>>();
        let report =
            firkin::evidence::BenchmarkEvidenceReport::from_samples(samples).expect("report");
        firkin::evidence::BenchmarkEvidenceArtifact::write_json(&artifact, &report)
            .expect("artifact");
        let args = ValidateLifecycleSloArgs {
            artifact,
            min_samples: 3,
        };
        let mut output = Vec::new();

        validate_lifecycle_slo(&args, &mut output).expect("slo passes");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("benchmark_slo_gate=passed kind=lifecycle"));
        assert!(output.contains(&format!(
            "targets={}",
            firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS.len()
        )));
        assert!(output.contains("min_samples=3"));
        let _ = std::fs::remove_file(args.artifact);
    }

    #[test]
    fn reports_lifecycle_benchmark_artifact() {
        let artifact = std::env::temp_dir().join(format!(
            "firkin-cli-benchmark-report-lifecycle-{}.json",
            std::process::id()
        ));
        let samples = firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS
            .iter()
            .flat_map(|metric| {
                (1..=100).map(move |value| {
                    firkin::trace::BenchmarkSample::new(
                        *metric,
                        firkin::trace::BenchmarkMetricKind::LifecycleLatency,
                        firkin::trace::BenchmarkUnit::Milliseconds,
                        f64::from(value),
                    )
                })
            })
            .collect::<Vec<_>>();
        let report =
            firkin::evidence::BenchmarkEvidenceReport::from_samples(samples).expect("report");
        firkin::evidence::BenchmarkEvidenceArtifact::write_json(&artifact, &report)
            .expect("artifact");
        let args = BenchmarkReportArgs {
            kind: BenchmarkReportKind::Lifecycle,
            artifact,
        };
        let mut output = Vec::new();

        write_benchmark_report(&args, &mut output).expect("benchmark report");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("benchmark_report=summary kind=lifecycle"));
        assert!(output.contains(&format!(
            "metrics={}",
            firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS.len()
        )));
        assert!(
            output.contains("metric=start.hot_to_first_stdout_ms kind=LifecycleLatency unit=ms")
        );
        assert!(output.contains("count=100 p50=50 p90=90 p95=95 p99=99 max=100"));
        let _ = std::fs::remove_file(args.artifact);
    }

    #[test]
    fn reports_decision_confidence_without_overclaiming_tiny_samples() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let artifact = tempdir.path().join("decision-smoke.json");
        write_lifecycle_artifact_with_values(&artifact, [1.0, 2.0, 3.0]);
        let args = BenchmarkReportArgs {
            kind: BenchmarkReportKind::Decision,
            artifact,
        };
        let mut output = Vec::new();

        write_benchmark_report(&args, &mut output).expect("decision report");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("benchmark_report=decision artifact="));
        assert!(output.contains("artifact_kind=lifecycle"));
        assert!(output.contains("metric=start.hot_to_first_stdout_ms"));
        assert!(output.contains("count=3"));
        assert!(output.contains("confidence=superfast_iteration"));
        assert!(output.contains("unstable_percentile=true"));
        assert!(output.contains("p95_status=unstable"));
        assert!(output.contains("p99_status=experimental"));
    }

    #[test]
    fn reports_decision_confidence_for_baseline_checkpoint_samples() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let artifact = tempdir.path().join("decision-checkpoint.json");
        write_lifecycle_artifact_with_values(
            &artifact,
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
        );
        let args = BenchmarkReportArgs {
            kind: BenchmarkReportKind::Decision,
            artifact,
        };
        let mut output = Vec::new();

        write_benchmark_report(&args, &mut output).expect("decision report");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("metric=start.hot_to_first_stdout_ms"));
        assert!(output.contains("count=10"));
        assert!(output.contains("confidence=baseline_checkpoint"));
        assert!(output.contains("unstable_percentile=true"));
        assert!(output.contains("p95_status=unstable"));
    }

    #[test]
    fn reports_decision_from_raw_live_sample_artifact() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let artifact = tempdir.path().join("retained-batch.json");
        write_raw_live_sample_artifact(
            &artifact,
            "live_retained_shell_batch_100",
            "exec.batch_100_small_commands_ms",
            [28.0, 113.0, 148.0],
        );
        let args = BenchmarkReportArgs {
            kind: BenchmarkReportKind::Decision,
            artifact,
        };
        let mut output = Vec::new();

        write_benchmark_report(&args, &mut output).expect("decision report");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("benchmark_report=decision artifact="));
        assert!(output.contains("artifact_kind=live_retained_shell_batch_100"));
        assert!(output.contains("metric=exec.batch_100_small_commands_ms"));
        assert!(output.contains("count=3"));
        assert!(output.contains("confidence=superfast_iteration"));
    }

    #[test]
    fn reports_lifecycle_from_raw_live_sample_artifact() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let artifact = tempdir.path().join("retained-batch.json");
        write_raw_live_sample_artifact(
            &artifact,
            "live_retained_shell_batch_100",
            "exec.batch_100_small_commands_ms",
            [28.0, 113.0, 148.0],
        );
        let args = BenchmarkReportArgs {
            kind: BenchmarkReportKind::Lifecycle,
            artifact,
        };
        let mut output = Vec::new();

        write_benchmark_report(&args, &mut output).expect("lifecycle report");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("benchmark_report=summary kind=live_retained_shell_batch_100"));
        assert!(output.contains("metric=exec.batch_100_small_commands_ms"));
        assert!(output.contains("count=3 p50=113 p90=148 p95=148 p99=148 max=148"));
    }

    #[test]
    fn saves_lists_and_compares_benchmark_baselines() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let baseline_artifact = tempdir.path().join("baseline.json");
        let current_artifact = tempdir.path().join("current.json");
        write_lifecycle_artifact_with_value(&baseline_artifact, 10.0);
        write_lifecycle_artifact_with_value(&current_artifact, 20.0);

        let save_args = BenchmarkBaselineSaveArgs {
            artifact: baseline_artifact,
            name: "local_agent_core".to_owned(),
            benchmark_root: Some(tempdir.path().join("benchmarks")),
        };
        let mut save_output = Vec::new();
        save_benchmark_baseline(&save_args, &mut save_output).expect("save baseline");
        let save_output = String::from_utf8(save_output).expect("utf8");
        assert!(save_output.contains("benchmark_baseline=saved"));

        let list_args = BenchmarkBaselineListArgs {
            benchmark_root: save_args.benchmark_root.clone(),
        };
        let mut list_output = Vec::new();
        list_benchmark_baselines(&list_args, &mut list_output).expect("list baselines");
        let list_output = String::from_utf8(list_output).expect("utf8");
        assert!(list_output.contains("baseline=local_agent_core"));

        let compare_args = BenchmarkCompareArgs {
            baseline: baseline_path(
                save_args.benchmark_root.as_ref().expect("root"),
                "local_agent_core",
            ),
            current: current_artifact,
            rank: BenchmarkCompareRank::Bottlenecks,
        };
        let mut compare_output = Vec::new();
        compare_benchmark_artifacts(&compare_args, &mut compare_output).expect("compare");
        let compare_output = String::from_utf8(compare_output).expect("utf8");

        assert!(compare_output.contains("benchmark_compare=summary rank=bottlenecks"));
        assert!(compare_output.contains("metric=start.hot_to_first_stdout_ms"));
        assert!(compare_output.contains("delta_p95=10"));
    }

    #[test]
    fn compares_broad_benchmark_summary_artifacts() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let baseline_artifact = tempdir.path().join("broad-baseline.json");
        let current_artifact = tempdir.path().join("broad-current.json");
        write_lifecycle_artifact_with_value(&baseline_artifact, 10.0);
        write_lifecycle_artifact_with_value(&current_artifact, 20.0);
        make_required_metrics_match_summaries(&baseline_artifact);
        make_required_metrics_match_summaries(&current_artifact);

        let compare_args = BenchmarkCompareArgs {
            baseline: baseline_artifact,
            current: current_artifact,
            rank: BenchmarkCompareRank::Bottlenecks,
        };
        let mut compare_output = Vec::new();
        compare_benchmark_artifacts(&compare_args, &mut compare_output).expect("compare");
        let compare_output = String::from_utf8(compare_output).expect("utf8");

        assert!(compare_output.contains("baseline_kind=benchmark current_kind=benchmark"));
        assert!(compare_output.contains("metric=start.hot_to_first_stdout_ms"));
        assert!(compare_output.contains("delta_p95=10"));
    }

    #[test]
    fn compare_marks_low_sample_rows_collect_more_samples() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let baseline_artifact = tempdir.path().join("baseline.json");
        let current_artifact = tempdir.path().join("current.json");
        write_lifecycle_artifact_with_values(&baseline_artifact, [1.0, 2.0, 3.0]);
        write_lifecycle_artifact_with_values(&current_artifact, [2.0, 3.0, 4.0]);

        let compare_args = BenchmarkCompareArgs {
            baseline: baseline_artifact,
            current: current_artifact,
            rank: BenchmarkCompareRank::Bottlenecks,
        };
        let mut output = Vec::new();
        compare_benchmark_artifacts(&compare_args, &mut output).expect("compare");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("metric=start.hot_to_first_stdout_ms"));
        assert!(output.contains("next_action=collect_more_samples"));
        assert!(output.contains("confidence=superfast_iteration"));
        assert!(output.contains("p95_status=unstable"));
    }

    fn write_lifecycle_artifact_with_value(path: &Path, value: f64) {
        write_lifecycle_artifact_with_values(path, [value]);
    }

    fn write_lifecycle_artifact_with_values(path: &Path, values: impl IntoIterator<Item = f64>) {
        let samples = lifecycle_samples_with_values(values);
        let report =
            firkin::evidence::BenchmarkEvidenceReport::from_samples(samples).expect("report");
        firkin::evidence::BenchmarkEvidenceArtifact::write_json(path, &report).expect("artifact");
    }

    fn write_raw_live_sample_artifact(
        path: &Path,
        kind: &str,
        metric: &str,
        values: impl IntoIterator<Item = f64>,
    ) {
        let samples = values
            .into_iter()
            .map(|value| {
                firkin::trace::BenchmarkSample::new(
                    metric,
                    firkin::trace::BenchmarkMetricKind::LifecycleLatency,
                    firkin::trace::BenchmarkUnit::Milliseconds,
                    value,
                )
            })
            .collect::<Vec<_>>();
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": kind,
                "samples": samples,
            }))
            .expect("json"),
        )
        .expect("artifact");
    }

    fn make_required_metrics_match_summaries(path: &Path) {
        let bytes = std::fs::read(path).expect("read artifact");
        let mut value = serde_json::from_slice::<serde_json::Value>(&bytes).expect("json");
        let metrics = value["summaries"]
            .as_array()
            .expect("summaries")
            .iter()
            .map(|summary| summary["metric"].clone())
            .collect::<Vec<_>>();
        let mut required_metrics = metrics;
        required_metrics.push(serde_json::Value::String(
            "benchmark.extra_probe".to_owned(),
        ));
        value["required_metrics"] = serde_json::Value::Array(required_metrics);
        std::fs::write(path, serde_json::to_vec_pretty(&value).expect("json")).expect("write");
    }

    fn lifecycle_samples_with_values(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<firkin::trace::BenchmarkSample> {
        let values = values.into_iter().collect::<Vec<_>>();
        let mut samples = firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS
            .iter()
            .flat_map(|metric| {
                values.iter().copied().map(move |value| {
                    firkin::trace::BenchmarkSample::new(
                        *metric,
                        firkin::trace::BenchmarkMetricKind::LifecycleLatency,
                        firkin::trace::BenchmarkUnit::Milliseconds,
                        value,
                    )
                })
            })
            .collect::<Vec<_>>();
        samples.extend(
            firkin::evidence::p0_scorecard_measurement_coverage()
                .iter()
                .filter(|coverage| {
                    coverage
                        .source
                        .starts_with("live_runtime_benchmark_evidence:")
                })
                .filter_map(|coverage| {
                    firkin::evidence::benchmark_metric_definition(coverage.metric)
                })
                .filter(|definition| {
                    !firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS.contains(&definition.name)
                })
                .flat_map(|definition| {
                    values.iter().copied().map(move |value| {
                        firkin::trace::BenchmarkSample::new(
                            definition.name,
                            definition.kind,
                            definition.unit,
                            value,
                        )
                    })
                }),
        );
        samples
    }

    fn write_overhead_artifact_with_value(path: &Path, value: f64) {
        let mut samples = firkin::evidence::REQUIRED_FIRKIN_OVERHEAD_METRICS
            .iter()
            .map(|metric| {
                firkin::trace::BenchmarkSample::new(
                    metric.name,
                    firkin::trace::BenchmarkMetricKind::FirkinOverhead,
                    metric.unit,
                    value,
                )
            })
            .collect::<Vec<_>>();
        samples.extend([
            firkin::trace::BenchmarkSample::new(
                "sandbox.mem.idle_host_footprint_bytes",
                firkin::trace::BenchmarkMetricKind::WorkloadResource,
                firkin::trace::BenchmarkUnit::Bytes,
                value,
            ),
            firkin::trace::BenchmarkSample::new(
                "sandbox.mem.post_task_residual_bytes",
                firkin::trace::BenchmarkMetricKind::WorkloadResource,
                firkin::trace::BenchmarkUnit::Bytes,
                value,
            ),
            firkin::trace::BenchmarkSample::new(
                "sandbox.mem.reclaim_effectiveness_ratio",
                firkin::trace::BenchmarkMetricKind::WorkloadResource,
                firkin::trace::BenchmarkUnit::Ratio,
                value,
            ),
        ]);
        let report = firkin::evidence::BenchmarkOverheadEvidenceReport::from_samples(samples)
            .expect("report");
        firkin::evidence::BenchmarkOverheadEvidenceArtifact::write_json(path, &report)
            .expect("artifact");
    }

    fn write_scorecard_artifact_with_values(path: &Path, values: impl IntoIterator<Item = f64>) {
        let report = firkin::evidence::AgentBenchmarkScorecardReport::from_samples(
            scorecard_samples(values),
        )
        .expect("report");
        firkin::evidence::AgentBenchmarkScorecardArtifact::write_json(path, &report)
            .expect("artifact");
    }

    fn write_autoscale_scorecard_artifact_with_values(
        path: &Path,
        values: impl IntoIterator<Item = f64>,
    ) {
        let report = firkin::evidence::AutoscaleEfficiencyScorecardReport::from_samples(
            autoscale_scorecard_samples(values),
        )
        .expect("report");
        firkin::evidence::AutoscaleEfficiencyScorecardArtifact::write_json(path, &report)
            .expect("artifact");
    }

    #[test]
    fn reports_overhead_benchmark_artifact() {
        let artifact = std::env::temp_dir().join(format!(
            "firkin-cli-benchmark-report-overhead-{}.json",
            std::process::id()
        ));
        let samples = firkin::evidence::REQUIRED_FIRKIN_OVERHEAD_METRICS
            .iter()
            .flat_map(|metric| {
                (1..=100).map(move |value| {
                    firkin::trace::BenchmarkSample::new(
                        metric.name,
                        firkin::trace::BenchmarkMetricKind::FirkinOverhead,
                        metric.unit,
                        f64::from(value) / 10.0,
                    )
                })
            })
            .collect::<Vec<_>>();
        let report = firkin::evidence::BenchmarkOverheadEvidenceReport::from_samples(samples)
            .expect("report");
        firkin::evidence::BenchmarkOverheadEvidenceArtifact::write_json(&artifact, &report)
            .expect("artifact");
        let args = BenchmarkReportArgs {
            kind: BenchmarkReportKind::Overhead,
            artifact,
        };
        let mut output = Vec::new();

        write_benchmark_report(&args, &mut output).expect("benchmark report");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("benchmark_report=summary kind=overhead"));
        assert!(output.contains("metrics=5"));
        assert!(output.contains("metric=control_plane_cpu_idle kind=FirkinOverhead unit=percent"));
        assert!(output.contains("count=100 p50=5 p90=9 p95=9.5 p99=9.9 max=10"));
        let _ = std::fs::remove_file(args.artifact);
    }

    #[test]
    fn validates_overhead_slo_artifact_against_targets() {
        let artifact = std::env::temp_dir().join(format!(
            "firkin-cli-overhead-evidence-{}.json",
            std::process::id()
        ));
        let samples = firkin::evidence::REQUIRED_FIRKIN_OVERHEAD_METRICS
            .iter()
            .flat_map(|metric| {
                [0.1, 0.2, 0.3].into_iter().map(move |value| {
                    firkin::trace::BenchmarkSample::new(
                        metric.name,
                        firkin::trace::BenchmarkMetricKind::FirkinOverhead,
                        metric.unit,
                        value,
                    )
                })
            })
            .collect::<Vec<_>>();
        let report = firkin::evidence::BenchmarkOverheadEvidenceReport::from_samples(samples)
            .expect("report");
        firkin::evidence::BenchmarkOverheadEvidenceArtifact::write_json(&artifact, &report)
            .expect("artifact");
        let args = ValidateOverheadSloArgs {
            artifact,
            min_samples: 3,
        };
        let mut output = Vec::new();

        validate_overhead_slo(&args, &mut output).expect("slo passes");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("benchmark_slo_gate=passed kind=overhead"));
        assert!(output.contains("targets=5"));
        assert!(output.contains("min_samples=3"));
        let _ = std::fs::remove_file(args.artifact);
    }

    fn scorecard_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<firkin::trace::BenchmarkSample> {
        let values = values.into_iter().collect::<Vec<_>>();
        firkin::evidence::required_scorecard_metric_definitions()
            .into_iter()
            .flat_map(|metric| {
                values.iter().copied().map(move |value| {
                    let sample = firkin::trace::BenchmarkSample::new(
                        metric.name,
                        metric.kind,
                        metric.unit,
                        value,
                    );
                    match metric.name {
                        "product.agent_computer_ready_ms" | "product.agent_computer_resume_ms" => {
                            sample
                                .with_static_tag("cli_boundary", "real_cli")
                                .with_static_tag("browser_boundary", "real_browser_sidecar")
                                .with_static_tag("database_boundary", "real_db_sidecar")
                        }
                        _ => sample,
                    }
                })
            })
            .collect()
    }

    fn autoscale_scorecard_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<firkin::trace::BenchmarkSample> {
        let values = values.into_iter().collect::<Vec<_>>();
        firkin::evidence::required_autoscale_efficiency_metric_definitions()
            .into_iter()
            .flat_map(|metric| {
                values.iter().copied().map(move |value| {
                    let sample = firkin::trace::BenchmarkSample::new(
                        metric.name,
                        metric.kind,
                        metric.unit,
                        value,
                    );
                    match metric.name {
                        "product.agent_computer_ready_ms" | "product.agent_computer_resume_ms" => {
                            sample
                                .with_static_tag("cli_boundary", "real_cli")
                                .with_static_tag("browser_boundary", "real_browser_sidecar")
                                .with_static_tag("database_boundary", "real_db_sidecar")
                        }
                        _ => sample,
                    }
                })
            })
            .collect()
    }

    fn agent_computer_scorecard_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<firkin::trace::BenchmarkSample> {
        let values = values.into_iter().collect::<Vec<_>>();
        let mut samples = firkin::evidence::required_agent_computer_metric_definitions()
            .into_iter()
            .flat_map(|metric| {
                values.iter().copied().map(move |value| {
                    let sample = firkin::trace::BenchmarkSample::new(
                        metric.name,
                        metric.kind,
                        metric.unit,
                        value,
                    );
                    match metric.name {
                        "product.agent_computer_ready_ms" | "product.agent_computer_resume_ms" => {
                            sample
                                .with_static_tag("cli_boundary", "real_cli")
                                .with_static_tag("browser_boundary", "real_browser_sidecar")
                                .with_static_tag("database_boundary", "real_db_sidecar")
                        }
                        "density.max_agent_computers_before_ready_p95_doubles" => sample
                            .with_static_tag("measurement_boundary", "product_path")
                            .with_static_tag("pod_surface", "product_pod_ready_deck")
                            .with_static_tag("excludes_container_add", "false")
                            .with_static_tag(
                                "ready_signal",
                                "agent_computer_ready_after_container_add",
                            )
                            .with_static_tag("cli_boundary", "real_cli")
                            .with_static_tag("browser_boundary", "real_browser_sidecar")
                            .with_static_tag("database_boundary", "real_db_sidecar"),
                        _ => sample,
                    }
                })
            })
            .collect::<Vec<_>>();
        samples.extend(product_density_capacity_tier_samples());
        samples
    }

    fn product_density_capacity_tier_samples() -> Vec<firkin::trace::BenchmarkSample> {
        [
            (
                "debug.product.agent_computer_ready_deck_c4_ms",
                110.0,
                "snappy_4",
            ),
            (
                "debug.product.agent_computer_ready_deck_c8_ms",
                225.0,
                "snappy_8",
            ),
            (
                "debug.product.agent_computer_ready_deck_c16_ms",
                450.0,
                "degraded_16",
            ),
        ]
        .into_iter()
        .map(|(metric, value, tier)| {
            firkin::trace::BenchmarkSample::new(
                metric,
                firkin::trace::BenchmarkMetricKind::LifecycleLatency,
                firkin::trace::BenchmarkUnit::Milliseconds,
                value,
            )
            .with_static_tag("probe_surface", "browser_db_cli_readiness")
            .with_static_tag("measurement_boundary", "product_path_density_level")
            .with_static_tag("cli_boundary", "real_cli")
            .with_static_tag("browser_boundary", "real_browser_sidecar")
            .with_static_tag("database_boundary", "real_db_sidecar")
            .with_static_tag("pod_surface", "product_pod_ready_deck")
            .with_static_tag("excludes_container_add", "false")
            .with_static_tag("ready_signal", "agent_computer_ready_after_container_add")
            .with_static_tag("density_tier", tier)
            .with_static_tag("capacity_status", "pass")
        })
        .collect()
    }

    fn proxy_database_ready_samples(
        values: impl IntoIterator<Item = f64>,
    ) -> Vec<firkin::trace::BenchmarkSample> {
        values
            .into_iter()
            .map(|value| {
                firkin::trace::BenchmarkSample::new(
                    "product.database_ready_ms",
                    firkin::trace::BenchmarkMetricKind::LifecycleLatency,
                    firkin::trace::BenchmarkUnit::Milliseconds,
                    value,
                )
                .with_static_tag("measurement_boundary", "sqlite_proxy_not_db_sidecar")
            })
            .collect()
    }

    fn agent_computer_trace(
        lifecycle: firkin::trace::LifecycleClass,
        start_event: firkin::trace::SandboxEventName,
        ready_ns: u128,
    ) -> firkin::trace::SandboxEventTrace {
        agent_computer_trace_with_workload(
            lifecycle,
            firkin::trace::WorkloadClass::AgentComputer,
            start_event,
            ready_ns,
        )
    }

    fn agent_computer_trace_with_workload(
        lifecycle: firkin::trace::LifecycleClass,
        workload: firkin::trace::WorkloadClass,
        start_event: firkin::trace::SandboxEventName,
        ready_ns: u128,
    ) -> firkin::trace::SandboxEventTrace {
        let mut trace = firkin::trace::SandboxEventTrace::new();
        trace.push(firkin::trace::SandboxTraceEvent::new(
            start_event,
            0,
            lifecycle,
            workload,
            firkin::trace::RuntimeProfile::BrowserDbCli,
        ));
        trace.push(firkin::trace::SandboxTraceEvent::new(
            firkin::trace::SandboxEventName::AgentComputerSandboxCreated,
            50_000_000,
            lifecycle,
            workload,
            firkin::trace::RuntimeProfile::BrowserDbCli,
        ));
        trace.push(firkin::trace::SandboxTraceEvent::new(
            firkin::trace::SandboxEventName::AgentComputerProbeStart,
            75_000_000,
            lifecycle,
            workload,
            firkin::trace::RuntimeProfile::BrowserDbCli,
        ));
        trace.push(firkin::trace::SandboxTraceEvent::new(
            firkin::trace::SandboxEventName::BrowserReady,
            90_000_000,
            lifecycle,
            workload,
            firkin::trace::RuntimeProfile::BrowserDbCli,
        ));
        trace.push(firkin::trace::SandboxTraceEvent::new(
            firkin::trace::SandboxEventName::CliFirstUsefulStdout,
            100_000_000,
            lifecycle,
            workload,
            firkin::trace::RuntimeProfile::BrowserDbCli,
        ));
        trace.push(firkin::trace::SandboxTraceEvent::new(
            firkin::trace::SandboxEventName::DatabaseReady,
            ready_ns,
            lifecycle,
            workload,
            firkin::trace::RuntimeProfile::BrowserDbCli,
        ));
        trace.push(firkin::trace::SandboxTraceEvent::new(
            firkin::trace::SandboxEventName::AgentComputerReady,
            ready_ns,
            lifecycle,
            workload,
            firkin::trace::RuntimeProfile::BrowserDbCli,
        ));
        trace
    }

    #[test]
    fn writes_validates_and_reports_agent_scorecard_artifact() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let samples_path = tempdir.path().join("samples.json");
        let artifact = tempdir.path().join("scorecard.json");
        std::fs::write(
            &samples_path,
            serde_json::to_vec_pretty(&scorecard_samples((1..=100_u32).map(f64::from)))
                .expect("sample json"),
        )
        .expect("write samples");

        let write_args = WriteScorecardArgs {
            samples: samples_path,
            artifact: artifact.clone(),
            min_samples: 3,
        };
        let mut write_output = Vec::new();
        write_scorecard_artifact(&write_args, &mut write_output).expect("write scorecard");
        let write_output = String::from_utf8(write_output).expect("utf8");

        assert!(write_output.contains("scorecard_artifact=written"));
        assert!(write_output.contains(&format!(
            "required_metrics={}",
            firkin::evidence::P0_SCORECARD_METRICS.len()
        )));

        let validate_args = ValidateScorecardArgs {
            artifact: artifact.clone(),
            min_samples: 3,
            require_snappy: false,
        };
        let mut validate_output = Vec::new();
        validate_scorecard(&validate_args, &mut validate_output).expect("validate scorecard");
        let validate_output = String::from_utf8(validate_output).expect("utf8");

        assert!(validate_output.contains("scorecard=valid"));
        assert!(validate_output.contains("min_samples=3"));

        let report_args = ReportScorecardArgs { artifact };
        let mut report_output = Vec::new();
        write_scorecard_report(&report_args, &mut report_output).expect("report scorecard");
        let report_output = String::from_utf8(report_output).expect("utf8");

        assert!(report_output.contains("scorecard_report=summary"));
        assert!(report_output.contains(&format!(
            "metrics={}",
            firkin::evidence::P0_SCORECARD_METRICS.len()
        )));
        assert!(report_output.contains("metric=start.agent_task_ready_ms"));
        assert!(report_output.contains("count=100 p50=50 p90=90 p95=95 p99=99 max=100"));
    }

    #[test]
    fn writes_validates_and_reports_autoscale_scorecard_artifact() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let samples_path = tempdir.path().join("autoscale-samples.json");
        let artifact = tempdir.path().join("autoscale-scorecard.json");
        std::fs::write(
            &samples_path,
            serde_json::to_vec_pretty(&autoscale_scorecard_samples((1..=100_u32).map(f64::from)))
                .expect("sample json"),
        )
        .expect("write samples");
        let expected_promotion_blockers =
            firkin::evidence::AutoscaleEfficiencyScorecardReport::from_samples(
                autoscale_scorecard_samples((1..=100_u32).map(f64::from)),
            )
            .expect("expected autoscale report")
            .promotion_blockers()
            .len();

        let write_args = WriteAutoscaleScorecardArgs {
            samples: samples_path,
            artifact: artifact.clone(),
            min_samples: 3,
        };
        let mut write_output = Vec::new();
        write_autoscale_scorecard_artifact(&write_args, &mut write_output)
            .expect("write autoscale scorecard");
        let write_output = String::from_utf8(write_output).expect("utf8");

        assert!(write_output.contains("autoscale_scorecard_artifact=written"));
        assert!(write_output.contains(&format!(
            "required_metrics={}",
            firkin::evidence::AUTOSCALE_EFFICIENCY_SCORECARD_METRICS.len()
        )));

        let validate_args = ValidateAutoscaleScorecardArgs {
            artifact: artifact.clone(),
            min_samples: 3,
            require_promotable: false,
            require_snappy: false,
        };
        let mut validate_output = Vec::new();
        validate_autoscale_scorecard(&validate_args, &mut validate_output)
            .expect("validate autoscale scorecard");
        let validate_output = String::from_utf8(validate_output).expect("utf8");

        assert!(validate_output.contains("autoscale_scorecard=valid"));
        assert!(validate_output.contains("min_samples=3"));
        assert!(
            validate_output.contains(&format!("promotion_blockers={expected_promotion_blockers}"))
        );
        assert!(validate_output.contains("metric=autoscale.ready_queue_hit_rate_pct"));
        assert!(validate_output.contains("signed-live autoscale harness"));

        let report_args = ReportAutoscaleScorecardArgs {
            artifact: artifact.clone(),
        };
        let mut report_output = Vec::new();
        write_autoscale_scorecard_report(&report_args, &mut report_output)
            .expect("report autoscale scorecard");
        let report_output = String::from_utf8(report_output).expect("utf8");

        assert!(report_output.contains("autoscale_scorecard_report=summary"));
        assert!(report_output.contains(&format!(
            "metrics={}",
            firkin::evidence::AUTOSCALE_EFFICIENCY_SCORECARD_METRICS.len()
        )));
        assert!(report_output.contains("metric=autoscale.ready_queue_hit_rate_pct"));
        assert!(report_output.contains("count=100 p50=50 p90=90 p95=95 p99=99 max=100"));
        assert!(report_output.contains("promotion_blocker"));
        assert!(report_output.contains("unit_validated_only"));

        let validate_args = ValidateAutoscaleScorecardArgs {
            artifact: artifact.clone(),
            min_samples: 3,
            require_promotable: true,
            require_snappy: false,
        };
        let mut validate_output = Vec::new();
        let error = validate_autoscale_scorecard(&validate_args, &mut validate_output)
            .expect_err("autoscale promotion blockers fail promotable validation");
        let validate_output = String::from_utf8(validate_output).expect("utf8");

        assert!(
            validate_output.contains(&format!("promotion_blockers={expected_promotion_blockers}"))
        );
        assert!(error.to_string().contains("not promotion-grade"));
    }

    #[test]
    fn writes_validates_and_reports_agent_computer_scorecard_artifact() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let samples_path = tempdir.path().join("agent-computer-samples.json");
        let artifact = tempdir.path().join("agent-computer-scorecard.json");
        std::fs::write(
            &samples_path,
            serde_json::to_vec_pretty(&agent_computer_scorecard_samples(
                (1..=100_u32).map(f64::from),
            ))
            .expect("sample json"),
        )
        .expect("write samples");

        let write_args = WriteAgentComputerScorecardArgs {
            samples: samples_path,
            artifact: artifact.clone(),
            min_samples: 3,
        };
        let mut write_output = Vec::new();
        write_agent_computer_scorecard_artifact(&write_args, &mut write_output)
            .expect("write agent-computer scorecard");
        let write_output = String::from_utf8(write_output).expect("utf8");

        assert!(write_output.contains("agent_computer_scorecard_artifact=written"));
        assert!(write_output.contains(&format!(
            "required_metrics={}",
            firkin::evidence::AGENT_COMPUTER_SCORECARD_METRICS.len()
        )));

        let validate_args = ValidateAgentComputerScorecardArgs {
            artifact: artifact.clone(),
            min_samples: 3,
            require_promotable: false,
            require_snappy: false,
        };
        let mut validate_output = Vec::new();
        validate_agent_computer_scorecard(&validate_args, &mut validate_output)
            .expect("validate agent-computer scorecard");
        let validate_output = String::from_utf8(validate_output).expect("utf8");

        assert!(validate_output.contains("agent_computer_scorecard=valid"));
        assert!(validate_output.contains("min_samples=3"));
        assert!(validate_output.contains("promotion_blockers=0"));

        let report_args = ReportAgentComputerScorecardArgs {
            artifact: artifact.clone(),
        };
        let mut report_output = Vec::new();
        write_agent_computer_scorecard_report(&report_args, &mut report_output)
            .expect("report agent-computer scorecard");
        let report_output = String::from_utf8(report_output).expect("utf8");

        assert!(report_output.contains("agent_computer_scorecard_report=summary"));
        assert!(report_output.contains(&format!(
            "metrics={}",
            firkin::evidence::AGENT_COMPUTER_SCORECARD_METRICS.len()
        )));
        assert!(report_output.contains("metric=product.agent_computer_ready_ms"));
        assert!(report_output.contains("count=100 p50=50 p90=90 p95=95 p99=99 max=100"));

        let validate_args = ValidateAgentComputerScorecardArgs {
            artifact: artifact.clone(),
            min_samples: 3,
            require_promotable: true,
            require_snappy: true,
        };
        let mut validate_output = Vec::new();
        let error = validate_agent_computer_scorecard(&validate_args, &mut validate_output)
            .expect_err("slow but promotable agent-computer scorecard fails snappy validation");
        let validate_output = String::from_utf8(validate_output).expect("utf8");

        assert!(validate_output.contains("promotion_blockers=0"));
        assert!(validate_output.contains("snappy_target_status=miss"));
        assert!(validate_output.contains("snappy_target_miss"));
        assert!(validate_output.contains("metric=product.agent_computer_resume_ms"));
        assert!(error.to_string().contains("not snappy"));
    }

    #[test]
    fn reports_agent_computer_proxy_database_promotion_blocker() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let samples_path = tempdir.path().join("agent-computer-samples.json");
        let artifact = tempdir.path().join("agent-computer-scorecard.json");
        let mut samples = agent_computer_scorecard_samples((1..=100_u32).map(f64::from));
        samples.extend(proxy_database_ready_samples((1..=100_u32).map(f64::from)));
        std::fs::write(
            &samples_path,
            serde_json::to_vec_pretty(&samples).expect("sample json"),
        )
        .expect("write samples");

        let write_args = WriteAgentComputerScorecardArgs {
            samples: samples_path,
            artifact: artifact.clone(),
            min_samples: 3,
        };
        write_agent_computer_scorecard_artifact(&write_args, Vec::new())
            .expect("write agent-computer scorecard");

        let validate_args = ValidateAgentComputerScorecardArgs {
            artifact: artifact.clone(),
            min_samples: 3,
            require_promotable: false,
            require_snappy: false,
        };
        let mut validate_output = Vec::new();
        validate_agent_computer_scorecard(&validate_args, &mut validate_output)
            .expect("validate agent-computer scorecard");
        let validate_output = String::from_utf8(validate_output).expect("utf8");

        assert!(validate_output.contains("promotion_blockers=1"));
        assert!(validate_output.contains("metric=product.database_ready_ms"));
        assert!(validate_output.contains("SQLite through code-interpreter"));

        let report_args = ReportAgentComputerScorecardArgs {
            artifact: artifact.clone(),
        };
        let mut report_output = Vec::new();
        write_agent_computer_scorecard_report(&report_args, &mut report_output)
            .expect("report agent-computer scorecard");
        let report_output = String::from_utf8(report_output).expect("utf8");

        assert!(report_output.contains("promotion_blocker"));
        assert!(report_output.contains("real DB sidecar"));

        let validate_args = ValidateAgentComputerScorecardArgs {
            artifact,
            min_samples: 3,
            require_promotable: true,
            require_snappy: false,
        };
        let mut validate_output = Vec::new();
        let error = validate_agent_computer_scorecard(&validate_args, &mut validate_output)
            .expect_err("proxy DB blocker fails promotable validation");
        let validate_output = String::from_utf8(validate_output).expect("utf8");

        assert!(validate_output.contains("promotion_blockers=1"));
        assert!(error.to_string().contains("not promotion-grade"));
    }

    #[test]
    fn reports_agent_computer_trace_sidecar() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let artifact = tempdir.path().join("agent-computer-scorecard.traces.json");
        let traces = vec![
            agent_computer_trace(
                firkin::trace::LifecycleClass::Hot,
                firkin::trace::SandboxEventName::AgentComputerRequestStart,
                200_000_000,
            ),
            agent_computer_trace(
                firkin::trace::LifecycleClass::Resumed,
                firkin::trace::SandboxEventName::AgentComputerResumed,
                150_000_000,
            ),
            agent_computer_trace_with_workload(
                firkin::trace::LifecycleClass::Resumed,
                firkin::trace::WorkloadClass::ConcurrentCreate,
                firkin::trace::SandboxEventName::AgentComputerResumed,
                175_000_000,
            ),
        ];
        std::fs::write(
            &artifact,
            serde_json::to_vec_pretty(&traces).expect("trace json"),
        )
        .expect("write traces");

        let args = ReportAgentComputerTracesArgs { artifact };
        let mut output = Vec::new();
        write_agent_computer_trace_report(&args, &mut output)
            .expect("report agent-computer traces");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("agent_computer_trace_report=summary"));
        assert!(output.contains("kind=raw_trace_array"));
        assert!(output.contains("traces=3 overflowed=0"));
        assert!(output.contains("metric=product.agent_computer_ready_ms"));
        assert!(output.contains("metric=product.agent_computer_resume_ms"));
        assert!(output.contains("confidence=smoke_only"));
        assert!(
            output.contains(
                "trace=0 metric=product.agent_computer_ready_ms lifecycle=Hot workload=AgentComputer profile=BrowserDbCli"
            )
        );
        assert!(output.contains("total_ms=200"));
        assert!(output.contains("create_ms=50"));
        assert!(output.contains("probe_ms=125"));
        assert!(
            output.contains("trace=1 metric=product.agent_computer_resume_ms lifecycle=Resumed")
        );
        assert!(output.contains("total_ms=150"));
        assert!(output.contains("probe_ms=75"));
        assert!(output.contains(
            "trace=2 metric=debug.product.agent_computer_density_trace_ms lifecycle=Resumed workload=ConcurrentCreate"
        ));
    }

    #[test]
    fn reports_agent_computer_traces_from_proof_artifact() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let artifact = tempdir.path().join("product-pod-readiness.json");
        let trace = agent_computer_trace(
            firkin::trace::LifecycleClass::Hot,
            firkin::trace::SandboxEventName::AgentComputerRequestStart,
            250_000_000,
        );
        std::fs::write(
            &artifact,
            serde_json::to_vec_pretty(&serde_json::json!({
                "kind": "live_product_pod_readiness",
                "samples": [firkin::trace::BenchmarkSample::from_static(
                    "density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles",
                    firkin::trace::BenchmarkMetricKind::WorkloadResource,
                    firkin::trace::BenchmarkUnit::Count,
                    4.0,
                )
                .with_static_tag("measurement_boundary", "prestarted_slot_checkout")
                .with_static_tag("slot_surface", "prestarted_agent_slot")
                .with_static_tag("excludes_container_add", "true")],
                "traces": [trace],
            }))
            .expect("proof json"),
        )
        .expect("write proof artifact");

        let args = ReportAgentComputerTracesArgs { artifact };
        let mut output = Vec::new();
        write_agent_computer_trace_report(&args, &mut output)
            .expect("report proof artifact traces");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("agent_computer_trace_report=summary"));
        assert!(output.contains("kind=live_product_pod_readiness"));
        assert!(output.contains("traces=1 overflowed=0"));
        assert!(output.contains("explicit_metrics=1 explicit_samples=1"));
        assert!(output.contains("agent_computer_artifact_metric=summary"));
        assert!(output.contains(
            "metric=density.max_prestarted_agent_slots_before_checkout_ready_p95_doubles"
        ));
        assert!(output.contains("agent_computer_artifact_sample=tags"));
        assert!(output.contains("measurement_boundary=prestarted_slot_checkout"));
        assert!(output.contains("slot_surface=prestarted_agent_slot"));
        assert!(output.contains("excludes_container_add=true"));
        assert!(output.contains("total_ms=250"));
    }

    #[test]
    fn writes_benchmark_proof_html() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let source = tempdir.path().join("proof.txt");
        let out = tempdir.path().join("proof.html");
        std::fs::write(&source, "command=ok\nexit=0\n").expect("source");
        let args = BenchmarkProofArgs {
            milestone: "m1".to_owned(),
            source,
            out: out.clone(),
        };
        let mut output = Vec::new();

        write_benchmark_proof(&args, &mut output).expect("proof");
        let output = String::from_utf8(output).expect("utf8");
        let html = std::fs::read_to_string(out).expect("html");

        assert!(output.contains("benchmark_proof=written milestone=m1"));
        assert!(html.contains("Firkin Benchmark M1 Proof"));
        assert!(html.contains("command=ok"));
    }

    #[test]
    fn validates_soak_artifact_against_production_gate() {
        let artifact = std::env::temp_dir().join(format!(
            "firkin-cli-soak-evidence-{}.json",
            std::process::id()
        ));
        let benchmark_artifact = std::env::temp_dir().join(format!(
            "firkin-cli-soak-benchmark-evidence-{}.json",
            std::process::id()
        ));
        let samples = firkin::evidence::REQUIRED_LIFECYCLE_LATENCY_METRICS
            .iter()
            .flat_map(|metric| {
                [1.0, 2.0, 3.0].into_iter().map(move |value| {
                    firkin::trace::BenchmarkSample::new(
                        *metric,
                        firkin::trace::BenchmarkMetricKind::LifecycleLatency,
                        firkin::trace::BenchmarkUnit::Milliseconds,
                        value,
                    )
                })
            })
            .collect::<Vec<_>>();
        let benchmark_report = firkin::evidence::BenchmarkEvidenceReport::from_samples(samples)
            .expect("benchmark report");
        firkin::evidence::BenchmarkEvidenceArtifact::write_json(
            &benchmark_artifact,
            &benchmark_report,
        )
        .expect("benchmark artifact");
        let report = firkin::substrate::SoakEvidenceReport::new(
            std::time::Duration::from_hours(24),
            firkin::substrate::SoakStep::required_inspect_loop().map(|step| (step, 2, 0)),
        )
        .with_benchmark_artifact(benchmark_artifact.to_string_lossy())
        .with_cleanup_evidence(firkin::substrate::SoakCleanupEvidence::clean());
        firkin::substrate::SoakEvidenceArtifact::write_json(&artifact, &report).expect("artifact");
        let args = ValidateSoakArgs { artifact };
        let mut output = Vec::new();

        validate_soak(&args, &mut output).expect("soak passes");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains("soak_gate=passed"));
        assert!(output.contains("duration_seconds=86400"));
        assert!(output.contains("steps=7"));
        let _ = std::fs::remove_file(args.artifact);
        let _ = std::fs::remove_file(benchmark_artifact);
    }

    #[test]
    fn validate_soak_rejects_missing_benchmark_artifact() {
        let artifact = std::env::temp_dir().join(format!(
            "firkin-cli-soak-missing-benchmark-{}.json",
            std::process::id()
        ));
        let missing_benchmark = std::env::temp_dir().join(format!(
            "firkin-cli-missing-benchmark-{}.json",
            std::process::id()
        ));
        let report = firkin::substrate::SoakEvidenceReport::new(
            std::time::Duration::from_hours(24),
            firkin::substrate::SoakStep::required_inspect_loop().map(|step| (step, 2, 0)),
        )
        .with_benchmark_artifact(missing_benchmark.to_string_lossy())
        .with_cleanup_evidence(firkin::substrate::SoakCleanupEvidence::clean());
        firkin::substrate::SoakEvidenceArtifact::write_json(&artifact, &report).expect("artifact");
        let args = ValidateSoakArgs { artifact };
        let mut output = Vec::new();

        let error = validate_soak(&args, &mut output).expect_err("missing benchmark rejects soak");

        assert!(
            error
                .to_string()
                .contains("soak benchmark artifact missing"),
            "{error}"
        );
        let _ = std::fs::remove_file(args.artifact);
    }

    #[test]
    fn formats_substrate_acceptance_checklist_manifest() {
        let mut output = Vec::new();
        write_substrate_acceptance_checklist(&mut output).expect("acceptance checklist");
        let output = String::from_utf8(output).expect("utf8");

        assert!(output.contains(
            "check=template_build_snapshot status=signed_live_vz_template_snapshot_proven"
        ));
        assert!(
            output
                .contains("check=snapshot_restore_default_create status=core_live_restore_proven")
        );
        assert!(
            output.contains("check=warm_pool_lifecycle status=signed_live_product_route_proven")
        );
        assert!(output.contains("clean-prewarm-policy"));
        assert!(output.contains(
            "check=freshness_sync_readonly_then_writable status=signed_live_product_route_proven"
        ));
        assert!(output.contains(
            "check=continuation_snapshot_resume status=signed_live_product_route_proven"
        ));
        assert!(output.contains(
            "check=reserved_port_routing status=code_interpreter_python_context_smoke_proven"
        ));
        assert!(output.contains("python-context-id-persists"));
        assert!(output.contains("full-jupyter-kernel-parity"));
        assert!(output.contains("guest-mcp-service-semantics-deferred-to-v2"));
        assert!(output.contains(
            "check=envd_filesystem_operations status=signed_live_sdk_domain_proxy_filesystem_proven"
        ));
        assert!(output.contains("envd-http-server-grpc-web-text-watch-streaming-proof"));
        assert!(output.contains("grpc-web-text-watch-streams-start-and-filesystem-events"));
        assert!(output.contains(
            "check=envd_process_records status=signed_live_sdk_domain_proxy_process_proven"
        ));
        assert!(output.contains("check=stop_lifecycle status=signed_live_sdk_kill_delete_proven"));
        assert!(output.contains("check=latency_benchmarks status=representative_slo_gate_proven"));
        assert!(output.contains("check=overhead_benchmarks status=representative_slo_gate_proven"));
        assert!(
            output.contains("check=runtime_preflight status=product_and_adapter_preflight_wired")
        );
        assert!(output.contains(
            "check=snapshot_artifact_integrity status=signed_live_integrity_reject_proven"
        ));
        assert!(output.contains("fk-substrate-snapshot-sidecars"));
        assert!(output.contains(
            "check=capacity_scheduler_pressure status=runtime_active_queue_backpressure_wired"
        ));
        assert!(output.contains("firkin-substrate-active-backpressure-tests"));
        assert!(output.contains("firkin-runtime-e2b-adapter-active-queue-test"));
        assert!(output.contains("firkin-runtime-e2b-adapter-followup-queue-test"));
        assert!(output.contains("firkin-runtime-e2b-adapter-warm-disk-floor-test"));
        assert!(output.contains("adapter-prewarm-stop-at-20gib"));
        assert!(output.contains(
            "check=restart_reconciliation status=signed_live_vz_marker_host_scan_proven"
        ));
        assert!(output.contains("runtime-pid-and-executable-directory-marker"));
        assert!(output.contains("json-host-scan-output-includes-stuck-vm-runtime-pid"));
        assert!(output.contains("runtime-restart-recovery-owner-scans-reconciles"));
        assert!(output.contains("executable-checked-host-process-terminator"));
        assert!(output.contains("signed-live-vz-active-marker-host-scan-smoke"));
        assert!(
            output
                .contains("check=snapshot_artifact_gc status=signed_live_hygiene_pressure_proven")
        );
        assert!(output.contains(
            "check=snapshot_artifact_integrity status=signed_live_integrity_reject_proven"
        ));
        assert!(output.contains("check=log_rotation status=signed_live_hygiene_pressure_proven"));
        assert!(output.contains("signed-live-vz-hygiene-pressure-smoke"));
        assert!(
            output
                .contains("check=stuck_vm_cleanup status=signed_live_host_process_cleanup_proven")
        );
        assert!(output.contains("terminates-executable-matched-marked-host-process"));
        assert!(output.contains("signed-live-host-process-stuck-vm-cleanup-smoke"));
        assert!(
            output.contains("check=multi_container_vm_substrate status=core_live_smoke_proven")
        );
        assert!(output.contains("cubeapi-firkin-create-restore-single-vm-backed-container-tests"));
        assert!(output.contains("check=network_policy_hard_fail status=final_adapter_path_proven"));
        assert!(output.contains("check=single_node_24h_soak status=runner_smoke_proven"));
        assert!(output.contains("actual-24h-run-artifact-missing"));
    }

    #[test]
    fn e2b_domain_preflight_accepts_localhost_wildcard() {
        let domain: Hostname = "cube.localhost".parse().unwrap();
        preflight_e2b_proxy_domain(&domain, "127.0.0.1:49981".parse().unwrap()).unwrap();
        preflight_e2b_proxy_domain(&domain, "[::1]:49981".parse().unwrap()).unwrap();
    }

    #[test]
    fn e2b_domain_preflight_rejects_wrong_listener_address() {
        let domain: Hostname = "cube.localhost".parse().unwrap();
        let error = preflight_e2b_proxy_domain(&domain, "192.0.2.10:49981".parse().unwrap())
            .expect_err("wrong listener address");
        assert!(error.to_string().contains("not proxy listener"));
    }

    #[test]
    fn rejects_unknown_platform() {
        let error = client(Some("linux/sparc")).expect_err("bad platform");
        assert!(error.to_string().contains("unsupported platform"));
    }
}
