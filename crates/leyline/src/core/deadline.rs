use std::future::Future;
use std::time::Duration;

use tokio::time::Instant;

use crate::core::config::TimeoutConfig;
use crate::core::error::{Error, Kind, Result};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Deadline {
    at: Instant,
    response_header: Option<Duration>,
    read: Option<Duration>,
}

#[derive(Debug)]
pub(crate) struct Elapsed;

impl Deadline {
    pub(crate) fn new(
        session: &TimeoutConfig,
        request: Option<&TimeoutConfig>,
        total: Option<Duration>,
    ) -> Self {
        let pick = |field: fn(&TimeoutConfig) -> Option<Duration>| {
            request.and_then(field).or_else(|| field(session))
        };
        Self {
            at: Instant::now() + total.unwrap_or(session.total),
            response_header: pick(|t| t.response_header),
            read: pick(|t| t.read),
        }
    }

    pub(crate) fn remaining(&self) -> Duration {
        self.at.saturating_duration_since(Instant::now())
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
        tokio::time::timeout_at(self.at, fut)
            .await
            .map_err(|_| Error::new(Kind::Timeout))?
    }

    pub(crate) async fn response_header<T>(
        &self,
        fut: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        within(self.response_header, fut)
            .await
            .map_err(|Elapsed| Error::new(Kind::Timeout))?
    }

    pub(crate) async fn read_body<T>(&self, fut: impl Future<Output = Result<T>>) -> Result<T> {
        within(self.read, fut)
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
