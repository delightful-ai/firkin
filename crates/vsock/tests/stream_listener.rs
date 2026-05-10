//! Public API coverage for fd-backed vsock streams and listeners.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream as StdUnixStream;

use firkin_types::VsockPort;
use firkin_vsock::{Error, VsockListener, VsockPeer, VsockStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn stream_pair() -> (VsockStream, tokio::net::UnixStream) {
    let (left, right) = StdUnixStream::pair().expect("unix stream pair");
    let owned: OwnedFd = left.into();

    right.set_nonblocking(true).expect("right nonblocking");
    let right = tokio::net::UnixStream::from_std(right).expect("right tokio stream");

    let left = VsockStream::from_owned_fd(owned).expect("vsock stream");
    (left, right)
}

#[test]
fn peer_preserves_cid_and_port() {
    let port = VsockPort::new(8080);
    let peer = VsockPeer::new(3, port);

    assert_eq!(peer.cid(), 3);
    assert_eq!(peer.port(), port);
    assert_eq!(peer.to_string(), "3:8080");
}

#[tokio::test]
async fn stream_wraps_an_owned_fd() {
    let (mut stream, mut peer) = stream_pair();

    stream.write_all(b"ping").await.expect("write to peer");
    let mut buf = [0; 4];
    peer.read_exact(&mut buf).await.expect("read from peer");
    assert_eq!(&buf, b"ping");

    peer.write_all(b"pong").await.expect("write from peer");
    stream.read_exact(&mut buf).await.expect("read from stream");
    assert_eq!(&buf, b"pong");
}

#[tokio::test]
async fn listener_accepts_streams_from_vm_side_channel() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let listener = VsockListener::from_receiver(rx);
    let (left, right) = StdUnixStream::pair().expect("unix stream pair");
    let fd: OwnedFd = left.into();
    right.set_nonblocking(true).expect("right nonblocking");
    let mut peer_side = tokio::net::UnixStream::from_std(right).expect("right tokio stream");
    let expected_peer = VsockPeer::new(2, VsockPort::new(7001));

    tx.send(Ok((fd, expected_peer))).await.expect("send fd");

    let (mut accepted, actual_peer) = listener.accept().await.expect("accepted stream");
    assert_eq!(actual_peer, expected_peer);

    accepted.write_all(b"stdio").await.expect("write accepted");
    let mut buf = [0; 5];
    peer_side.read_exact(&mut buf).await.expect("read accepted");
    assert_eq!(&buf, b"stdio");
}

#[tokio::test]
async fn listener_reports_closed_channel() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let listener = VsockListener::from_receiver(rx);
    drop(tx);

    let error = listener.accept().await.expect_err("listener closes");
    assert_eq!(error, Error::ListenerClosed);
}
