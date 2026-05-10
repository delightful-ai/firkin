#![forbid(unsafe_code)]
#![warn(missing_docs)]

//! Async vsock stream primitives.

use std::fmt;
use std::future::Future;
use std::io;
use std::os::fd::OwnedFd;
use std::pin::Pin;
use std::task::{Context, Poll};

use firkin_types::VsockPort;
use futures_core::Stream;
use hyper_util::rt::TokioIo;
use thiserror::Error as ThisError;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Mutex, mpsc};
use tower_service::Service;

/// Crate-local result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by vsock stream and listener primitives.
#[derive(Debug, ThisError)]
pub enum Error {
    /// The listener-side stream channel was closed before another connection arrived.
    #[error("vsock listener closed")]
    ListenerClosed,

    /// The operating system rejected fd or stream setup.
    #[error("vsock io error: {0}")]
    Io(#[from] io::Error),
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::ListenerClosed, Self::ListenerClosed) => true,
            (Self::Io(left), Self::Io(right)) => {
                left.kind() == right.kind() && left.raw_os_error() == right.raw_os_error()
            }
            _ => false,
        }
    }
}

impl Eq for Error {}

/// Remote endpoint information for an accepted vsock connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VsockPeer {
    cid: u32,
    port: VsockPort,
}

impl VsockPeer {
    /// Construct a peer from a context id and port.
    #[must_use]
    pub const fn new(cid: u32, port: VsockPort) -> Self {
        Self { cid, port }
    }

    /// Return the peer context id.
    #[must_use]
    pub const fn cid(self) -> u32 {
        self.cid
    }

    /// Return the peer port.
    #[must_use]
    pub const fn port(self) -> VsockPort {
        self.port
    }
}

impl fmt::Display for VsockPeer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.cid, self.port.get())
    }
}

/// Async byte stream backed by an owned socket fd.
#[derive(Debug)]
pub struct VsockStream {
    inner: tokio::net::UnixStream,
}

impl VsockStream {
    /// Take ownership of an fd delivered by the VM-specific layer.
    ///
    /// Virtualization.framework provides `AF_VSOCK` stream fds. Tokio does not
    /// inspect the socket family when wrapping an already-open stream, so the
    /// portable crate stores it as a nonblocking Unix stream and only exposes
    /// byte-oriented async IO.
    ///
    /// # Errors
    ///
    /// Returns an error if the fd cannot be switched to nonblocking mode or
    /// Tokio rejects the stream wrapper.
    pub fn from_owned_fd(fd: OwnedFd) -> Result<Self> {
        let stream = std::os::unix::net::UnixStream::from(fd);
        stream.set_nonblocking(true)?;
        Ok(Self {
            inner: tokio::net::UnixStream::from_std(stream)?,
        })
    }

    /// Return the wrapped Tokio stream.
    #[must_use]
    pub fn into_inner(self) -> tokio::net::UnixStream {
        self.inner
    }
}

impl AsyncRead for VsockStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for VsockStream {
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

/// Async listener fed by VM-specific accept callbacks.
#[derive(Debug)]
pub struct VsockListener {
    receiver: Mutex<mpsc::Receiver<Result<(OwnedFd, VsockPeer)>>>,
}

impl VsockListener {
    /// Build a listener from the channel filled by the VM-specific layer.
    #[must_use]
    pub const fn from_receiver(receiver: mpsc::Receiver<Result<(OwnedFd, VsockPeer)>>) -> Self {
        Self {
            receiver: Mutex::const_new(receiver),
        }
    }

    /// Accept the next stream and its peer information.
    ///
    /// Multiple concurrent callers are serialized over the same backing
    /// channel, matching a single listening socket.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ListenerClosed`] when the VM-specific accept side drops
    /// the channel. Propagates any stream setup error sent through the channel.
    pub async fn accept(&self) -> Result<(VsockStream, VsockPeer)> {
        let mut receiver = self.receiver.lock().await;
        let (fd, peer) = receiver.recv().await.ok_or(Error::ListenerClosed)??;
        Ok((VsockStream::from_owned_fd(fd)?, peer))
    }

    /// Return a stream of accepted byte streams.
    pub fn incoming(&self) -> impl Stream<Item = Result<VsockStream>> + '_ {
        async_stream::stream! {
            loop {
                match self.accept().await {
                    Ok((stream, _peer)) => yield Ok(stream),
                    Err(Error::ListenerClosed) => break,
                    Err(error) => yield Err(error),
                }
            }
        }
    }

    /// Finish the listener and drop its receive side.
    pub fn finish(self) {}
}

/// Hyper connector that dials a fixed vsock port for each connection.
#[derive(Clone, Debug)]
pub struct VsockConnector<D> {
    port: VsockPort,
    dialer: D,
}

impl<D> VsockConnector<D> {
    /// Construct a connector from a port and async dialer.
    ///
    /// The dialer is supplied by the VM layer because creating the fd is
    /// platform-specific.
    #[must_use]
    pub const fn new(port: VsockPort, dialer: D) -> Self {
        Self { port, dialer }
    }

    /// Return the configured port.
    #[must_use]
    pub const fn port(&self) -> VsockPort {
        self.port
    }
}

impl<D, F> Service<http::Uri> for VsockConnector<D>
where
    D: Fn(VsockPort) -> F + Clone + Send + Sync + 'static,
    F: Future<Output = Result<VsockStream>> + Send + 'static,
{
    type Response = TokioIo<VsockStream>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: http::Uri) -> Self::Future {
        let port = self.port;
        let dialer = self.dialer.clone();
        Box::pin(async move {
            let stream = dialer(port).await?;
            Ok(TokioIo::new(stream))
        })
    }
}
