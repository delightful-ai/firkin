use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures_core::Stream;

use crate::backend::BoxBackend;
use crate::capability::CapabilityName;
use crate::error::Result;
use crate::ids::SandboxId;
use crate::sandbox::unsupported;

pub type FilesystemEventStream =
    Pin<Box<dyn Stream<Item = Result<FilesystemEvent>> + Send + 'static>>;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SandboxPath(String);

impl SandboxPath {
    pub fn new(path: impl Into<String>) -> Result<Self> {
        let path = path.into();
        validate_path(&path)?;
        Ok(Self(path))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SandboxPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum InvalidSandboxPath {
    #[error("sandbox path is empty")]
    Empty,
    #[error("sandbox path `{0}` must be absolute")]
    Relative(String),
    #[error("sandbox path `{0}` contains a NUL byte")]
    Nul(String),
}

fn validate_path(path: &str) -> std::result::Result<(), InvalidSandboxPath> {
    if path.is_empty() {
        return Err(InvalidSandboxPath::Empty);
    }
    if !path.starts_with('/') {
        return Err(InvalidSandboxPath::Relative(path.to_owned()));
    }
    if path.bytes().any(|byte| byte == 0) {
        return Err(InvalidSandboxPath::Nul(path.to_owned()));
    }
    Ok(())
}

#[derive(Clone)]
pub struct FilesystemClient {
    backend: BoxBackend,
    sandbox_id: SandboxId,
}

impl FilesystemClient {
    pub(crate) fn new(backend: BoxBackend, sandbox_id: SandboxId) -> Self {
        Self {
            backend,
            sandbox_id,
        }
    }

    pub async fn read(&self, path: SandboxPath) -> Result<Bytes> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported("read file", CapabilityName::FilesystemRead));
        };
        control.read_file(&self.sandbox_id, path, ReadOptions).await
    }

    pub async fn write(&self, path: SandboxPath, data: impl Into<Bytes>) -> Result<FileEntry> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported("write file", CapabilityName::FilesystemWrite));
        };
        control
            .write_file(&self.sandbox_id, path, data.into(), WriteOptions::default())
            .await
    }

    pub async fn append(&self, path: SandboxPath, data: impl Into<Bytes>) -> Result<FileEntry> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported("append file", CapabilityName::FilesystemWrite));
        };
        control
            .write_file(
                &self.sandbox_id,
                path,
                data.into(),
                WriteOptions { append: true },
            )
            .await
    }

    pub async fn copy_in(
        &self,
        host: impl AsRef<Path>,
        sandbox: SandboxPath,
        options: CopyOptions,
    ) -> Result<FileEntry> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported(
                "copy into sandbox",
                CapabilityName::FilesystemCopyIn,
            ));
        };
        control
            .copy_in(&self.sandbox_id, host.as_ref(), sandbox, options)
            .await
    }

    pub async fn copy_out(
        &self,
        sandbox: SandboxPath,
        host: impl AsRef<Path>,
        options: CopyOptions,
    ) -> Result<()> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported(
                "copy out of sandbox",
                CapabilityName::FilesystemCopyOut,
            ));
        };
        control
            .copy_out(&self.sandbox_id, sandbox, host.as_ref(), options)
            .await
    }

    pub async fn list(&self, path: SandboxPath, options: ListOptions) -> Result<Vec<FileEntry>> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported("list files", CapabilityName::FilesystemList));
        };
        control.list_files(&self.sandbox_id, path, options).await
    }

    pub async fn mkdir(&self, path: SandboxPath) -> Result<FileEntry> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported(
                "make directory",
                CapabilityName::FilesystemWrite,
            ));
        };
        control.mkdir(&self.sandbox_id, path, false).await
    }

    pub async fn mkdir_all(&self, path: SandboxPath) -> Result<FileEntry> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported(
                "make directories",
                CapabilityName::FilesystemWrite,
            ));
        };
        control.mkdir(&self.sandbox_id, path, true).await
    }

    pub async fn rename(&self, from: SandboxPath, to: SandboxPath) -> Result<FileEntry> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported("rename file", CapabilityName::FilesystemWrite));
        };
        control.rename(&self.sandbox_id, from, to).await
    }

    pub async fn remove(&self, path: SandboxPath) -> Result<()> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported("remove file", CapabilityName::FilesystemWrite));
        };
        control.remove_file(&self.sandbox_id, path).await
    }

    pub async fn stat(&self, path: SandboxPath) -> Result<FileStat> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported("stat file", CapabilityName::FilesystemRead));
        };
        control.stat_file(&self.sandbox_id, path).await
    }

    pub async fn watch(
        &self,
        path: SandboxPath,
        options: WatchOptions,
    ) -> Result<FilesystemEventStream> {
        let Some(control) = self.backend.filesystems() else {
            return Err(unsupported("watch files", CapabilityName::FilesystemWatch));
        };
        control.watch_files(&self.sandbox_id, path, options).await
    }
}

impl From<(Arc<dyn crate::backend::SandboxBackend>, SandboxId)> for FilesystemClient {
    fn from((backend, sandbox_id): (Arc<dyn crate::backend::SandboxBackend>, SandboxId)) -> Self {
        Self::new(backend, sandbox_id)
    }
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: SandboxPath,
    pub file_type: FileType,
    pub size_bytes: u64,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Other,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStat {
    pub entry: FileEntry,
    pub permissions: FilePermissions,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilePermissions {
    pub mode: u32,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadOptions;

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteOptions {
    pub append: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListOptions {
    pub recursive: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyOptions {
    pub overwrite: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WatchOptions {
    pub recursive: bool,
}

#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilesystemEvent {
    Created(FileEntry),
    Modified(FileEntry),
    Removed(SandboxPath),
}

#[cfg(test)]
mod tests {
    use super::SandboxPath;

    #[test]
    fn sandbox_path_requires_absolute_guest_path() {
        assert!(SandboxPath::new("/work/file.txt").is_ok());
        assert!(SandboxPath::new("work/file.txt").is_err());
    }
}
