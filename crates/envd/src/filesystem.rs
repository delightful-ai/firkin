//! envd filesystem protocol contracts.
#![allow(missing_docs)]
#[allow(unused_imports)]
use async_trait::async_trait;
#[allow(unused_imports)]
use serde::Serialize;
#[allow(unused_imports)]
use tokio::sync::mpsc;
/// envd filesystem entry kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum EnvdFilesystemFileType {
    /// Regular file.
    File,
    /// Directory.
    Directory,
}
/// envd filesystem entry metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct EnvdFilesystemEntry {
    /// Basename.
    pub name: String,
    /// Absolute or sandbox-relative path.
    pub path: String,
    /// Entry kind.
    pub file_type: EnvdFilesystemFileType,
    /// Entry size in bytes.
    pub size: i64,
    /// Unix mode bits.
    pub mode: u32,
    /// Human-readable permissions.
    pub permissions: String,
    /// Owner name.
    pub owner: String,
    /// Group name.
    pub group: String,
    /// Optional symlink target.
    pub symlink_target: Option<String>,
}
/// envd filesystem write result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[allow(private_interfaces)]
pub struct EnvdFilesystemWriteInfo {
    /// Basename.
    pub name: String,
    /// Entry kind.
    #[serde(rename = "type")]
    pub file_type: String,
    /// Written path.
    pub path: String,
}
/// envd filesystem watch event kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub enum EnvdFilesystemEventType {
    /// Entry was created.
    Create,
    /// Entry was written.
    Write,
    /// Entry was removed.
    Remove,
    /// Entry was renamed.
    Rename,
    /// Entry permissions changed.
    Chmod,
}
/// envd filesystem watch event.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(private_interfaces)]
pub struct EnvdFilesystemEvent {
    /// Event entry name.
    pub name: String,
    /// Event kind.
    pub event_type: EnvdFilesystemEventType,
}
/// Streaming envd filesystem watch output.
#[derive(Debug)]
#[allow(private_interfaces)]
pub struct EnvdFilesystemEventStream<E> {
    #[allow(missing_docs)]
    pub receiver: mpsc::Receiver<Result<EnvdFilesystemEvent, E>>,
}
impl<E> EnvdFilesystemEventStream<E> {
    fn from_events(events: Vec<EnvdFilesystemEvent>) -> Self {
        let capacity = events.len().saturating_add(1).max(1);
        let (sender, receiver) = mpsc::channel(capacity);
        for event in events {
            sender
                .try_send(Ok(event))
                .expect("fresh filesystem event stream channel has capacity");
        }
        Self { receiver }
    }
    #[allow(missing_docs)]
    pub async fn recv(&mut self) -> Option<Result<EnvdFilesystemEvent, E>> {
        self.receiver.recv().await
    }
}
/// Runtime adapter for the envd filesystem API.
#[async_trait]
#[allow(private_interfaces)]
pub trait EnvdFilesystemAdapter: Clone + Send + Sync + 'static {
    /// Error returned by this envd adapter.
    type Error: Send + 'static;

    /// Read file bytes.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn read_file(&self, path: String) -> Result<Vec<u8>, Self::Error>;
    /// Write file bytes.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn write_file(
        &self,
        path: String,
        data: Vec<u8>,
    ) -> Result<EnvdFilesystemWriteInfo, Self::Error>;
    /// List a directory.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn list_dir(
        &self,
        path: String,
        depth: u32,
    ) -> Result<Vec<EnvdFilesystemEntry>, Self::Error>;
    /// Make a directory.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn make_dir(&self, path: String) -> Result<EnvdFilesystemEntry, Self::Error>;
    /// Move a filesystem entry.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn move_entry(
        &self,
        source: String,
        destination: String,
    ) -> Result<EnvdFilesystemEntry, Self::Error>;
    /// Remove a filesystem entry.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn remove_entry(&self, path: String) -> Result<(), Self::Error>;
    /// Stat a filesystem entry.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn stat_entry(&self, path: String) -> Result<EnvdFilesystemEntry, Self::Error>;
    /// Watch a directory and return finite events for the current substrate.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn watch_dir(
        &self,
        path: String,
        recursive: bool,
    ) -> Result<Vec<EnvdFilesystemEvent>, Self::Error>;
    /// Watch a directory and stream filesystem events.
    ///
    /// # Errors
    ///
    /// Returns runtime adapter errors.
    async fn watch_dir_stream(
        &self,
        path: String,
        recursive: bool,
    ) -> Result<EnvdFilesystemEventStream<Self::Error>, Self::Error> {
        Ok(EnvdFilesystemEventStream::from_events(
            self.watch_dir(path, recursive).await?,
        ))
    }
}
