//! stats — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
use crate::StatCategory;
#[allow(unused_imports)]
use crate::pb;
/// Typed builder for `ContainerStatistics` requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerStatisticsQuery {
    container_ids: Vec<String>,
    categories: StatCategory,
}
impl ContainerStatisticsQuery {
    /// Construct a query for the given container IDs.
    #[must_use]
    pub fn new<I, S>(container_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            container_ids: container_ids.into_iter().map(Into::into).collect(),
            categories: StatCategory::all(),
        }
    }
    /// Set requested statistic categories.
    #[must_use]
    pub const fn categories(mut self, categories: StatCategory) -> Self {
        self.categories = categories;
        self
    }
    /// Return requested categories.
    #[must_use]
    pub const fn requested_categories(&self) -> StatCategory {
        self.categories
    }
    /// Convert into the generated protobuf request.
    #[must_use]
    pub fn into_request(self) -> pb::ContainerStatisticsRequest {
        pb::ContainerStatisticsRequest {
            container_ids: self.container_ids,
            categories: self.categories.proto_categories(),
        }
    }
}
/// Statistics for one container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerStatistics {
    /// Container ID reported by vminitd.
    pub id: String,
    /// Process pid statistics.
    pub process: Option<ProcessStatistics>,
    /// Memory usage statistics.
    pub memory: Option<MemoryStatistics>,
    /// CPU usage statistics.
    pub cpu: Option<CpuStatistics>,
    /// Block I/O statistics.
    pub block_io: Option<BlockIoStatistics>,
    /// Network interface statistics.
    pub networks: Option<Vec<NetworkStatistics>>,
    /// Memory event counters.
    pub memory_events: Option<MemoryEventStatistics>,
}
impl ContainerStatistics {
    /// Convert one generated protobuf response into a typed statistic value.
    #[must_use]
    pub fn from_proto(stats: pb::ContainerStats, categories: StatCategory) -> Self {
        Self {
            id: stats.container_id,
            process: if categories.wants(StatCategory::PROCESS) {
                stats.process.map(ProcessStatistics::from)
            } else {
                None
            },
            memory: if categories.wants(StatCategory::MEMORY) {
                stats.memory.map(MemoryStatistics::from)
            } else {
                None
            },
            cpu: if categories.wants(StatCategory::CPU) {
                stats.cpu.map(CpuStatistics::from)
            } else {
                None
            },
            block_io: if categories.wants(StatCategory::BLOCK_IO) {
                stats.block_io.map(BlockIoStatistics::from)
            } else {
                None
            },
            networks: if categories.wants(StatCategory::NETWORK) {
                Some(
                    stats
                        .networks
                        .into_iter()
                        .map(NetworkStatistics::from)
                        .collect(),
                )
            } else {
                None
            },
            memory_events: if categories.wants(StatCategory::MEMORY_EVENTS) {
                stats.memory_events.map(MemoryEventStatistics::from)
            } else {
                None
            },
        }
    }
    /// Convert a generated response into typed statistic values.
    #[must_use]
    pub fn list_from_response(
        response: pb::ContainerStatisticsResponse,
        categories: StatCategory,
    ) -> Vec<Self> {
        response
            .containers
            .into_iter()
            .map(|stats| Self::from_proto(stats, categories))
            .collect()
    }
}
/// Process pid statistics for a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessStatistics {
    /// Current process count.
    pub current: u64,
    /// Process limit. Zero means unlimited.
    pub limit: u64,
}
impl From<pb::ProcessStats> for ProcessStatistics {
    fn from(value: pb::ProcessStats) -> Self {
        Self {
            current: value.current,
            limit: value.limit,
        }
    }
}
/// Memory statistics for a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryStatistics {
    /// Current memory usage in bytes.
    pub usage_bytes: u64,
    /// Memory limit in bytes. Zero means unlimited.
    pub limit_bytes: u64,
    /// Current swap usage in bytes.
    pub swap_usage_bytes: u64,
    /// Swap limit in bytes. Zero means unlimited.
    pub swap_limit_bytes: u64,
    /// Page cache bytes.
    pub cache_bytes: u64,
    /// Kernel stack bytes.
    pub kernel_stack_bytes: u64,
    /// Slab bytes.
    pub slab_bytes: u64,
    /// Page fault count.
    pub page_faults: u64,
    /// Major page fault count.
    pub major_page_faults: u64,
    /// Inactive file bytes.
    pub inactive_file: u64,
    /// Anonymous memory bytes.
    pub anon: u64,
}
impl From<pb::MemoryStats> for MemoryStatistics {
    fn from(value: pb::MemoryStats) -> Self {
        Self {
            usage_bytes: value.usage_bytes,
            limit_bytes: value.limit_bytes,
            swap_usage_bytes: value.swap_usage_bytes,
            swap_limit_bytes: value.swap_limit_bytes,
            cache_bytes: value.cache_bytes,
            kernel_stack_bytes: value.kernel_stack_bytes,
            slab_bytes: value.slab_bytes,
            page_faults: value.page_faults,
            major_page_faults: value.major_page_faults,
            inactive_file: value.inactive_file,
            anon: value.anon,
        }
    }
}
/// CPU statistics for a container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuStatistics {
    /// Total CPU usage in microseconds.
    pub usage_usec: u64,
    /// User CPU usage in microseconds.
    pub user_usec: u64,
    /// System CPU usage in microseconds.
    pub system_usec: u64,
    /// CFS throttling periods.
    pub throttling_periods: u64,
    /// CFS throttled periods.
    pub throttled_periods: u64,
    /// CFS throttled time in microseconds.
    pub throttled_time_usec: u64,
}
impl From<pb::CpuStats> for CpuStatistics {
    fn from(value: pb::CpuStats) -> Self {
        Self {
            usage_usec: value.usage_usec,
            user_usec: value.user_usec,
            system_usec: value.system_usec,
            throttling_periods: value.throttling_periods,
            throttled_periods: value.throttled_periods,
            throttled_time_usec: value.throttled_time_usec,
        }
    }
}
/// Block I/O statistics for a container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockIoStatistics {
    /// Per-device block I/O counters.
    pub devices: Vec<BlockIoDevice>,
}
impl From<pb::BlockIoStats> for BlockIoStatistics {
    fn from(value: pb::BlockIoStats) -> Self {
        Self {
            devices: value.devices.into_iter().map(BlockIoDevice::from).collect(),
        }
    }
}
/// Block I/O statistics for a specific device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockIoDevice {
    /// Device major number.
    pub major: u64,
    /// Device minor number.
    pub minor: u64,
    /// Bytes read.
    pub read_bytes: u64,
    /// Bytes written.
    pub write_bytes: u64,
    /// Read operations.
    pub read_operations: u64,
    /// Write operations.
    pub write_operations: u64,
}
impl From<pb::BlockIoEntry> for BlockIoDevice {
    fn from(value: pb::BlockIoEntry) -> Self {
        Self {
            major: value.major,
            minor: value.minor,
            read_bytes: value.read_bytes,
            write_bytes: value.write_bytes,
            read_operations: value.read_operations,
            write_operations: value.write_operations,
        }
    }
}
/// Network statistics for one interface.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkStatistics {
    /// Interface name.
    pub interface: String,
    /// Received packet count.
    pub received_packets: u64,
    /// Transmitted packet count.
    pub transmitted_packets: u64,
    /// Received bytes.
    pub received_bytes: u64,
    /// Transmitted bytes.
    pub transmitted_bytes: u64,
    /// Receive errors.
    pub received_errors: u64,
    /// Transmit errors.
    pub transmitted_errors: u64,
}
impl From<pb::NetworkStats> for NetworkStatistics {
    fn from(value: pb::NetworkStats) -> Self {
        Self {
            interface: value.interface,
            received_packets: value.received_packets,
            transmitted_packets: value.transmitted_packets,
            received_bytes: value.received_bytes,
            transmitted_bytes: value.transmitted_bytes,
            received_errors: value.received_errors,
            transmitted_errors: value.transmitted_errors,
        }
    }
}
/// Memory event counters from cgroup2's `memory.events`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryEventStatistics {
    /// Low memory reclaim events.
    pub low: u64,
    /// High memory limit events.
    pub high: u64,
    /// Max memory limit events.
    pub max: u64,
    /// OOM events.
    pub oom: u64,
    /// OOM kill events.
    pub oom_kill: u64,
}
impl From<pb::MemoryEventStats> for MemoryEventStatistics {
    fn from(value: pb::MemoryEventStats) -> Self {
        Self {
            low: value.low,
            high: value.high,
            max: value.max,
            oom: value.oom,
            oom_kill: value.oom_kill,
        }
    }
}
