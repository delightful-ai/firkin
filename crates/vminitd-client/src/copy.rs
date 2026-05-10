//! copy — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::error::{Result, VminitdError};
#[allow(unused_imports)]
use crate::pb;
#[allow(unused_imports)]
use firkin_types::VsockPort;
/// Direction for a vminitd host/guest copy transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyDirection {
    /// Copy from host into the guest.
    In,
    /// Copy from guest out to the host.
    Out,
}
impl CopyDirection {
    pub(crate) fn proto(self) -> pb::copy_request::Direction {
        match self {
            Self::In => pb::copy_request::Direction::CopyIn,
            Self::Out => pb::copy_request::Direction::CopyOut,
        }
    }
}
/// Metadata sent before a guest-to-host copy stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyMetadata {
    /// True when transfer bytes are a tar+gzip archive.
    pub is_archive: bool,
    /// Total transfer size when vminitd knows it.
    pub total_size: u64,
}
/// Parsed control event from vminitd's copy response stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyResponseEvent {
    /// Transfer metadata.
    Metadata(CopyMetadata),
    /// Transfer completed.
    Complete,
}
impl TryFrom<pb::CopyResponse> for CopyResponseEvent {
    type Error = VminitdError;
    fn try_from(response: pb::CopyResponse) -> Result<Self> {
        if !response.error.is_empty() {
            return Err(VminitdError::Copy {
                reason: response.error,
            });
        }
        match pb::copy_response::Status::try_from(response.status) {
            Ok(pb::copy_response::Status::Metadata) => Ok(Self::Metadata(CopyMetadata {
                is_archive: response.is_archive,
                total_size: response.total_size,
            })),
            Ok(pb::copy_response::Status::Complete) => Ok(Self::Complete),
            Err(_) => Err(VminitdError::UnknownCopyStatus {
                status: response.status,
            }),
        }
    }
}
/// Typed builder for a vminitd `Copy` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyTransfer {
    pub(crate) direction: CopyDirection,
    pub(crate) path: String,
    pub(crate) mode: u32,
    create_parents: bool,
    pub(crate) vsock_port: VsockPort,
    is_archive: bool,
}
impl CopyTransfer {
    /// Construct a host-to-guest file copy request.
    #[must_use]
    pub fn copy_in(path: impl Into<String>, vsock_port: VsockPort) -> Self {
        Self {
            direction: CopyDirection::In,
            path: path.into(),
            mode: 0o644,
            create_parents: true,
            vsock_port,
            is_archive: false,
        }
    }
    /// Construct a guest-to-host file copy request.
    #[must_use]
    pub fn copy_out(path: impl Into<String>, vsock_port: VsockPort) -> Self {
        Self {
            direction: CopyDirection::Out,
            path: path.into(),
            mode: 0,
            create_parents: false,
            vsock_port,
            is_archive: false,
        }
    }
    /// Set file mode for a single-file host-to-guest copy.
    #[must_use]
    pub const fn mode(mut self, mode: u32) -> Self {
        self.mode = mode;
        self
    }
    /// Set whether vminitd creates missing parent directories.
    #[must_use]
    pub const fn create_parents(mut self, create_parents: bool) -> Self {
        self.create_parents = create_parents;
        self
    }
    /// Set whether transfer bytes are a tar+gzip archive.
    #[must_use]
    pub const fn archive(mut self, is_archive: bool) -> Self {
        self.is_archive = is_archive;
        self
    }
    /// Convert into the generated protobuf request.
    #[must_use]
    pub fn into_request(self) -> pb::CopyRequest {
        pb::CopyRequest {
            direction: self.direction.proto() as i32,
            path: self.path,
            mode: self.mode,
            create_parents: self.create_parents,
            vsock_port: self.vsock_port.get(),
            is_archive: self.is_archive,
        }
    }
}
