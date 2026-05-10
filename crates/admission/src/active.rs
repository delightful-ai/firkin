//! active — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::budget::{CapacityError, ResourceBudget};
#[allow(unused_imports)]
use crate::budget_fits;
#[allow(unused_imports)]
use crate::capacity::CapacityLedger;
#[allow(unused_imports)]
use crate::warm_pool::{WarmPoolEntry, WarmPoolLedger};
/// Active sandbox admission plan against current capacity and retained warm entries.
///
/// Active requests have priority over background warm-pool inventory. When a
/// request does not fit immediately, this plan lists the deterministic warm
/// entries that should be evicted before rejecting the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveCapacityAdmissionPlan {
    request: ResourceBudget,
    evictions: Vec<WarmPoolEntry>,
    error: Option<CapacityError>,
}
impl ActiveCapacityAdmissionPlan {
    /// Plan active sandbox admission using warm-pool entries as reclaimable capacity.
    #[must_use]
    pub fn from_warm_pool(
        request: ResourceBudget,
        pool: &WarmPoolLedger,
        capacity: CapacityLedger,
    ) -> Self {
        let mut available = capacity.available();
        let mut evictions = Vec::new();
        if !budget_fits(request, available) {
            'entries: for entries in pool.entries.values() {
                for entry in entries {
                    available = available + entry.budget();
                    evictions.push(entry.clone());
                    if budget_fits(request, available) {
                        break 'entries;
                    }
                }
            }
        }
        let error = capacity_error_for_request(request, available);
        Self {
            request,
            evictions,
            error,
        }
    }
    /// Return the active resource request being planned.
    #[must_use]
    pub const fn request(&self) -> ResourceBudget {
        self.request
    }
    /// Return warm entries that should be evicted before active reservation.
    #[must_use]
    pub fn evictions(&self) -> &[WarmPoolEntry] {
        &self.evictions
    }
    /// Return whether the active request can be admitted after planned evictions.
    #[must_use]
    pub const fn is_admitted(&self) -> bool {
        self.error.is_none()
    }
    /// Return the capacity error that remains after all planned evictions.
    #[must_use]
    pub const fn error(&self) -> Option<CapacityError> {
        self.error
    }
}
/// Queue policy for active sandbox work that cannot be admitted immediately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveQueuePolicy {
    max_pending: usize,
}
impl ActiveQueuePolicy {
    /// Construct an active-work queue policy.
    #[must_use]
    pub const fn new(max_pending: usize) -> Self {
        Self { max_pending }
    }
    /// Return the maximum number of active requests allowed to wait.
    #[must_use]
    pub const fn max_pending(self) -> usize {
        self.max_pending
    }
}
/// Backpressure decision for one active sandbox request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveBackpressureDecision {
    /// Request can reserve capacity immediately after planned warm evictions.
    AdmitNow,
    /// Request is possible, but must wait for active capacity to be released.
    Queue,
    /// Request should fail instead of entering the queue.
    Reject,
}
/// Reason an active request is rejected by the backpressure policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackpressureRejection {
    /// The request exceeds total single-node capacity and can never fit.
    Oversized(CapacityError),
    /// The active waiting queue already reached the configured bound.
    QueueFull {
        /// Maximum configured pending active requests.
        max_pending: usize,
        /// Current pending active requests.
        pending: usize,
    },
}
/// Active sandbox backpressure plan against current capacity and warm entries.
///
/// This is the queue-aware layer above immediate admission. It keeps active
/// work prioritized over optional warm-pool entries, queues only requests that
/// can fit after active releases, and rejects impossible or over-queued work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveBackpressurePlan {
    request: ResourceBudget,
    #[allow(missing_docs)]
    pub decision: ActiveBackpressureDecision,
    evictions: Vec<WarmPoolEntry>,
    queue_position: Option<usize>,
    rejection: Option<BackpressureRejection>,
}
impl ActiveBackpressurePlan {
    /// Plan active sandbox admission plus bounded queue backpressure.
    #[must_use]
    pub fn from_warm_pool(
        request: ResourceBudget,
        pool: &WarmPoolLedger,
        capacity: CapacityLedger,
        pending_active: usize,
        queue_policy: ActiveQueuePolicy,
    ) -> Self {
        let admission = ActiveCapacityAdmissionPlan::from_warm_pool(request, pool, capacity);
        if admission.is_admitted() {
            return Self {
                request,
                decision: ActiveBackpressureDecision::AdmitNow,
                evictions: admission.evictions().to_vec(),
                queue_position: None,
                rejection: None,
            };
        }
        if let Some(error) = capacity_error_for_request(request, capacity.capacity()) {
            return Self {
                request,
                decision: ActiveBackpressureDecision::Reject,
                evictions: Vec::new(),
                queue_position: None,
                rejection: Some(BackpressureRejection::Oversized(error)),
            };
        }
        if pending_active >= queue_policy.max_pending() {
            return Self {
                request,
                decision: ActiveBackpressureDecision::Reject,
                evictions: Vec::new(),
                queue_position: None,
                rejection: Some(BackpressureRejection::QueueFull {
                    max_pending: queue_policy.max_pending(),
                    pending: pending_active,
                }),
            };
        }
        Self {
            request,
            decision: ActiveBackpressureDecision::Queue,
            evictions: Vec::new(),
            queue_position: Some(pending_active + 1),
            rejection: None,
        }
    }
    /// Return the active resource request being planned.
    #[must_use]
    pub const fn request(&self) -> ResourceBudget {
        self.request
    }
    /// Return the queue-aware admission decision.
    #[must_use]
    pub const fn decision(&self) -> ActiveBackpressureDecision {
        self.decision
    }
    /// Return warm entries that should be evicted before immediate admission.
    #[must_use]
    pub fn evictions(&self) -> &[WarmPoolEntry] {
        &self.evictions
    }
    /// Return the one-based queue position for queued active work.
    #[must_use]
    pub const fn queue_position(&self) -> Option<usize> {
        self.queue_position
    }
    /// Return why the request was rejected, if it was rejected.
    #[must_use]
    pub const fn rejection(&self) -> Option<BackpressureRejection> {
        self.rejection
    }
}
fn capacity_error_for_request(
    request: ResourceBudget,
    available: ResourceBudget,
) -> Option<CapacityError> {
    if request.cpus > available.cpus {
        return Some(CapacityError::Cpu {
            requested: request.cpus,
            available: available.cpus,
        });
    }
    if request.memory > available.memory {
        return Some(CapacityError::Memory {
            requested: request.memory,
            available: available.memory,
        });
    }
    if request.disk > available.disk {
        return Some(CapacityError::Disk {
            requested: request.disk,
            available: available.disk,
        });
    }
    None
}
