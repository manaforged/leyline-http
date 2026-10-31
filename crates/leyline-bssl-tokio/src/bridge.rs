use std::fmt;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const INBOUND: usize = 16 * 1024;

pub(crate) struct AsyncStreamBridge<S> {
    pub(crate) stream: S,
    waker: Option<Waker>,
    inbound: Box<[u8]>,
    start: usize,
    end: usize,
}

impl<S> AsyncStreamBridge<S> {
    pub(crate) fn new(stream: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        Self {
            stream,
            waker: None,
            inbound: vec![0; INBOUND].into_boxed_slice(),
            start: 0,
            end: 0,
        }
    }

    pub(crate) fn set_waker(&mut self, ctx: Option<&mut Context<'_>>) {
        self.waker = ctx.map(|ctx| ctx.waker().clone());
    }

    pub(crate) fn with_context<F, R>(&mut self, f: F) -> R
    where
        S: Unpin,
        F: FnOnce(&mut Context<'_>, Pin<&mut S>) -> R,
    {
        let mut ctx =
            Context::from_waker(self.waker.as_ref().expect("BUG: missing waker in bridge"));

        f(&mut ctx, Pin::new(&mut self.stream))
    }
}

impl<S> io::Read for AsyncStreamBridge<S>
where
    S: AsyncRead + Unpin,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.start == self.end {
            if buf.len() >= INBOUND {
                return self.read_socket(buf);
            }
            let mut inbound = std::mem::take(&mut self.inbound);
            let filled = self.read_socket(&mut inbound);
            self.inbound = inbound;
            self.end = filled?;
            self.start = 0;
        }
        let n = buf.len().min(self.end - self.start);
        buf[..n].copy_from_slice(&self.inbound[self.start..self.start + n]);
        self.start += n;
        Ok(n)
    }
}

impl<S> AsyncStreamBridge<S>
where
    S: AsyncRead + Unpin,
{
    fn read_socket(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.with_context(|ctx, stream| {
            let mut buf = ReadBuf::new(buf);

            match stream.poll_read(ctx, &mut buf)? {
                Poll::Ready(()) => Ok(buf.filled().len()),
                Poll::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            }
        })
    }
}

impl<S> io::Write for AsyncStreamBridge<S>
where
    S: AsyncWrite + Unpin,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.with_context(|ctx, stream| stream.poll_write(ctx, buf)) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.with_context(|ctx, stream| stream.poll_flush(ctx)) {
            Poll::Ready(r) => r,
            Poll::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
        }
    }
}

impl<S> fmt::Debug for AsyncStreamBridge<S>
where
    S: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.stream, fmt)
    }
}

#[cfg(test)]
mod tests;
