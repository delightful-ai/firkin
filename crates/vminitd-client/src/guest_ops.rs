//! Guest filesystem operation request builders.

use crate::pb;

/// Typed builder for a vminitd `RemovePath` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovePath {
    path: String,
    recursive: bool,
    allow_missing: bool,
}

impl RemovePath {
    /// Construct a non-recursive guest path removal request.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            recursive: false,
            allow_missing: false,
        }
    }

    /// Construct a recursive guest path removal request.
    #[must_use]
    pub fn recursive(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            recursive: true,
            allow_missing: false,
        }
    }

    /// Set whether a missing path is accepted as success.
    #[must_use]
    pub const fn allow_missing(mut self, allow_missing: bool) -> Self {
        self.allow_missing = allow_missing;
        self
    }

    /// Convert into the generated protobuf request.
    #[must_use]
    pub fn into_request(self) -> pb::RemovePathRequest {
        pb::RemovePathRequest {
            path: self.path,
            recursive: self.recursive,
            allow_missing: self.allow_missing,
        }
    }
}

/// Typed builder for a vminitd `Fstrim` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fstrim {
    path: String,
    minimum_bytes: u64,
}

impl Fstrim {
    /// Construct a filesystem trim request for the filesystem mounted at path.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            minimum_bytes: 0,
        }
    }

    /// Set the minimum extent size passed to `FITRIM`.
    #[must_use]
    pub const fn minimum_bytes(mut self, minimum_bytes: u64) -> Self {
        self.minimum_bytes = minimum_bytes;
        self
    }

    /// Convert into the generated protobuf request.
    #[must_use]
    pub fn into_request(self) -> pb::FstrimRequest {
        pb::FstrimRequest {
            path: self.path,
            minimum_bytes: self.minimum_bytes,
        }
    }
}

/// Typed builder for a vminitd `ApplyOciLayer` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyOciLayer {
    archive_path: String,
    destination: String,
}

impl ApplyOciLayer {
    /// Construct an OCI layer application request.
    #[must_use]
    pub fn new(archive_path: impl Into<String>, destination: impl Into<String>) -> Self {
        Self {
            archive_path: archive_path.into(),
            destination: destination.into(),
        }
    }

    /// Convert into the generated protobuf request.
    #[must_use]
    pub fn into_request(self) -> pb::ApplyOciLayerRequest {
        pb::ApplyOciLayerRequest {
            archive_path: self.archive_path,
            destination: self.destination,
        }
    }
}

/// Typed builder for a vminitd `FilesystemUsage` request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesystemUsage {
    path: String,
}

impl FilesystemUsage {
    /// Construct a filesystem usage request for the filesystem containing path.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }

    /// Convert into the generated protobuf request.
    #[must_use]
    pub fn into_request(self) -> pb::FilesystemUsageRequest {
        pb::FilesystemUsageRequest { path: self.path }
    }
}

/// Filesystem usage values returned by vminitd.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesystemUsageStats {
    /// Filesystem block size in bytes.
    pub block_size: u64,
    /// Total block count.
    pub total_blocks: u64,
    /// Free block count.
    pub free_blocks: u64,
    /// Free blocks available to unprivileged callers.
    pub available_blocks: u64,
}

impl From<pb::FilesystemUsageResponse> for FilesystemUsageStats {
    fn from(response: pb::FilesystemUsageResponse) -> Self {
        Self {
            block_size: response.block_size,
            total_blocks: response.total_blocks,
            free_blocks: response.free_blocks,
            available_blocks: response.available_blocks,
        }
    }
}
