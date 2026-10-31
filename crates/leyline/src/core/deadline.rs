use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use futures_util::future::{Either, FutureExt};
use tokio::time::Instant;

use crate::core::config::Timeout;
use crate::core::error::{Cause, Error, Result};

#[derive(Debug, Clone, Copy)]
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
            return Err(Error::new(Cause::Timeout));
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

    pub(crate) fn total<T, F>(&self, fut: F) -> impl Future<Output = Result<T>> + use<T, F>
    where
        F: Future<Output = Result<T>>,
    {
        match self.at {
            Some(at) => Either::Left(tokio::time::timeout_at(at, fut).map(|out| match out {
                Ok(out) => out,
                Err(e) => Err(Error::new(Cause::Timeout).with_source(e)),
            })),
            None => Either::Right(fut),
        }
    }

    pub(crate) async fn response_header<T, F>(&self, fut: Pin<&mut F>) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        match self.response_header {
            Some(limit) => tokio::time::timeout(limit, fut)
                .await
                .unwrap_or_else(|_| Err(Error::new(Cause::Timeout))),
            None => fut.await,
        }
    }
}

pub(crate) fn within<F: Future>(
    limit: Option<Duration>,
    fut: F,
) -> impl Future<Output = std::result::Result<F::Output, Elapsed>> {
    match limit {
        Some(limit) => {
            Either::Left(tokio::time::timeout(limit, fut).map(|out| out.map_err(Elapsed::from)))
        }
        None => Either::Right(fut.map(Ok)),
    }
}
