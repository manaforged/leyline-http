//! Backend-agnostic TLS byte stream — the plug point for a second TLS backend.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// The active TLS backend's byte stream.
pub(crate) enum TlsIo {
    /// BoringSSL over TCP, via `leyline-bssl-tokio`.
    Boring(leyline_bssl_tokio::SslStream<TcpStream>),
    /// Nested TLS: the inner handshake (to the origin) runs over an outer TLS stream (to an `https://` CONNECT proxy).
    Nested(Box<leyline_bssl_tokio::SslStream<TlsIo>>),
}

impl AsyncRead for TlsIo {
    #[inline]
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            TlsIo::Boring(s) => Pin::new(s).poll_read(cx, buf),
            TlsIo::Nested(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TlsIo {
    #[inline]
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            TlsIo::Boring(s) => Pin::new(s).poll_write(cx, buf),
            TlsIo::Nested(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            TlsIo::Boring(s) => Pin::new(s).poll_flush(cx),
            TlsIo::Nested(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            TlsIo::Boring(s) => Pin::new(s).poll_shutdown(cx),
            TlsIo::Nested(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}
