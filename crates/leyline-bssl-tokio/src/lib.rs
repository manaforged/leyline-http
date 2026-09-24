#![warn(missing_docs)]

use leyline_bssl::ssl::{
    self, ConnectConfiguration, ErrorCode, MidHandshakeSslStream, ShutdownResult, SslAcceptor,
    SslRef,
};
use leyline_bssl_sys as ffi;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

mod async_callbacks;
mod bridge;

use self::bridge::AsyncStreamBridge;

pub use crate::async_callbacks::SslContextBuilderExt;
pub use leyline_bssl::ssl::{
    AsyncPrivateKeyMethod, AsyncPrivateKeyMethodError, AsyncSelectCertError, BoxGetSessionFinish,
    BoxGetSessionFuture, BoxPrivateKeyMethodFinish, BoxPrivateKeyMethodFuture, BoxSelectCertFinish,
    BoxSelectCertFuture, ExDataFuture,
};

pub async fn connect<S>(
    config: ConnectConfiguration,
    domain: &str,
    stream: S,
) -> Result<SslStream<S>, HandshakeError<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mid_handshake = config
        .setup_connect(domain, AsyncStreamBridge::new(stream))
        .map_err(|err| HandshakeError(ssl::HandshakeError::SetupFailure(err)))?;

    HandshakeFuture(Some(mid_handshake)).await
}

pub async fn accept<S>(acceptor: &SslAcceptor, stream: S) -> Result<SslStream<S>, HandshakeError<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mid_handshake = acceptor
        .setup_accept(AsyncStreamBridge::new(stream))
        .map_err(|err| HandshakeError(ssl::HandshakeError::SetupFailure(err)))?;

    HandshakeFuture(Some(mid_handshake)).await
}

fn cvt<T>(r: io::Result<T>) -> Poll<io::Result<T>> {
    match r {
        Ok(v) => Poll::Ready(Ok(v)),
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
        Err(e) => Poll::Ready(Err(e)),
    }
}

pub struct SslStreamBuilder<S> {
    inner: ssl::SslStreamBuilder<AsyncStreamBridge<S>>,
}

impl<S> SslStreamBuilder<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(ssl: ssl::Ssl, stream: S) -> Self {
        Self {
            inner: ssl::SslStreamBuilder::new(ssl, AsyncStreamBridge::new(stream)),
        }
    }

    pub async fn accept(self) -> Result<SslStream<S>, HandshakeError<S>> {
        let mid_handshake = self.inner.setup_accept();

        HandshakeFuture(Some(mid_handshake)).await
    }

    pub async fn connect(self) -> Result<SslStream<S>, HandshakeError<S>> {
        let mid_handshake = self.inner.setup_connect();

        HandshakeFuture(Some(mid_handshake)).await
    }
}

impl<S> SslStreamBuilder<S> {
    #[must_use]
    pub fn ssl(&self) -> &SslRef {
        self.inner.ssl()
    }

    pub fn ssl_mut(&mut self) -> &mut SslRef {
        self.inner.ssl_mut()
    }
}

#[derive(Debug)]
pub struct SslStream<S>(ssl::SslStream<AsyncStreamBridge<S>>);

impl<S> SslStream<S> {
    #[must_use]
    pub fn ssl(&self) -> &SslRef {
        self.0.ssl()
    }

    pub fn ssl_mut(&mut self) -> &mut SslRef {
        self.0.ssl_mut()
    }

    #[must_use]
    pub fn get_ref(&self) -> &S {
        &self.0.get_ref().stream
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.0.get_mut().stream
    }

    fn run_in_context<F, R>(&mut self, ctx: &mut Context<'_>, f: F) -> R
    where
        F: FnOnce(&mut ssl::SslStream<AsyncStreamBridge<S>>) -> R,
    {
        self.0.get_mut().set_waker(Some(ctx));

        let result = f(&mut self.0);

        self.0.get_mut().set_waker(None);

        result
    }
}

impl<S> SslStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub unsafe fn from_raw_parts(ssl: *mut ffi::SSL, stream: S) -> Self {
        unsafe {
            Self(ssl::SslStream::from_raw_parts(
                ssl,
                AsyncStreamBridge::new(stream),
            ))
        }
    }
}

impl<S> AsyncRead for SslStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        self.run_in_context(ctx, |s| {
            // SAFETY: read_uninit does not de-initialize the buffer.
            match cvt(s.read_uninit(unsafe { buf.unfilled_mut() }))? {
                Poll::Ready(nread) => {
                    unsafe {
                        buf.assume_init(nread);
                    }
                    buf.advance(nread);
                    Poll::Ready(Ok(()))
                }
                Poll::Pending => Poll::Pending,
            }
        })
    }
}

impl<S> AsyncWrite for SslStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        ctx: &mut Context,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.run_in_context(ctx, |s| cvt(s.write(buf)))
    }

    fn poll_flush(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<io::Result<()>> {
        self.run_in_context(ctx, |s| cvt(s.flush()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, ctx: &mut Context) -> Poll<io::Result<()>> {
        match self.run_in_context(ctx, |s| s.shutdown()) {
            Ok(ShutdownResult::Sent | ShutdownResult::Received) => {}
            Err(ref e) if e.code() == ErrorCode::ZERO_RETURN => {}
            Err(ref e) if e.code() == ErrorCode::WANT_READ || e.code() == ErrorCode::WANT_WRITE => {
                return Poll::Pending;
            }
            Err(e) => {
                return Poll::Ready(Err(e.into_io_error().unwrap_or_else(io::Error::other)));
            }
        }

        Pin::new(&mut self.0.get_mut().stream).poll_shutdown(ctx)
    }
}

pub struct HandshakeError<S>(ssl::HandshakeError<AsyncStreamBridge<S>>);

impl<S> HandshakeError<S> {
    #[must_use]
    pub fn ssl(&self) -> Option<&SslRef> {
        match &self.0 {
            ssl::HandshakeError::Failure(s) => Some(s.ssl()),
            _ => None,
        }
    }

    #[must_use]
    pub fn into_source_stream(self) -> Option<S> {
        match self.0 {
            ssl::HandshakeError::Failure(s) => Some(s.into_source_stream().stream),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_source_stream(&self) -> Option<&S> {
        match &self.0 {
            ssl::HandshakeError::Failure(s) => Some(&s.get_ref().stream),
            _ => None,
        }
    }

    #[must_use]
    pub fn code(&self) -> Option<ErrorCode> {
        match &self.0 {
            ssl::HandshakeError::Failure(s) => Some(s.error().code()),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_io_error(&self) -> Option<&io::Error> {
        match &self.0 {
            ssl::HandshakeError::Failure(s) => s.error().io_error(),
            _ => None,
        }
    }
}

impl<S> fmt::Debug for HandshakeError<S>
where
    S: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, fmt)
    }
}

impl<S> fmt::Display for HandshakeError<S> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, fmt)
    }
}

impl<S> Error for HandshakeError<S>
where
    S: fmt::Debug,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.0.source()
    }
}

pub struct HandshakeFuture<S>(Option<MidHandshakeSslStream<AsyncStreamBridge<S>>>);

impl<S> Future for HandshakeFuture<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<SslStream<S>, HandshakeError<S>>;

    fn poll(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut mid_handshake = self.0.take().expect("future polled after completion");

        mid_handshake.get_mut().set_waker(Some(ctx));
        mid_handshake
            .ssl_mut()
            .set_task_waker(Some(ctx.waker().clone()));

        match mid_handshake.handshake() {
            Ok(mut stream) => {
                stream.get_mut().set_waker(None);
                stream.ssl_mut().set_task_waker(None);

                Poll::Ready(Ok(SslStream(stream)))
            }
            Err(ssl::HandshakeError::WouldBlock(mut mid_handshake)) => {
                mid_handshake.get_mut().set_waker(None);
                mid_handshake.ssl_mut().set_task_waker(None);

                self.0 = Some(mid_handshake);

                Poll::Pending
            }
            Err(ssl::HandshakeError::Failure(mut mid_handshake)) => {
                mid_handshake.get_mut().set_waker(None);

                Poll::Ready(Err(HandshakeError(ssl::HandshakeError::Failure(
                    mid_handshake,
                ))))
            }
            Err(err @ ssl::HandshakeError::SetupFailure(_)) => {
                Poll::Ready(Err(HandshakeError(err)))
            }
        }
    }
}
