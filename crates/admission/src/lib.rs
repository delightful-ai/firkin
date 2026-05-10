#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Production substrate control models.
pub mod active;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use active::*;
pub mod budget;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use budget::*;
pub mod capacity;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use capacity::*;
pub mod replenish;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use replenish::*;
pub mod warm_pool;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use warm_pool::*;
pub(crate) fn budget_fits(request: ResourceBudget, available: ResourceBudget) -> bool {
    request.cpus <= available.cpus
        && request.memory <= available.memory
        && request.disk <= available.disk
}
