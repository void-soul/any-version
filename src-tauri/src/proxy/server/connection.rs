//! Cancellation must reach TCP reads before a complete HTTP header reaches middleware.
use super::lifecycle::StopSignal;
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
};

pub(super) struct OwnedListener {
    listener: TcpListener,
    stop: StopSignal,
}
impl OwnedListener {
    pub(super) fn new(listener: TcpListener, stop: StopSignal) -> Self {
        Self { listener, stop }
    }
}
impl axum::serve::Listener for OwnedListener {
    type Io = OwnedConnection;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (socket, address) = axum::serve::Listener::accept(&mut self.listener).await;
        let stop = self.stop.clone();
        (
            OwnedConnection {
                socket: Some(socket),
                cancelled: Box::pin(async move { stop.cancelled().await }),
            },
            address,
        )
    }
    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}

pub(super) struct OwnedConnection {
    socket: Option<TcpStream>,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
}
impl OwnedConnection {
    fn stopped(&mut self, context: &mut Context<'_>) -> bool {
        if self.socket.is_none() {
            return true;
        }
        if self.cancelled.as_mut().poll(context).is_ready() {
            self.socket.take(); // Close the actual accepted socket, including partial-header reads.
            return true;
        }
        false
    }
    fn closed() -> io::Error {
        io::Error::new(io::ErrorKind::BrokenPipe, "proxy instance stopped")
    }
}
impl AsyncRead for OwnedConnection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.stopped(context) {
            return Poll::Ready(Ok(()));
        }
        Pin::new(self.socket.as_mut().unwrap()).poll_read(context, buffer)
    }
}
impl AsyncWrite for OwnedConnection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.stopped(context) {
            return Poll::Ready(Err(Self::closed()));
        }
        Pin::new(self.socket.as_mut().unwrap()).poll_write(context, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.stopped(context) {
            return Poll::Ready(Err(Self::closed()));
        }
        Pin::new(self.socket.as_mut().unwrap()).poll_flush(context)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.stopped(context) {
            return Poll::Ready(Ok(()));
        }
        Pin::new(self.socket.as_mut().unwrap()).poll_shutdown(context)
    }
}
