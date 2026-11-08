use std::cell::Cell;
use std::io::{self, Write};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

use crate::deadline::expiry;

const VALIDATE_AFTER: Duration = Duration::from_secs(2);
const TIMEOUT_SLACK: Duration = Duration::from_millis(1);
const TIMEOUT_SLACK_PER: u32 = 1000;

thread_local! {
    static PROBING: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Runs `poll` as a liveness probe, so a blocking connection answers without waiting; `strict` checks even a recently used connection.
pub(crate) fn probing<T>(strict: bool, poll: impl FnOnce() -> T) -> T {
    crate::deadline::scoped(&PROBING, Some(strict), poll)
}

/// A TCP connection whose reads and writes block the calling thread until the active deadline.
pub(crate) struct BlockingTcp {
    stream: std::net::TcpStream,
    wait: Option<Duration>,
    read: Option<Duration>,
    write: Option<Duration>,
    used: Instant,
}

impl BlockingTcp {
    pub(crate) fn new(stream: tokio::net::TcpStream, wait: Option<Duration>) -> io::Result<Self> {
        let stream = stream.into_std()?;
        stream.set_nonblocking(false)?;
        Ok(Self {
            stream,
            wait,
            read: None,
            write: None,
            used: Instant::now(),
        })
    }

    fn until(&mut self) -> (Instant, Option<Instant>) {
        let now = Instant::now();
        self.used = now;
        let waited = self.wait.and_then(|wait| now.checked_add(wait));
        let until = match (expiry(), waited) {
            (Some(at), Some(waited)) => Some(at.min(waited)),
            (at, waited) => at.or(waited),
        };
        (now, until)
    }

    fn read_now(&mut self, buf: &mut ReadBuf<'_>) -> io::Result<()> {
        let (now, until) = self.until();
        let stream = &self.stream;
        waiting(
            &mut self.read,
            now,
            until,
            |limit| stream.set_read_timeout(limit),
            || recv(stream, buf),
        )
    }

    fn probe(&mut self, strict: bool, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        if !strict && self.used.elapsed() < VALIDATE_AFTER {
            return Poll::Pending;
        }
        let read = self
            .stream
            .set_nonblocking(true)
            .and_then(|()| recv(&self.stream, buf));
        let restored = self.stream.set_nonblocking(false);
        match read.and(restored) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            other => Poll::Ready(other),
        }
    }
}

fn recv(stream: &std::net::TcpStream, buf: &mut ReadBuf<'_>) -> io::Result<()> {
    let socket = socket2::SockRef::from(stream);
    // SAFETY: `recv` writes only initialized bytes and reports how many; the region is not read before that.
    let unfilled = unsafe { buf.unfilled_mut() };
    let n = socket.recv(unfilled)?;
    // SAFETY: the kernel initialized the first `n` bytes of the unfilled region.
    unsafe { buf.assume_init(n) };
    buf.advance(n);
    Ok(())
}

fn waiting<T>(
    applied: &mut Option<Duration>,
    mut now: Instant,
    until: Option<Instant>,
    set: impl Fn(Option<Duration>) -> io::Result<()>,
    mut io: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    loop {
        let limit = until.map(|until| until.saturating_duration_since(now));
        if limit.is_some_and(|limit| limit.is_zero()) {
            return Err(io::ErrorKind::TimedOut.into());
        }
        settle(applied, limit, &set)?;
        match io() {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => now = Instant::now(),
            done => return done.map_err(timed_out),
        }
    }
}

fn timed_out(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::WouldBlock {
        io::Error::new(io::ErrorKind::TimedOut, "blocking socket wait timed out")
    } else {
        e
    }
}

fn settle(
    current: &mut Option<Duration>,
    wanted: Option<Duration>,
    set: &impl Fn(Option<Duration>) -> io::Result<()>,
) -> io::Result<()> {
    let close = match (*current, wanted) {
        (Some(have), Some(want)) => have.abs_diff(want) <= want / TIMEOUT_SLACK_PER + TIMEOUT_SLACK,
        (None, None) => true,
        _ => false,
    };
    if !close {
        set(wanted)?;
        *current = wanted;
    }
    Ok(())
}

impl AsyncRead for BlockingTcp {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if let Some(strict) = PROBING.with(Cell::get) {
            return this.probe(strict, buf);
        }
        Poll::Ready(this.read_now(buf))
    }
}

impl AsyncWrite for BlockingTcp {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let (now, until) = this.until();
        let stream = &this.stream;
        Poll::Ready(waiting(
            &mut this.write,
            now,
            until,
            |limit| stream.set_write_timeout(limit),
            || (&*stream).write(buf),
        ))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.stream.shutdown(std::net::Shutdown::Write))
    }
}

#[cfg(test)]
#[path = "blocking_tests.rs"]
mod tests;
