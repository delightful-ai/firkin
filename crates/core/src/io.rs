//! io — auto-split from the parent module by `split-by-grouping`.
#![allow(missing_docs)]
#[allow(unused_imports)]
use crate::builder::ContainerStdio;
#[allow(unused_imports)]
use crate::error::{Error, Result};
#[allow(unused_imports)]
use crate::runtime::VminitdClient;
#[allow(unused_imports)]
use crate::runtime_rpc_error;
#[allow(unused_imports)]
use crate::sealed;
#[allow(unused_imports)]
pub use firkin_oci::{
    LinuxSeccompAction as SeccompAction, LinuxSeccompArch as SeccompArch,
    LinuxSeccompArg as SeccompArgRule, LinuxSeccompFlag as SeccompFlag,
    LinuxSeccompOperator as SeccompOp, LinuxSeccompProfile as Seccomp,
    LinuxSyscall as SeccompSyscallRule, Mount,
};
#[allow(unused_imports)]
use firkin_types::VirtiofsTag;
#[allow(unused_imports)]
use firkin_types::VsockPort;
#[allow(unused_imports)]
use firkin_types::{ContainerId, ProcessId};
#[allow(unused_imports)]
use firkin_vminitd_client::ProcessCreate;
use sha2::Digest as _;
#[allow(unused_imports)]
use sha2::Sha256;
#[allow(unused_imports)]
use std::io;
#[allow(unused_imports)]
use std::net::IpAddr;
#[allow(unused_imports)]
use std::path::Path;
#[allow(unused_imports)]
use std::path::PathBuf;
#[allow(unused_imports)]
use std::pin::Pin;
#[allow(unused_imports)]
use std::task::{Context, Poll};
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use tokio::io::AsyncReadExt;
#[allow(unused_imports)]
use tokio::io::AsyncWriteExt;
#[allow(unused_imports)]
use tokio::io::{AsyncRead, AsyncWrite};
pub(crate) const STDIN_PORT: VsockPort = VsockPort::new(0x1000_0000);
pub(crate) const STDOUT_PORT: VsockPort = VsockPort::new(0x1000_0001);
pub(crate) const STDERR_PORT: VsockPort = VsockPort::new(0x1000_0002);
pub(crate) const EXEC_STDIO_PORT_START: u32 = 0x1000_0100;
pub(crate) const STDIO_CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
/// Marker for stream-backed stdio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Streams;
impl sealed::Sealed for Streams {}
impl ContainerStdio for Streams {
    const TERMINAL: bool = false;
}
/// Terminal size for a pseudo-terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtyConfig {
    /// Terminal columns.
    pub cols: u16,
    /// Terminal rows.
    pub rows: u16,
}
impl PtyConfig {
    /// Construct a terminal size.
    #[must_use]
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}
impl Default for PtyConfig {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}
impl From<(u16, u16)> for PtyConfig {
    fn from((cols, rows): (u16, u16)) -> Self {
        Self { cols, rows }
    }
}
/// DNS resolver configuration for a container rootfs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DnsConfig {
    /// Nameserver IP addresses.
    pub nameservers: Vec<IpAddr>,
    /// Optional DNS domain.
    pub domain: Option<String>,
    /// DNS search domains.
    pub search: Vec<String>,
    /// Resolver options, such as `ndots:2`.
    pub options: Vec<String>,
}
/// Static `/etc/hosts` configuration for a container rootfs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostsConfig {
    /// Hosts file entries.
    pub entries: Vec<HostsEntry>,
    /// Optional file-level comment.
    pub comment: Option<String>,
}
/// One `/etc/hosts` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostsEntry {
    /// IPv4 or IPv6 address.
    pub ip: IpAddr,
    /// Hostnames attached to this address.
    pub hostnames: Vec<String>,
    /// Optional entry comment.
    pub comment: Option<String>,
}
/// Unix domain socket relay configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnixSocketConfig {
    /// Unique relay identifier inside the container.
    pub id: String,
    /// Source socket path.
    pub source: PathBuf,
    /// Destination socket path.
    pub destination: PathBuf,
    /// Relay direction.
    pub direction: SocketDirection,
    /// Mode bits for the socket created by the relay, when supported.
    pub permissions: Option<u32>,
}
impl UnixSocketConfig {
    /// Construct a socket relay configuration.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        source: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
        direction: SocketDirection,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            destination: destination.into(),
            direction,
            permissions: None,
        }
    }
    /// Construct a relay from a host Unix socket into the container.
    #[must_use]
    pub fn into_guest(
        id: impl Into<String>,
        source: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
    ) -> Self {
        Self::new(id, source, destination, SocketDirection::Into)
    }
    /// Construct a relay from a container Unix socket out to the host.
    #[must_use]
    pub fn out_of_guest(
        id: impl Into<String>,
        source: impl Into<PathBuf>,
        destination: impl Into<PathBuf>,
    ) -> Self {
        Self::new(id, source, destination, SocketDirection::OutOf)
    }
    /// Set mode bits for the socket created by the relay.
    #[must_use]
    pub const fn permissions(mut self, permissions: u32) -> Self {
        self.permissions = Some(permissions);
        self
    }
}
/// Direction for a Unix domain socket relay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SocketDirection {
    /// Host socket to guest socket.
    Into,
    /// Guest socket to host socket.
    OutOf,
}
/// Single-file mount declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMount {
    /// Host file source.
    pub source: PathBuf,
    /// Container file destination.
    pub destination: PathBuf,
    /// Whether the mounted file should be read-only in the container.
    pub read_only: bool,
}
impl FileMount {
    /// Construct a read-only file mount.
    #[must_use]
    pub fn read_only(source: impl Into<PathBuf>, destination: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            read_only: true,
        }
    }
    /// Construct a writable file mount.
    #[must_use]
    pub fn read_write(source: impl Into<PathBuf>, destination: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            read_only: false,
        }
    }
}
/// Duplex pseudo-terminal stream for a process.
pub struct Pty {
    input: firkin_vsock::VsockStream,
    output: firkin_vsock::VsockStream,
    size: PtyConfig,
    pub(crate) client: VminitdClient,
    pub(crate) container_id: ContainerId,
    pub(crate) process_id: ProcessId,
}
impl std::fmt::Debug for Pty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pty")
            .field("size", &self.size)
            .field("container_id", &self.container_id)
            .field("process_id", &self.process_id)
            .finish_non_exhaustive()
    }
}
impl Pty {
    pub(crate) fn new(
        input: firkin_vsock::VsockStream,
        output: firkin_vsock::VsockStream,
        size: PtyConfig,
        client: VminitdClient,
        container_id: ContainerId,
        process_id: ProcessId,
    ) -> Self {
        Self {
            input,
            output,
            size,
            client,
            container_id,
            process_id,
        }
    }
    /// Resize the guest pseudo-terminal.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd rejects the resize request.
    pub async fn resize(&mut self, size: impl Into<PtyConfig>) -> Result<()> {
        let size = size.into();
        self.client
            .resize_process(tonic::Request::new(ProcessCreate::resize_request(
                &self.process_id,
                Some(&self.container_id),
                u32::from(size.rows),
                u32::from(size.cols),
            )))
            .await
            .map_err(runtime_rpc_error("resize pty"))?;
        self.size = size;
        Ok(())
    }
    /// Return the last terminal size requested by the host.
    #[must_use]
    pub const fn size(&self) -> PtyConfig {
        self.size
    }
    /// Split this pseudo-terminal into independent input, output, and control handles.
    #[must_use]
    pub fn split(self) -> (PtyInput, PtyOutput, PtyControl) {
        (
            PtyInput { inner: self.input },
            PtyOutput { inner: self.output },
            PtyControl {
                size: self.size,
                client: self.client,
                container_id: self.container_id,
                process_id: self.process_id,
            },
        )
    }
}
impl AsyncRead for Pty {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.output).poll_read(cx, buf)
    }
}
impl AsyncWrite for Pty {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.input).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.input).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.input).poll_shutdown(cx)
    }
}
impl sealed::Sealed for Pty {}
impl ContainerStdio for Pty {
    const TERMINAL: bool = true;
}
/// Writable input handle for a pseudo-terminal process.
#[derive(Debug)]
pub struct PtyInput {
    inner: firkin_vsock::VsockStream,
}
impl AsyncWrite for PtyInput {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
/// Readable output handle for a pseudo-terminal process.
#[derive(Debug)]
pub struct PtyOutput {
    inner: firkin_vsock::VsockStream,
}
impl AsyncRead for PtyOutput {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
/// Resize/control handle for a pseudo-terminal process.
#[derive(Debug)]
pub struct PtyControl {
    size: PtyConfig,
    pub(crate) client: VminitdClient,
    pub(crate) container_id: ContainerId,
    pub(crate) process_id: ProcessId,
}
impl PtyControl {
    /// Resize the guest pseudo-terminal.
    ///
    /// # Errors
    ///
    /// Returns a runtime error if vminitd rejects the resize request.
    pub async fn resize(&mut self, size: impl Into<PtyConfig>) -> Result<()> {
        let size = size.into();
        self.client
            .resize_process(tonic::Request::new(ProcessCreate::resize_request(
                &self.process_id,
                Some(&self.container_id),
                u32::from(size.rows),
                u32::from(size.cols),
            )))
            .await
            .map_err(runtime_rpc_error("resize pty"))?;
        self.size = size;
        Ok(())
    }
    /// Return the last terminal size requested by the host.
    #[must_use]
    pub const fn size(&self) -> PtyConfig {
        self.size
    }
}
/// Standard stream configuration for process stdio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stdio {
    /// Do not allocate a host-side stream.
    #[default]
    Null,
    /// Allocate a host-side vsock stream and return it through `take_*`.
    Piped,
    /// Relay the guest-side stream to or from this process's stdio.
    Inherit,
}
impl Stdio {
    /// Construct a null stdio configuration.
    #[must_use]
    pub const fn null() -> Self {
        Self::Null
    }
    /// Construct a piped stdio configuration.
    #[must_use]
    pub const fn piped() -> Self {
        Self::Piped
    }
    /// Construct an inherited stdio configuration.
    #[must_use]
    pub const fn inherit() -> Self {
        Self::Inherit
    }
}
/// Writable stdin handle for an exec'd process.
#[derive(Debug)]
pub struct ChildStdin {
    inner: firkin_vsock::VsockStream,
}
impl ChildStdin {
    pub(crate) fn new(inner: firkin_vsock::VsockStream) -> Self {
        Self { inner }
    }
    /// Return the underlying vsock stream.
    #[must_use]
    pub fn into_inner(self) -> firkin_vsock::VsockStream {
        self.inner
    }
}
impl AsyncWrite for ChildStdin {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
/// Readable stdout handle for an exec'd process.
#[derive(Debug)]
pub struct ChildStdout {
    inner: firkin_vsock::VsockStream,
}
impl ChildStdout {
    pub(crate) fn new(inner: firkin_vsock::VsockStream) -> Self {
        Self { inner }
    }
    /// Return the underlying vsock stream.
    #[must_use]
    pub fn into_inner(self) -> firkin_vsock::VsockStream {
        self.inner
    }
}
impl AsyncRead for ChildStdout {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
/// Readable stderr handle for an exec'd process.
#[derive(Debug)]
pub struct ChildStderr {
    inner: firkin_vsock::VsockStream,
}
impl ChildStderr {
    pub(crate) fn new(inner: firkin_vsock::VsockStream) -> Self {
        Self { inner }
    }
    /// Return the underlying vsock stream.
    #[must_use]
    pub fn into_inner(self) -> firkin_vsock::VsockStream {
        self.inner
    }
}
impl AsyncRead for ChildStderr {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedFileMount {
    pub(crate) tag: VirtiofsTag,
    pub(crate) parent: PathBuf,
    filename: String,
    pub(crate) guest_holding_path: String,
    container_destination: PathBuf,
    pub(crate) read_only: bool,
}
impl PreparedFileMount {
    fn from_mount(mount: &FileMount) -> Result<Self> {
        let source =
            std::fs::canonicalize(&mount.source).map_err(|error| Error::RuntimeOperation {
                operation: "prepare file mount",
                reason: format!("{}: {error}", mount.source.display()),
            })?;
        let metadata = std::fs::metadata(&source).map_err(|error| Error::RuntimeOperation {
            operation: "prepare file mount",
            reason: format!("{}: {error}", source.display()),
        })?;
        if !metadata.is_file() {
            return Err(Error::RuntimeOperation {
                operation: "prepare file mount",
                reason: format!("{} is not a regular file", source.display()),
            });
        }
        let filename = source
            .file_name()
            .and_then(|filename| filename.to_str())
            .ok_or_else(|| Error::RuntimeOperation {
                operation: "prepare file mount",
                reason: format!("{} has no UTF-8 file name", source.display()),
            })?
            .to_owned();
        let parent = source
            .parent()
            .ok_or_else(|| Error::RuntimeOperation {
                operation: "prepare file mount",
                reason: format!("{} has no parent directory", source.display()),
            })?
            .to_path_buf();
        let tag = file_mount_tag(&parent)?;
        let guest_holding_path = format!("/run/file-mounts/{}", tag.as_str());
        Ok(Self {
            tag,
            parent,
            filename,
            guest_holding_path,
            container_destination: mount.destination.clone(),
            read_only: mount.read_only,
        })
    }
    pub(crate) fn oci_bind_mount(&self) -> Result<Mount> {
        let destination =
            self.container_destination
                .to_str()
                .ok_or_else(|| Error::RuntimeOperation {
                    operation: "build file bind mount",
                    reason: format!(
                        "file mount destination {} is not valid UTF-8",
                        self.container_destination.display()
                    ),
                })?;
        let mut mount = Mount::custom(
            "none",
            format!("{}/{}", self.guest_holding_path, self.filename),
            destination,
        )
        .extra_option("bind");
        mount = if self.read_only {
            mount.read_only()
        } else {
            mount.extra_option("rw")
        };
        Ok(mount)
    }
}
pub(crate) fn prepare_file_mounts(file_mounts: &[FileMount]) -> Result<Vec<PreparedFileMount>> {
    file_mounts
        .iter()
        .map(PreparedFileMount::from_mount)
        .collect()
}
fn file_mount_tag(parent: &Path) -> Result<VirtiofsTag> {
    let parent = parent.to_str().ok_or_else(|| Error::RuntimeOperation {
        operation: "prepare file mount",
        reason: format!("file mount parent {} is not valid UTF-8", parent.display()),
    })?;
    let digest = Sha256::digest(parent.as_bytes());
    let hex = format!("{digest:x}");
    VirtiofsTag::new(hex[..36].to_owned()).map_err(|error| Error::RuntimeOperation {
        operation: "prepare file mount",
        reason: error.to_string(),
    })
}
