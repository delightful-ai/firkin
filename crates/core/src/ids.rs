//! ids — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use firkin_types::{ContainerId, InvalidContainerId};
#[allow(unused_imports)]
use firkin_types::{InvalidProcessId, ProcessId};
/// Convert supported inputs into a [`ContainerId`].
pub trait IntoContainerId {
    /// Convert into a validated container ID.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidContainerId`] when string inputs fail validation.
    fn into_container_id(self) -> std::result::Result<ContainerId, InvalidContainerId>;
}
impl IntoContainerId for ContainerId {
    fn into_container_id(self) -> std::result::Result<ContainerId, InvalidContainerId> {
        Ok(self)
    }
}
impl IntoContainerId for &str {
    fn into_container_id(self) -> std::result::Result<ContainerId, InvalidContainerId> {
        ContainerId::new(self)
    }
}
impl IntoContainerId for String {
    fn into_container_id(self) -> std::result::Result<ContainerId, InvalidContainerId> {
        ContainerId::new(self)
    }
}
/// Convert supported inputs into a [`ProcessId`].
pub trait IntoProcessId {
    /// Convert into a validated process ID.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidProcessId`] when string inputs fail validation.
    fn into_process_id(self) -> std::result::Result<ProcessId, InvalidProcessId>;
}
impl IntoProcessId for ProcessId {
    fn into_process_id(self) -> std::result::Result<ProcessId, InvalidProcessId> {
        Ok(self)
    }
}
impl IntoProcessId for &str {
    fn into_process_id(self) -> std::result::Result<ProcessId, InvalidProcessId> {
        ProcessId::new(self)
    }
}
impl IntoProcessId for String {
    fn into_process_id(self) -> std::result::Result<ProcessId, InvalidProcessId> {
        ProcessId::new(self)
    }
}
/// Absolute, normalized path inside the Linux guest.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GuestPath(String);
impl GuestPath {
    /// Construct a validated absolute guest path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidGuestPath`] when the path is empty, relative,
    /// root-only, contains NUL, or contains `..`.
    pub fn new(path: impl Into<String>) -> Result<Self> {
        let path = path.into();
        validate_guest_path(&path)?;
        Ok(Self(path))
    }
    /// Return the guest path as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Display for GuestPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
fn validate_guest_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return invalid_guest_path(path, "path cannot be empty");
    }
    if path.contains('\0') {
        return invalid_guest_path(path, "path cannot contain NUL");
    }
    if path == "/" {
        return invalid_guest_path(path, "path cannot be root");
    }
    if !path.starts_with('/') {
        return invalid_guest_path(path, "path must be absolute");
    }
    if path
        .split('/')
        .skip(1)
        .any(|component| component == ".." || component.is_empty())
    {
        return invalid_guest_path(path, "path must be normalized");
    }
    Ok(())
}
fn invalid_guest_path<T>(path: &str, reason: &'static str) -> Result<T> {
    Err(Error::InvalidGuestPath {
        path: path.to_owned(),
        reason,
    })
}
