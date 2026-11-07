use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures_util::future::{Either, FutureExt};
use tokio::time::Instant;

mod timeout;

use crate::error::{Error, Kind, Result};
pub use timeout::Timeout;

thread_local! {
    static EXPIRY: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Sets `key` to `value` while `run` runs, and restores the previous value even if `run` panics.
pub(crate) fn scoped<T: Copy + 'static, R>(
    key: &'static std::thread::LocalKey<std::cell::Cell<T>>,
    value: T,
    run: impl FnOnce() -> R,
) -> R {
    struct Restore<T: Copy + 'static>(&'static std::thread::LocalKey<std::cell::Cell<T>>, T);
    impl<T: Copy + 'static> Drop for Restore<T> {
        fn drop(&mut self) {
            self.0.try_with(|cell| cell.set(self.1)).unwrap_or(());
        }
    }
    let _restore = Restore(key, key.with(|cell| cell.replace(value)));
    run()
}

/// Runs `poll` with `at` as the earliest time a blocking read or write on this thread may wait until.
pub(crate) fn expiring<T>(at: Option<Instant>, poll: impl FnOnce() -> T) -> T {
    let Some(at) = at else {
        return poll();
    };
    let earliest = expiry().map_or(at, |outer| outer.min(at));
    scoped(&EXPIRY, Some(earliest), poll)
}

/// Returns the earliest time a blocking read or write on this thread may wait until.
pub(crate) fn expiry() -> Option<Instant> {
    EXPIRY.with(std::cell::Cell::get)
}

pin_project_lite::pin_project! {
    /// A future that publishes its expiry to blocking sockets while it is polled.
    pub(crate) struct Expiring<F> {
        at: Option<Instant>,
        #[pin]
        fut: F,
    }
}

impl<F: Future> Future for Expiring<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = self.project();
        expiring(*this.at, || this.fut.poll(cx))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Deadline {
    at: Option<Instant>,
    response_header: Option<Duration>,
    read: Option<Duration>,
    body: Option<Duration>,
    error_body: Option<Duration>,
}

#[derive(Debug)]
pub(crate) struct Elapsed;

impl From<tokio::time::error::Elapsed> for Elapsed {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        Self
    }
}

impl Deadline {
    pub(crate) fn new(session: &Timeout, request: Option<&Timeout>) -> Self {
        let merged = request.map_or(*session, |request| request.over(session));
        Self {
            at: merged
                .total_limit()
                .and_then(|total| Instant::now().checked_add(total)),
            response_header: merged.response_header_limit(),
            read: merged.read_limit(),
            body: merged.body_limit(),
            error_body: merged.error_body_limit(),
        }
    }

    pub(crate) fn remaining(&self) -> Duration {
        self.at.map_or(Duration::MAX, |at| {
            at.saturating_duration_since(Instant::now())
        })
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.remaining().is_zero() {
            return Err(Error::new(Kind::Timeout));
        }
        Ok(())
    }

    pub(crate) fn read(&self) -> Option<Duration> {
        self.read
    }

    pub(crate) fn body(&self) -> Option<Duration> {
        self.body
    }

    pub(crate) fn error_body(&self) -> Option<Duration> {
        self.error_body
    }

    pub(crate) fn error_body_wait(&self) -> Duration {
        let remaining = self.remaining();
        self.error_body()
            .map_or(remaining, |cap| cap.min(remaining))
    }

    #[cfg(feature = "html")]
    pub(crate) fn alarm(at: Instant) -> std::pin::Pin<Box<tokio::time::Sleep>> {
        Box::pin(tokio::time::sleep_until(at))
    }

    pub(crate) async fn sleep(&self, wait: Duration) {
        tokio::time::sleep(wait.min(self.remaining())).await;
    }

    pub(crate) fn timer(&self) -> Option<tokio::time::Sleep> {
        self.at.map(tokio::time::sleep_until)
    }

    pub(crate) fn total<T, F>(&self, fut: F) -> impl Future<Output = Result<T>> + use<T, F>
    where
        F: Future<Output = Result<T>>,
    {
        match self.at {
            Some(at) => Either::Left(Expiring {
                at: Some(at),
                fut: tokio::time::timeout_at(at, fut).map(|out| match out {
                    Ok(out) => out,
                    Err(e) => Err(Error::new(Kind::Timeout).with_source(e)),
                }),
            }),
            None => Either::Right(fut),
        }
    }

    pub(crate) async fn response_header<T, F>(&self, fut: Pin<&mut F>) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        match self
            .response_header
            .and_then(|limit| Instant::now().checked_add(limit))
        {
            Some(at) => Expiring {
                at: Some(at),
                fut: tokio::time::timeout_at(at, fut),
            }
            .await
            .unwrap_or_else(|_| Err(Error::new(Kind::Timeout))),
            None => fut.await,
        }
    }
}

pub(crate) fn within<F: Future>(
    limit: Option<Duration>,
    fut: F,
) -> impl Future<Output = std::result::Result<F::Output, Elapsed>> {
    match limit.and_then(|limit| Instant::now().checked_add(limit)) {
        Some(at) => Either::Left(Expiring {
            at: Some(at),
            fut: tokio::time::timeout_at(at, fut).map(|out| out.map_err(Elapsed::from)),
        }),
        None => Either::Right(fut.map(Ok)),
    }
}

/// A periodic timer that registers its waker once per period.
pub(crate) struct Tick {
    sleep: Pin<Box<tokio::time::Sleep>>,
    period: Duration,
    armed: bool,
}

impl Tick {
    pub(crate) fn every(period: Duration) -> Self {
        Self {
            sleep: Box::pin(tokio::time::sleep_until(Instant::now() + period)),
            period,
            armed: false,
        }
    }

    pub(crate) fn poll_tick(&mut self, cx: &mut Context<'_>) -> Poll<()> {
        if self.armed && !self.sleep.is_elapsed() {
            return Poll::Pending;
        }
        match self.sleep.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.armed = false;
                self.sleep.as_mut().reset(Instant::now() + self.period);
                Poll::Ready(())
            }
            Poll::Pending => {
                self.armed = true;
                Poll::Pending
            }
        }
    }
}
