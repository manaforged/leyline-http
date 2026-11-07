use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use crate::net::BlockingTcp;

pub(crate) enum TlsIo {
    Boring(leyline_bssl_tokio::SslStream<TcpStream>),
    Records(Box<leyline_bssl_tokio::RecordStream<TcpStream>>),
    Blocking(Box<leyline_bssl_tokio::RecordStream<BlockingTcp>>),
    Nested(Box<leyline_bssl_tokio::SslStream<TlsIo>>),
}

impl TlsIo {
    /// Moves a TLS 1.3 record connection onto a blocking socket; other connections stay as they are.
    pub(crate) fn blocking(self, wait: Option<std::time::Duration>) -> io::Result<Self> {
        match self {
            TlsIo::Records(records) => Ok(TlsIo::Blocking(Box::new(
                records.try_map_io(|tcp| BlockingTcp::new(tcp, wait))?,
            ))),
            other => Ok(other),
        }
    }
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
            TlsIo::Records(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
            TlsIo::Blocking(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
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
            TlsIo::Records(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            TlsIo::Blocking(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            TlsIo::Nested(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            TlsIo::Boring(s) => Pin::new(s).poll_flush(cx),
            TlsIo::Records(s) => Pin::new(s.as_mut()).poll_flush(cx),
            TlsIo::Blocking(s) => Pin::new(s.as_mut()).poll_flush(cx),
            TlsIo::Nested(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            TlsIo::Boring(s) => Pin::new(s).poll_shutdown(cx),
            TlsIo::Records(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            TlsIo::Blocking(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            TlsIo::Nested(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

pub struct TlsStream {
    pub(crate) stream: TlsIo,
    pub alpn: Option<Vec<u8>>,
    pub peer_cert_der: Option<Vec<u8>>,
    pub tls_version: Option<String>,
    pub tls_cipher: Option<String>,
}

impl TlsStream {
    pub(crate) fn is_h2(&self) -> bool {
        self.alpn.as_deref() == Some(super::alpn::H2)
    }
}
