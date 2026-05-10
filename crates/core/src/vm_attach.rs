//! vm attach — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::builder::{ContainerBuilder, Init, OnVm, OnVmArc};
#[allow(unused_imports)]
use crate::error::Result;
#[allow(unused_imports)]
use crate::ids::IntoContainerId;
#[allow(unused_imports)]
use crate::sealed;
#[allow(unused_imports)]
use firkin_vmm::{Running, VirtualMachine};
#[allow(unused_imports)]
use std::sync::Arc;
/// Extension trait that adds container builders to running VMs.
pub trait CoreContainerFactory: sealed::Sealed {
    /// Build a borrow-bound container on this VM.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidContainerId`] when `id` fails validation.
    fn container(&self, id: impl IntoContainerId) -> Result<ContainerBuilder<OnVm<'_>, Init>>;
    /// Build an Arc-shared container on this VM.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidContainerId`] when `id` fails validation.
    fn container_shared(
        self: &Arc<Self>,
        id: impl IntoContainerId,
    ) -> Result<ContainerBuilder<OnVmArc, Init>>;
}
impl CoreContainerFactory for VirtualMachine<Running> {
    fn container(&self, id: impl IntoContainerId) -> Result<ContainerBuilder<OnVm<'_>, Init>> {
        let mut builder = ContainerBuilder::new(id.into_container_id()?);
        builder.vm = Some(Box::new(self.clone()));
        Ok(builder)
    }
    fn container_shared(
        self: &Arc<Self>,
        id: impl IntoContainerId,
    ) -> Result<ContainerBuilder<OnVmArc, Init>> {
        let mut builder = ContainerBuilder::new(id.into_container_id()?);
        builder.vm = Some(Box::new((**self).clone()));
        Ok(builder)
    }
}
