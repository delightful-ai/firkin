//! budget — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use firkin_types::Size;
#[allow(unused_imports)]
use thiserror::Error as ThisError;
/// Host resources reserved by active sandboxes or warm-pool entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceBudget {
    pub(crate) cpus: u32,
    pub(crate) memory: Size,
    pub(crate) disk: Size,
}
impl ResourceBudget {
    /// Construct a resource budget.
    #[must_use]
    pub const fn new(cpus: u32, memory: Size, disk: Size) -> Self {
        Self { cpus, memory, disk }
    }
    /// Return CPU slots.
    #[must_use]
    pub const fn cpus(self) -> u32 {
        self.cpus
    }
    /// Return memory bytes.
    #[must_use]
    pub const fn memory(self) -> Size {
        self.memory
    }
    /// Return disk bytes.
    #[must_use]
    pub const fn disk(self) -> Size {
        self.disk
    }
}
impl std::ops::Add for ResourceBudget {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self {
            cpus: self.cpus.saturating_add(rhs.cpus),
            memory: self.memory + rhs.memory,
            disk: self.disk + rhs.disk,
        }
    }
}
impl std::ops::Sub for ResourceBudget {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            cpus: self.cpus.saturating_sub(rhs.cpus),
            memory: self.memory - rhs.memory,
            disk: self.disk - rhs.disk,
        }
    }
}
/// Capacity admission error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ThisError)]
pub enum CapacityError {
    /// Not enough CPU slots are available.
    #[error("insufficient CPU slots: requested {requested}, available {available}")]
    Cpu {
        /// Requested CPU slots.
        requested: u32,
        /// Available CPU slots.
        available: u32,
    },
    /// Not enough memory is available.
    #[error("insufficient memory: requested {requested}, available {available}")]
    Memory {
        /// Requested memory.
        requested: Size,
        /// Available memory.
        available: Size,
    },
    /// Not enough disk is available.
    #[error("insufficient disk: requested {requested}, available {available}")]
    Disk {
        /// Requested disk.
        requested: Size,
        /// Available disk.
        available: Size,
    },
}
