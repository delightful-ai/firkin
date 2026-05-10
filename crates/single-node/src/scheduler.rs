use std::collections::HashMap;
use std::sync::Mutex;

use super::{Error, Result, SandboxResources, SingleNodeSchedulerConfig};
use firkin_admission::{CapacityError, CapacityLedger, ResourceBudget};
use firkin_types::Size;

/// In-memory admission controller for one local Apple/VZ host.
#[derive(Debug)]
pub struct SingleNodeScheduler {
    config: SingleNodeSchedulerConfig,
    inner: Mutex<SchedulerState>,
}

impl SingleNodeScheduler {
    /// Construct a scheduler from explicit capacity limits.
    #[must_use]
    pub fn new(config: SingleNodeSchedulerConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(SchedulerState::new(config.resources())),
        }
    }

    /// Admit a sandbox reservation if host capacity allows it.
    ///
    /// Re-admitting the same sandbox ID is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::CapacityRejected`] when the request exceeds configured limits.
    pub fn admit(&self, sandbox_id: &str, resources: SandboxResources) -> Result<()> {
        let mut inner = self.lock()?;
        if inner.admitted.contains_key(sandbox_id) {
            return Ok(());
        }

        if inner.admitted.len() >= self.config.max_sessions() {
            return Err(Error::CapacityRejected(format!(
                "single-node scheduler rejected sandbox `{sandbox_id}`: max sessions {} reached",
                self.config.max_sessions()
            )));
        }

        inner
            .ledger
            .reserve_active(resources.to_budget())
            .map_err(|error| capacity_error(sandbox_id, error))?;
        inner.admitted.insert(sandbox_id.to_owned(), resources);
        Ok(())
    }

    /// Release an admitted sandbox reservation.
    ///
    /// Releasing an unknown sandbox ID is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if scheduler state cannot be locked.
    pub fn release(&self, sandbox_id: &str) -> Result<()> {
        let mut inner = self.lock()?;
        if let Some(resources) = inner.admitted.remove(sandbox_id) {
            inner.ledger.release_active(resources.to_budget());
        }
        Ok(())
    }

    /// Return currently admitted session count.
    ///
    /// # Errors
    ///
    /// Returns an error if scheduler state cannot be locked.
    pub fn admitted_len(&self) -> Result<usize> {
        Ok(self.lock()?.admitted.len())
    }

    /// Return currently available CPU and memory.
    ///
    /// # Errors
    ///
    /// Returns an error if scheduler state cannot be locked.
    pub fn available_resources(&self) -> Result<SandboxResources> {
        Ok(SandboxResources::from_budget(
            self.lock()?.ledger.available(),
        ))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SchedulerState>> {
        self.inner
            .lock()
            .map_err(|_| Error::CapacityRejected("single-node scheduler lock poisoned".to_owned()))
    }
}

impl Default for SingleNodeScheduler {
    fn default() -> Self {
        Self::new(SingleNodeSchedulerConfig::default())
    }
}

#[derive(Debug)]
struct SchedulerState {
    admitted: HashMap<String, SandboxResources>,
    ledger: CapacityLedger,
}

impl SchedulerState {
    fn new(resources: SandboxResources) -> Self {
        Self {
            admitted: HashMap::new(),
            ledger: CapacityLedger::new(resources.to_budget()),
        }
    }
}

impl SandboxResources {
    fn to_budget(self) -> ResourceBudget {
        ResourceBudget::new(self.vcpus, Size::bytes(self.memory_bytes), Size::bytes(0))
    }

    fn from_budget(budget: ResourceBudget) -> Self {
        Self::new(budget.cpus(), budget.memory())
    }
}

fn capacity_error(sandbox_id: &str, error: CapacityError) -> Error {
    match error {
        CapacityError::Cpu {
            requested,
            available,
        } => Error::CapacityRejected(format!(
            "single-node scheduler rejected sandbox `{sandbox_id}`: CPU requested {requested}, available {available}"
        )),
        CapacityError::Memory {
            requested,
            available,
        } => Error::CapacityRejected(format!(
            "single-node scheduler rejected sandbox `{sandbox_id}`: memory requested {requested}, available {available}"
        )),
        CapacityError::Disk {
            requested,
            available,
        } => Error::CapacityRejected(format!(
            "single-node scheduler rejected sandbox `{sandbox_id}`: disk requested {requested}, available {available}"
        )),
    }
}
