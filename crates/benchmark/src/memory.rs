//! Host memory benchmark helpers.
//!
//! These helpers sample host task footprint with `vmmap -summary` and compute
//! before/after deltas. Current-process sampling is useful signed-live overhead
//! evidence, but it is not exact per-sandbox attribution. Exact P0 promotion
//! requires a task-scoped source that proves the benchmark owns the full VZ VM
//! host task set and pairs the host delta with guest reclaim evidence.
#![allow(missing_docs)]

use firkin_trace::{BenchmarkMetricKind, BenchmarkSample, BenchmarkUnit};
use std::collections::BTreeSet;
use std::io;
use std::process::Command;
use thiserror::Error as ThisError;

pub const EXACT_MEMORY_ATTRIBUTION_COLLECTOR_REQUIREMENT: &str =
    "exclusive-vz-virtual-machine-task-set-footprint-with-paired-guest-reclaim";

pub const P0_MEMORY_ATTRIBUTION_BLOCKER: MemoryAttributionBlocker = MemoryAttributionBlocker {
    attribution_source: "current-process vmmap Physical footprint",
    blocker: "Virtualization.framework exposes configured memory and balloon target controls, but no per-VM resident footprint or per-VM host-memory statistics API; macOS task_info/TASK_VM_INFO is task-wide, current-process sampling can host multiple VZVirtualMachine objects, and guest cgroup memory is guest-scoped and cannot attribute host VZ backing pages",
    next_spike: "run with no pre-existing VZ VM host tasks, sample the exclusive com.apple.Virtualization.VirtualMachine task set with task_info(TASK_VM_INFO).phys_footprint or vmmap Physical footprint, and pair the host task delta with guest free/compact plus VZ balloon target changes",
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryAttributionBlocker {
    pub attribution_source: &'static str,
    pub blocker: &'static str,
    pub next_spike: &'static str,
}

impl MemoryAttributionBlocker {
    #[must_use]
    pub const fn is_exact_vm_scoped(self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostMemoryAttributionScope {
    ProcessWideHostTask,
    ExactOneVmPerHostTask,
    ExactExclusiveVzTaskSet,
}

impl HostMemoryAttributionScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProcessWideHostTask => "process-wide-host-task",
            Self::ExactOneVmPerHostTask => "exact-one-vm-per-host-task",
            Self::ExactExclusiveVzTaskSet => "exact-exclusive-vz-task-set",
        }
    }

    #[must_use]
    pub const fn is_exact_vm_scoped(self) -> bool {
        matches!(
            self,
            Self::ExactOneVmPerHostTask | Self::ExactExclusiveVzTaskSet
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttributedHostMemorySnapshot {
    footprint: HostFootprintSnapshot,
    scope: HostMemoryAttributionScope,
    source: &'static str,
    paired_guest_reclaim: bool,
}

impl AttributedHostMemorySnapshot {
    #[must_use]
    pub const fn new(
        footprint: HostFootprintSnapshot,
        scope: HostMemoryAttributionScope,
        source: &'static str,
        paired_guest_reclaim: bool,
    ) -> Self {
        Self {
            footprint,
            scope,
            source,
            paired_guest_reclaim,
        }
    }

    #[must_use]
    pub const fn footprint(self) -> HostFootprintSnapshot {
        self.footprint
    }

    #[must_use]
    pub const fn scope(self) -> HostMemoryAttributionScope {
        self.scope
    }

    #[must_use]
    pub const fn source(self) -> &'static str {
        self.source
    }

    #[must_use]
    pub const fn paired_guest_reclaim(self) -> bool {
        self.paired_guest_reclaim
    }
}

pub trait HostMemoryAttributionCollector {
    fn source(&self) -> &'static str;
    fn scope(&self) -> HostMemoryAttributionScope;
    fn paired_guest_reclaim(&self) -> bool;
    fn snapshot(&self) -> Result<AttributedHostMemorySnapshot, HostMemoryProbeError>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CurrentProcessVmmapCollector;

impl HostMemoryAttributionCollector for CurrentProcessVmmapCollector {
    fn source(&self) -> &'static str {
        "current-process-vmmap-physical-footprint"
    }

    fn scope(&self) -> HostMemoryAttributionScope {
        HostMemoryAttributionScope::ProcessWideHostTask
    }

    fn paired_guest_reclaim(&self) -> bool {
        false
    }

    fn snapshot(&self) -> Result<AttributedHostMemorySnapshot, HostMemoryProbeError> {
        current_process_host_footprint_snapshot().map(|footprint| {
            AttributedHostMemorySnapshot::new(
                footprint,
                self.scope(),
                self.source(),
                self.paired_guest_reclaim(),
            )
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SingleVzVirtualMachineVmmapCollector {
    paired_guest_reclaim: bool,
}

impl SingleVzVirtualMachineVmmapCollector {
    #[must_use]
    pub const fn new(paired_guest_reclaim: bool) -> Self {
        Self {
            paired_guest_reclaim,
        }
    }
}

impl HostMemoryAttributionCollector for SingleVzVirtualMachineVmmapCollector {
    fn source(&self) -> &'static str {
        "single-vz-virtual-machine-vmmap-physical-footprint"
    }

    fn scope(&self) -> HostMemoryAttributionScope {
        HostMemoryAttributionScope::ExactOneVmPerHostTask
    }

    fn paired_guest_reclaim(&self) -> bool {
        self.paired_guest_reclaim
    }

    fn snapshot(&self) -> Result<AttributedHostMemorySnapshot, HostMemoryProbeError> {
        let pid = single_vz_virtual_machine_pid()?;
        process_host_footprint_snapshot(pid).map(|footprint| {
            AttributedHostMemorySnapshot::new(
                footprint,
                self.scope(),
                self.source(),
                self.paired_guest_reclaim(),
            )
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExclusiveVzTaskSetVmmapCollector {
    baseline_pids: BTreeSet<u32>,
    paired_guest_reclaim: bool,
}

impl ExclusiveVzTaskSetVmmapCollector {
    #[must_use]
    pub fn new(baseline_pids: BTreeSet<u32>, paired_guest_reclaim: bool) -> Self {
        Self {
            baseline_pids,
            paired_guest_reclaim,
        }
    }
}

impl HostMemoryAttributionCollector for ExclusiveVzTaskSetVmmapCollector {
    fn source(&self) -> &'static str {
        "exclusive-vz-virtual-machine-task-set-vmmap-physical-footprint"
    }

    fn scope(&self) -> HostMemoryAttributionScope {
        HostMemoryAttributionScope::ExactExclusiveVzTaskSet
    }

    fn paired_guest_reclaim(&self) -> bool {
        self.paired_guest_reclaim
    }

    fn snapshot(&self) -> Result<AttributedHostMemorySnapshot, HostMemoryProbeError> {
        exclusive_vz_task_set_host_footprint_snapshot(&self.baseline_pids).map(|footprint| {
            AttributedHostMemorySnapshot::new(
                footprint,
                self.scope(),
                self.source(),
                self.paired_guest_reclaim(),
            )
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostFootprintSnapshot {
    bytes: u64,
}

impl HostFootprintSnapshot {
    #[must_use]
    pub const fn new(bytes: u64) -> Self {
        Self { bytes }
    }

    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostMemoryFootprint {
    pub idle_host_footprint_bytes: u64,
    pub post_task_residual_bytes: u64,
    pub reclaim_effectiveness_micros: u64,
}

impl HostMemoryFootprint {
    #[must_use]
    pub fn reclaim_effectiveness_ratio(self) -> f64 {
        self.reclaim_effectiveness_micros as f64 / 1_000_000.0
    }

    #[must_use]
    pub fn benchmark_samples(self) -> Vec<BenchmarkSample> {
        self.benchmark_samples_with_source("host-vmmap-physical-footprint")
    }

    #[must_use]
    pub fn benchmark_samples_with_source(self, source: &'static str) -> Vec<BenchmarkSample> {
        vec![
            memory_sample(
                "sandbox.mem.idle_host_footprint_bytes",
                BenchmarkUnit::Bytes,
                self.idle_host_footprint_bytes as f64,
                source,
            ),
            memory_sample(
                "sandbox.mem.post_task_residual_bytes",
                BenchmarkUnit::Bytes,
                self.post_task_residual_bytes as f64,
                source,
            ),
            memory_sample(
                "sandbox.mem.reclaim_effectiveness_ratio",
                BenchmarkUnit::Ratio,
                self.reclaim_effectiveness_ratio(),
                source,
            ),
        ]
    }
}

#[derive(Debug, ThisError)]
pub enum HostMemoryProbeError {
    #[error("host footprint probe failed to run vmmap for pid {pid}: {source}")]
    Command { pid: u32, source: io::Error },
    #[error("host footprint probe failed to discover VZ VM tasks with ps: {source}")]
    DiscoverVzTasks { source: io::Error },
    #[error("host footprint probe ps command failed while discovering VZ VM tasks: {stderr}")]
    DiscoverVzTasksStatus { stderr: String },
    #[error("host footprint probe found no com.apple.Virtualization.VirtualMachine task")]
    NoVzVirtualMachineTask,
    #[error(
        "host footprint probe found {count} com.apple.Virtualization.VirtualMachine tasks; exact attribution requires exactly one"
    )]
    MultipleVzVirtualMachineTasks { count: usize },
    #[error("host footprint probe found invalid VZ VM task pid: {value:?}")]
    InvalidVzVirtualMachinePid { value: String },
    #[error("host footprint probe vmmap failed for pid {pid}: {stderr}")]
    Status { pid: u32, stderr: String },
    #[error("host footprint probe vmmap output for pid {pid} is missing Physical footprint")]
    MissingPhysicalFootprint { pid: u32 },
    #[error("host footprint probe returned invalid Physical footprint for pid {pid}: {value:?}")]
    InvalidPhysicalFootprint { pid: u32, value: String },
    #[error("host footprint value for pid {pid} overflows bytes: {value:?}")]
    Overflow { pid: u32, value: String },
}

#[derive(Debug, ThisError)]
pub enum MemoryAttributionPromotionError {
    #[error(
        "memory attribution source {source_name} has scope {scope}; exact P0 promotion requires {required}"
    )]
    NonExactScope {
        source_name: &'static str,
        scope: &'static str,
        required: &'static str,
    },
    #[error(
        "memory attribution source {source_name} is missing paired guest free/compact plus balloon or recycle reclaim evidence"
    )]
    MissingPairedGuestReclaim { source_name: &'static str },
    #[error("memory attribution snapshots mix sources: {first} and {second}")]
    MixedSources {
        first: &'static str,
        second: &'static str,
    },
}

#[must_use]
pub fn host_memory_footprint_from_snapshots(
    baseline: HostFootprintSnapshot,
    idle: HostFootprintSnapshot,
    post_task: HostFootprintSnapshot,
    after_reclaim: HostFootprintSnapshot,
) -> HostMemoryFootprint {
    let idle_delta = idle.bytes().saturating_sub(baseline.bytes());
    let task_growth = post_task.bytes().saturating_sub(idle.bytes());
    let residual = after_reclaim.bytes().saturating_sub(idle.bytes());
    let reclaimed = post_task.bytes().saturating_sub(after_reclaim.bytes());
    let reclaim_effectiveness_micros = reclaimed
        .saturating_mul(1_000_000)
        .checked_div(task_growth)
        .unwrap_or(1_000_000)
        .min(1_000_000);

    HostMemoryFootprint {
        idle_host_footprint_bytes: idle_delta,
        post_task_residual_bytes: residual,
        reclaim_effectiveness_micros,
    }
}

pub fn exact_host_memory_footprint_from_attributed_snapshots(
    baseline: AttributedHostMemorySnapshot,
    idle: AttributedHostMemorySnapshot,
    post_task: AttributedHostMemorySnapshot,
    after_reclaim: AttributedHostMemorySnapshot,
) -> Result<HostMemoryFootprint, MemoryAttributionPromotionError> {
    let snapshots = [baseline, idle, post_task, after_reclaim];
    let source = baseline.source();
    for snapshot in snapshots {
        if snapshot.source() != source {
            return Err(MemoryAttributionPromotionError::MixedSources {
                first: source,
                second: snapshot.source(),
            });
        }
        if !snapshot.scope().is_exact_vm_scoped() {
            return Err(MemoryAttributionPromotionError::NonExactScope {
                source_name: snapshot.source(),
                scope: snapshot.scope().as_str(),
                required: EXACT_MEMORY_ATTRIBUTION_COLLECTOR_REQUIREMENT,
            });
        }
        if !snapshot.paired_guest_reclaim() {
            return Err(MemoryAttributionPromotionError::MissingPairedGuestReclaim {
                source_name: snapshot.source(),
            });
        }
    }
    Ok(host_memory_footprint_from_snapshots(
        baseline.footprint(),
        idle.footprint(),
        post_task.footprint(),
        after_reclaim.footprint(),
    ))
}

pub fn current_process_host_footprint_snapshot()
-> Result<HostFootprintSnapshot, HostMemoryProbeError> {
    process_host_footprint_snapshot(std::process::id())
}

pub fn single_vz_virtual_machine_host_footprint_snapshot()
-> Result<HostFootprintSnapshot, HostMemoryProbeError> {
    process_host_footprint_snapshot(single_vz_virtual_machine_pid()?)
}

pub fn exclusive_vz_task_set_host_footprint_snapshot(
    baseline_pids: &BTreeSet<u32>,
) -> Result<HostFootprintSnapshot, HostMemoryProbeError> {
    let mut bytes = 0_u64;
    for pid in vz_virtual_machine_pids()? {
        if baseline_pids.contains(&pid) {
            continue;
        }
        bytes = bytes.saturating_add(process_host_footprint_snapshot(pid)?.bytes());
    }
    Ok(HostFootprintSnapshot::new(bytes))
}

pub fn vz_virtual_machine_pid_set() -> Result<BTreeSet<u32>, HostMemoryProbeError> {
    Ok(vz_virtual_machine_pids()?.into_iter().collect())
}

pub fn vz_virtual_machine_pids() -> Result<Vec<u32>, HostMemoryProbeError> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,comm="])
        .output()
        .map_err(|source| HostMemoryProbeError::DiscoverVzTasks { source })?;
    if !output.status.success() {
        return Err(HostMemoryProbeError::DiscoverVzTasksStatus {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    vz_virtual_machine_pids_from_ps(&String::from_utf8_lossy(&output.stdout))
}

pub fn single_vz_virtual_machine_pid() -> Result<u32, HostMemoryProbeError> {
    match optional_single_vz_virtual_machine_pid()? {
        Some(pid) => Ok(pid),
        None => Err(HostMemoryProbeError::NoVzVirtualMachineTask),
    }
}

pub fn optional_single_vz_virtual_machine_pid() -> Result<Option<u32>, HostMemoryProbeError> {
    optional_single_vz_virtual_machine_pid_from_pids(&vz_virtual_machine_pids()?)
}

pub fn single_vz_virtual_machine_pid_from_ps(stdout: &str) -> Result<u32, HostMemoryProbeError> {
    match optional_single_vz_virtual_machine_pid_from_ps(stdout)? {
        Some(pid) => Ok(pid),
        None => Err(HostMemoryProbeError::NoVzVirtualMachineTask),
    }
}

pub fn optional_single_vz_virtual_machine_pid_from_ps(
    stdout: &str,
) -> Result<Option<u32>, HostMemoryProbeError> {
    let pids = vz_virtual_machine_pids_from_ps(stdout)?;
    optional_single_vz_virtual_machine_pid_from_pids(&pids)
}

fn optional_single_vz_virtual_machine_pid_from_pids(
    pids: &[u32],
) -> Result<Option<u32>, HostMemoryProbeError> {
    match pids {
        [pid] => Ok(Some(*pid)),
        [] => Ok(None),
        _ => Err(HostMemoryProbeError::MultipleVzVirtualMachineTasks { count: pids.len() }),
    }
}

pub fn vz_virtual_machine_pids_from_ps(stdout: &str) -> Result<Vec<u32>, HostMemoryProbeError> {
    let mut pids = Vec::new();
    for line in stdout.lines() {
        if !line.contains("com.apple.Virtualization.VirtualMachine") {
            continue;
        }
        let pid = line.split_whitespace().next().ok_or_else(|| {
            HostMemoryProbeError::InvalidVzVirtualMachinePid {
                value: line.to_owned(),
            }
        })?;
        pids.push(pid.parse::<u32>().map_err(|_| {
            HostMemoryProbeError::InvalidVzVirtualMachinePid {
                value: pid.to_owned(),
            }
        })?);
    }
    Ok(pids)
}

pub fn process_host_footprint_snapshot(
    pid: u32,
) -> Result<HostFootprintSnapshot, HostMemoryProbeError> {
    let output = Command::new("vmmap")
        .args(["-summary", &pid.to_string()])
        .output()
        .map_err(|source| HostMemoryProbeError::Command { pid, source })?;
    if !output.status.success() {
        return Err(HostMemoryProbeError::Status {
            pid,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    parse_vmmap_physical_footprint_bytes(pid, &String::from_utf8_lossy(&output.stdout))
        .map(HostFootprintSnapshot::new)
}

pub fn parse_vmmap_physical_footprint_bytes(
    pid: u32,
    stdout: &str,
) -> Result<u64, HostMemoryProbeError> {
    let raw = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("Physical footprint:"))
        .map(str::trim)
        .ok_or(HostMemoryProbeError::MissingPhysicalFootprint { pid })?;
    parse_vmmap_size_bytes(pid, raw)
}

fn parse_vmmap_size_bytes(pid: u32, raw: &str) -> Result<u64, HostMemoryProbeError> {
    let value = raw.trim();
    let unit_start = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .ok_or_else(|| HostMemoryProbeError::InvalidPhysicalFootprint {
            pid,
            value: value.to_owned(),
        })?;
    let (number, unit) = value.split_at(unit_start);
    let multiplier = match unit.trim() {
        "B" => 1_f64,
        "K" => 1024_f64,
        "M" => 1024_f64 * 1024_f64,
        "G" => 1024_f64 * 1024_f64 * 1024_f64,
        "T" => 1024_f64 * 1024_f64 * 1024_f64 * 1024_f64,
        _ => {
            return Err(HostMemoryProbeError::InvalidPhysicalFootprint {
                pid,
                value: value.to_owned(),
            });
        }
    };
    let bytes =
        number
            .parse::<f64>()
            .map_err(|_| HostMemoryProbeError::InvalidPhysicalFootprint {
                pid,
                value: value.to_owned(),
            })?
            * multiplier;
    if !bytes.is_finite() || bytes < 0.0 || bytes > u64::MAX as f64 {
        return Err(HostMemoryProbeError::Overflow {
            pid,
            value: value.to_owned(),
        });
    }
    Ok(bytes.round() as u64)
}

fn memory_sample(
    metric: &'static str,
    unit: BenchmarkUnit,
    value: f64,
    source: &'static str,
) -> BenchmarkSample {
    BenchmarkSample::from_static(metric, BenchmarkMetricKind::WorkloadResource, unit, value)
        .with_static_tag("source", source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vmmap_physical_footprint_line() {
        assert_eq!(
            parse_vmmap_physical_footprint_bytes(
                42,
                "Process: zsh [42]\nPhysical footprint:         17.5M\n",
            )
            .unwrap(),
            18_350_080
        );
    }

    #[test]
    fn rejects_vmmap_output_without_physical_footprint() {
        assert!(matches!(
            parse_vmmap_physical_footprint_bytes(42, "Physical footprint (peak): 18M\n"),
            Err(HostMemoryProbeError::MissingPhysicalFootprint { pid: 42 })
        ));
    }

    #[test]
    fn host_memory_footprint_uses_idle_residual_and_reclaim_deltas() {
        let footprint = host_memory_footprint_from_snapshots(
            HostFootprintSnapshot::new(1_000),
            HostFootprintSnapshot::new(1_400),
            HostFootprintSnapshot::new(2_400),
            HostFootprintSnapshot::new(1_650),
        );

        assert_eq!(footprint.idle_host_footprint_bytes, 400);
        assert_eq!(footprint.post_task_residual_bytes, 250);
        assert_eq!(footprint.reclaim_effectiveness_ratio(), 0.75);
    }

    #[test]
    fn host_memory_footprint_treats_no_task_growth_as_fully_reclaimed() {
        let footprint = host_memory_footprint_from_snapshots(
            HostFootprintSnapshot::new(1_000),
            HostFootprintSnapshot::new(1_400),
            HostFootprintSnapshot::new(1_300),
            HostFootprintSnapshot::new(1_200),
        );

        assert_eq!(footprint.idle_host_footprint_bytes, 400);
        assert_eq!(footprint.post_task_residual_bytes, 0);
        assert_eq!(footprint.reclaim_effectiveness_ratio(), 1.0);
    }

    #[test]
    fn p0_memory_attribution_blocker_rejects_exact_vm_scope() {
        let blocker = P0_MEMORY_ATTRIBUTION_BLOCKER;

        assert!(!blocker.is_exact_vm_scoped());
        assert!(blocker.blocker.contains("no per-VM resident footprint"));
        assert!(blocker.blocker.contains("per-VM host-memory statistics"));
        assert!(blocker.blocker.contains("task_info"));
        assert!(blocker.blocker.contains("guest cgroup"));
        assert!(
            blocker
                .next_spike
                .contains("no pre-existing VZ VM host tasks")
        );
        assert!(blocker.next_spike.contains("guest free/compact"));
        assert!(blocker.next_spike.contains("VZ balloon target"));
    }

    #[test]
    fn current_vmmap_collector_declares_process_wide_proxy_scope() {
        let collector = CurrentProcessVmmapCollector;

        assert_eq!(
            collector.scope(),
            HostMemoryAttributionScope::ProcessWideHostTask
        );
        assert!(!collector.scope().is_exact_vm_scoped());
        assert!(!collector.paired_guest_reclaim());
    }

    #[test]
    fn parses_single_vz_virtual_machine_task_from_ps() {
        let stdout = "\
          101 /usr/libexec/something
          202 /System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine
        ";

        assert_eq!(single_vz_virtual_machine_pid_from_ps(stdout).unwrap(), 202);
    }

    #[test]
    fn rejects_missing_vz_virtual_machine_task() {
        assert!(matches!(
            single_vz_virtual_machine_pid_from_ps("101 /usr/libexec/something\n"),
            Err(HostMemoryProbeError::NoVzVirtualMachineTask)
        ));
        assert_eq!(
            optional_single_vz_virtual_machine_pid_from_ps("101 /usr/libexec/something\n").unwrap(),
            None
        );
    }

    #[test]
    fn rejects_multiple_vz_virtual_machine_tasks() {
        let stdout = "\
          202 /System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine
          303 /System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine
        ";

        assert!(matches!(
            single_vz_virtual_machine_pid_from_ps(stdout),
            Err(HostMemoryProbeError::MultipleVzVirtualMachineTasks { count: 2 })
        ));
    }

    #[test]
    fn single_vz_collector_declares_exact_scope_but_requires_reclaim_flag() {
        let collector = SingleVzVirtualMachineVmmapCollector::new(true);

        assert_eq!(
            collector.scope(),
            HostMemoryAttributionScope::ExactOneVmPerHostTask
        );
        assert!(collector.scope().is_exact_vm_scoped());
        assert!(collector.paired_guest_reclaim());
    }

    #[test]
    fn exclusive_vz_task_set_collector_declares_exact_scope() {
        let collector = ExclusiveVzTaskSetVmmapCollector::new(BTreeSet::new(), true);

        assert_eq!(
            collector.scope(),
            HostMemoryAttributionScope::ExactExclusiveVzTaskSet
        );
        assert!(collector.scope().is_exact_vm_scoped());
        assert!(collector.paired_guest_reclaim());
    }

    #[test]
    fn exact_memory_promotion_rejects_process_wide_collector_snapshots() {
        let snapshot = AttributedHostMemorySnapshot::new(
            HostFootprintSnapshot::new(1_000),
            HostMemoryAttributionScope::ProcessWideHostTask,
            "current-process-vmmap-physical-footprint",
            false,
        );

        let error = exact_host_memory_footprint_from_attributed_snapshots(
            snapshot, snapshot, snapshot, snapshot,
        )
        .expect_err("process-wide attribution must not promote");

        assert!(matches!(
            error,
            MemoryAttributionPromotionError::NonExactScope { .. }
        ));
        assert!(
            error
                .to_string()
                .contains(EXACT_MEMORY_ATTRIBUTION_COLLECTOR_REQUIREMENT)
        );
    }

    #[test]
    fn exact_memory_promotion_accepts_one_vm_task_with_paired_reclaim() {
        let snapshot = |bytes| {
            AttributedHostMemorySnapshot::new(
                HostFootprintSnapshot::new(bytes),
                HostMemoryAttributionScope::ExactOneVmPerHostTask,
                "signed-helper-task-info-phys-footprint",
                true,
            )
        };

        let footprint = exact_host_memory_footprint_from_attributed_snapshots(
            snapshot(1_000),
            snapshot(1_400),
            snapshot(2_400),
            snapshot(1_650),
        )
        .expect("exact one-VM task attribution");

        assert_eq!(footprint.idle_host_footprint_bytes, 400);
        assert_eq!(footprint.post_task_residual_bytes, 250);
        assert_eq!(footprint.reclaim_effectiveness_ratio(), 0.75);
    }
}
