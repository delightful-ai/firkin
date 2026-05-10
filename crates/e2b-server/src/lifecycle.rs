//! lifecycle — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::backend::LocalRuntimeBackend;
#[allow(unused_imports)]
use firkin_e2b_contract::{BackendError, RuntimeAdapter, SandboxExpiration};
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use time::OffsetDateTime;
#[allow(unused_imports)]
use time::format_description::well_known::Rfc3339;
#[allow(unused_imports)]
use tokio::sync::Mutex;
#[allow(unused_imports)]
use tokio::task::JoinHandle;
/// Clock used by the lifecycle scheduler.
pub trait LifecycleClock: Clone + Send + Sync + 'static {
    /// Return the current time as a sortable RFC3339 timestamp.
    fn now_rfc3339(&self) -> String;
}
/// Lifecycle clock backed by the host system UTC clock.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemLifecycleClock;
impl LifecycleClock for SystemLifecycleClock {
    fn now_rfc3339(&self) -> String {
        OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("RFC3339 formatting current UTC time is infallible")
    }
}
/// Periodic timeout-expiration scheduler for a local E2B backend.
#[derive(Clone, Debug)]
pub struct LifecycleScheduler<A, C = SystemLifecycleClock> {
    #[allow(missing_docs)]
    pub backend: Arc<Mutex<LocalRuntimeBackend<A>>>,
    pub(crate) state_path: Option<PathBuf>,
    pub(crate) clock: C,
    pub(crate) interval: Duration,
}
impl<A, C> LifecycleScheduler<A, C>
where
    A: RuntimeAdapter,
    C: LifecycleClock,
{
    /// Return the scheduler interval.
    #[must_use]
    pub const fn interval(&self) -> Duration {
        self.interval
    }
    /// Apply one lifecycle expiration pass.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors, registry errors, or persistent state
    /// save failures.
    pub async fn tick(&self) -> Result<Vec<SandboxExpiration>, BackendError> {
        let now = self.clock.now_rfc3339();
        let mut backend = self.backend.lock().await;
        let expired = backend.expire_due_sandboxes(&now).await?;
        if !expired.is_empty()
            && let Some(path) = &self.state_path
        {
            backend
                .save_state_json(path)
                .map_err(|error| BackendError::Runtime(format!("save lifecycle state: {error}")))?;
        }
        Ok(expired)
    }
    /// Spawn the periodic lifecycle loop.
    ///
    /// The task keeps running until its join handle is aborted or the runtime
    /// shuts down. Individual expiration errors do not stop later ticks.
    #[must_use]
    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(self.interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let _ = self.tick().await;
            }
        })
    }
}
