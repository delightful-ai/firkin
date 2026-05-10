#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#[allow(unused_imports)]
use base64::Engine as _;
#[allow(unused_imports)]
use firkin_types::Size;
#[allow(unused_imports)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(crate) const DEFAULT_RUNTIME_MINIMUM_FREE_DISK: Size = Size::gib(10);
pub(crate) const DEFAULT_RUNTIME_WARM_POOL_MINIMUM_FREE_DISK: Size = Size::gib(20);
pub(crate) const FRESHNESS_SYNC_CHECKOUT_METADATA: &str = "firkin.sync.checkout";
/// VM/container mechanics consumed by the production runtime composition layer.
pub mod core {
    pub use firkin_core::*;
}
/// E2B/Cube-compatible API contracts consumed by the production runtime composition layer.
pub mod e2b {
    #[allow(unused_imports, ambiguous_glob_reexports)]
    pub use {firkin_e2b_contract::*, firkin_e2b_server::*, firkin_e2b_wire::*};
}
/// Template build and freshness-sync execution consumed by the runtime composition layer.
pub mod template {
    pub use firkin_template::*;
}
/// Shared Firkin value types consumed by the runtime composition layer.
pub mod types {
    pub use firkin_types::*;
}
pub mod adapter;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use adapter::*;
pub mod continuation;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use continuation::*;
pub mod disk;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use disk::*;
pub mod hygiene;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use hygiene::*;
pub mod interactive;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use interactive::*;
/// Runtime storage root configuration and default Firkin state/cache layout.
pub mod paths;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use paths::*;
pub mod preflight;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use preflight::*;
pub mod restore;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use restore::*;
pub mod session;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use session::*;
pub mod template_build;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use template_build::*;
pub mod warm_pool;
#[allow(unused_imports, ambiguous_glob_reexports)]
pub use warm_pool::*;
