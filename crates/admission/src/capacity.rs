//! capacity — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::budget::{CapacityError, ResourceBudget};
#[allow(unused_imports)]
use firkin_types::Size;
/// Single-node resource ledger for active sandboxes and warm-pool reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapacityLedger {
    capacity: ResourceBudget,
    active: ResourceBudget,
    warm_pool: ResourceBudget,
}
impl CapacityLedger {
    /// Construct an empty ledger with the given total capacity.
    #[must_use]
    pub const fn new(capacity: ResourceBudget) -> Self {
        Self {
            capacity,
            active: ResourceBudget::new(0, Size::bytes(0), Size::bytes(0)),
            warm_pool: ResourceBudget::new(0, Size::bytes(0), Size::bytes(0)),
        }
    }
    /// Return total capacity.
    #[must_use]
    pub const fn capacity(self) -> ResourceBudget {
        self.capacity
    }
    /// Return active sandbox reservations.
    #[must_use]
    pub const fn active(self) -> ResourceBudget {
        self.active
    }
    /// Return warm-pool reservations.
    #[must_use]
    pub const fn warm_pool(self) -> ResourceBudget {
        self.warm_pool
    }
    /// Return all reserved resources.
    #[must_use]
    pub fn used(self) -> ResourceBudget {
        self.active + self.warm_pool
    }
    /// Return unreserved resources.
    #[must_use]
    pub fn available(self) -> ResourceBudget {
        self.capacity - self.used()
    }
    /// Reserve resources for an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`CapacityError`] when the request exceeds available capacity.
    pub fn reserve_active(
        &mut self,
        request: ResourceBudget,
    ) -> std::result::Result<(), CapacityError> {
        self.check_available(request)?;
        self.active = self.active + request;
        Ok(())
    }
    /// Reserve resources for a warm-pool entry.
    ///
    /// # Errors
    ///
    /// Returns [`CapacityError`] when the request exceeds available capacity.
    pub fn reserve_warm_pool(
        &mut self,
        request: ResourceBudget,
    ) -> std::result::Result<(), CapacityError> {
        self.check_available(request)?;
        self.warm_pool = self.warm_pool + request;
        Ok(())
    }
    /// Release resources from an active sandbox reservation.
    pub fn release_active(&mut self, budget: ResourceBudget) {
        self.active = self.active - budget;
    }
    /// Release resources from a warm-pool reservation.
    pub fn release_warm_pool(&mut self, budget: ResourceBudget) {
        self.warm_pool = self.warm_pool - budget;
    }
    /// Promote an existing warm-pool reservation into an active sandbox.
    ///
    /// # Errors
    ///
    /// Returns [`CapacityError`] when the requested promotion exceeds the
    /// currently reserved warm-pool resources.
    pub fn promote_warm_pool_to_active(
        &mut self,
        budget: ResourceBudget,
    ) -> std::result::Result<(), CapacityError> {
        if budget.cpus > self.warm_pool.cpus {
            return Err(CapacityError::Cpu {
                requested: budget.cpus,
                available: self.warm_pool.cpus,
            });
        }
        if budget.memory > self.warm_pool.memory {
            return Err(CapacityError::Memory {
                requested: budget.memory,
                available: self.warm_pool.memory,
            });
        }
        if budget.disk > self.warm_pool.disk {
            return Err(CapacityError::Disk {
                requested: budget.disk,
                available: self.warm_pool.disk,
            });
        }
        self.warm_pool = self.warm_pool - budget;
        self.active = self.active + budget;
        Ok(())
    }
    fn check_available(self, request: ResourceBudget) -> std::result::Result<(), CapacityError> {
        let available = self.available();
        if request.cpus > available.cpus {
            return Err(CapacityError::Cpu {
                requested: request.cpus,
                available: available.cpus,
            });
        }
        if request.memory > available.memory {
            return Err(CapacityError::Memory {
                requested: request.memory,
                available: available.memory,
            });
        }
        if request.disk > available.disk {
            return Err(CapacityError::Disk {
                requested: request.disk,
                available: available.disk,
            });
        }
        Ok(())
    }
}
