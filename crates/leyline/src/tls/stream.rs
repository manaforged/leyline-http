//! Backend-agnostic TLS byte stream — the plug point for a second TLS
//! backend.
//!
//! [`TlsIo`] decouples the transport / h2 / pool / WebSocket layers from
//! the concrete TLS backend. Today it has a single arm — BoringSSL via
//! `leyline-bssl-tokio` — so it is monomorphic and the `match` compiles to a
//! direct call with zero dispatch cost. A future TLS backend slots in as a
//! new arm; nothing in the layers above moves.
//!
//! The TLS *metadata* (ALPN, peer cert, version, cipher) lives on
//! [`super::TlsStream`] as backend-neutral owned fields, so only the
//! inner IO object is abstracted here.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// The active TLS backend's byte stream.
///
/// One arm today (`Boring`); future backends are added as additional
/// arms without touching any caller. `Send`, `Unpin`, and `'static` are
/// inherited from the inner stream, so this satisfies the pool's
/// `H1Io` trait bound and `WebSocketStream<_>`'s
/// `S: AsyncRead + AsyncWrite + Unpin` requirement for free.
pub(crate) enum TlsIo {
    /// BoringSSL over TCP, via `leyline-bssl-tokio`.
    Boring(leyline_bssl_tokio::SslStream<TcpStream>),
    /// Nested TLS: the inner handshake (to the origin) runs over an outer TLS
    /// stream (to an `https://` CONNECT proxy). Boxed to break the recursive
    /// type (`SslStream<TlsIo>` would otherwise be infinitely sized).
    Nested(Box<leyline_bssl_tokio::SslStream<TlsIo>>),
    // future: an in-house `leyline-tls` arm slots in the same way.
}

// `SslStream<TcpStream>` is `Unpin` (the connector pins it on the stack
// via `Pin::new`), so the single-field enum is `Unpin` too and the impls
// below project the pin with the safe `get_mut()` — no unsafe needed.

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

    // `poll_write_vectored` / `is_write_vectored` are intentionally left
    // to the `AsyncWrite` default. `leyline-bssl-tokio`'s `SslStream` does not
    // override them either, so the default (write the first non-empty
    // buffer via `poll_write`, `is_write_vectored() == false`) reproduces
    // the pre-seam behavior byte-for-byte. A future backend with genuine
    // vectored support should forward these two methods then.
}
