use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::core::config::TimeoutConfig;
use crate::core::error::{Error, Kind, Result};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline {
    at: Option<Instant>,
    response_header: Option<Duration>,
    read: Option<Duration>,
}

#[derive(Debug)]
pub(crate) struct Elapsed;

impl Deadline {
    pub(crate) fn new(session: &TimeoutConfig, request: Option<&TimeoutConfig>) -> Self {
        let merged = request.map_or(*session, |request| request.over(session));
        Self {
            at: merged
                .total_limit()
                .and_then(|total| Instant::now().checked_add(total)),
            response_header: merged.response_header_limit(),
            read: merged.read_limit(),
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

    pub(crate) async fn sleep(&self, wait: Duration) {
        tokio::time::sleep(wait.min(self.remaining())).await;
    }

    pub(crate) async fn total<T>(&self, fut: impl Future<Output = Result<T>>) -> Result<T> {
        match self.at {
            Some(at) => tokio::time::timeout_at(at, fut)
                .await
                .map_err(|_| Error::new(Kind::Timeout))?,
            None => fut.await,
        }
    }

    pub(crate) async fn response_header<T>(
        &self,
        fut: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        within(self.response_header, fut)
            .await
            .map_err(|Elapsed| Error::new(Kind::Timeout))?
    }
}

pub(crate) async fn within<F: Future>(
    limit: Option<Duration>,
    fut: F,
) -> std::result::Result<F::Output, Elapsed> {
    match limit {
        Some(limit) => tokio::time::timeout(limit, fut).await.map_err(|_| Elapsed),
        None => Ok(fut.await),
    }
}
