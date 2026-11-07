use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::core::attempt::Attempt;
use crate::core::response::Response;
use crate::error::Result;

use crate::util::wait::{WaitFormat, WaitHeader, retry_after};

pub(crate) enum RetryPlan {
    Stop,
    Exhausted,
    Declined,
    Backoff(Duration),
}

const DEFAULT_MIN_BACKOFF: Duration = Duration::from_millis(100);
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(1);
const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(60);

type RetryPredicate = dyn Fn(&Attempt<'_>) -> bool + Send + Sync;

#[doc = include_str!("../../docs/retry.md")]
#[derive(Clone)]
#[must_use]
pub struct Retry {
    count: u32,
    when: Option<Arc<RetryPredicate>>,
    backoff: Backoff,
    max_wait: Duration,
    wait_headers: Vec<WaitHeader>,
    any_method: bool,
}

impl fmt::Debug for Retry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Retry")
            .field("count", &self.count)
            .field("when", &self.when.as_ref().map(|_| ".."))
            .field("backoff", &self.backoff)
            .field("max_wait", &self.max_wait)
            .field("wait_headers", &self.wait_headers)
            .field("any_method", &self.any_method)
            .finish()
    }
}

impl Default for Retry {
    fn default() -> Self {
        Self::new(0)
    }
}

impl From<u32> for Retry {
    fn from(count: u32) -> Self {
        Self::new(count)
    }
}

impl Retry {
    /// Returns a policy with `count` retries, exponential backoff from 100 ms to 1 s, and a 60 s maximum server wait.
    pub fn new(count: u32) -> Self {
        Self {
            count,
            when: None,
            backoff: Backoff::exponential(DEFAULT_MIN_BACKOFF, DEFAULT_MAX_BACKOFF),
            max_wait: DEFAULT_MAX_WAIT,
            wait_headers: Vec::new(),
            any_method: false,
        }
    }

    /// Sets the predicate that decides whether to retry, replacing the default `Attempt::is_transient` check.
    pub fn when<F>(mut self, f: F) -> Self
    where
        F: Fn(&Attempt<'_>) -> bool + Send + Sync + 'static,
    {
        self.when = Some(Arc::new(f));
        self
    }

    /// Sets a jittered delay between retries that doubles from `min` and caps at `max`, 100 ms to 1 s by default.
    pub fn backoff(mut self, min: Duration, max: Duration) -> Self {
        self.backoff = Backoff::exponential(min, max);
        self
    }

    /// Returns the backoff wait before retry `attempt`, between half of and all of the exponential delay, for loops that retry outside a request.
    #[must_use]
    pub fn delay(&self, attempt: u32) -> Duration {
        self.backoff.delay(attempt)
    }

    /// Sets the longest server-requested wait to honor, 60 seconds by default; a longer wait stops retrying.
    pub fn max_wait(mut self, wait: Duration) -> Self {
        self.max_wait = wait;
        self
    }

    pub(crate) fn allows_wait(&self, wait: Duration) -> bool {
        wait <= self.max_wait
    }

    /// Adds a header, checked before `Retry-After`, whose value is delay-seconds, a Unix timestamp in seconds, or an HTTP date.
    pub fn wait_header(self, name: impl Into<String>) -> Self {
        self.push_wait_header(name.into(), WaitFormat::Delay)
    }

    /// Adds a header, checked before `Retry-After`, whose value is a delay in milliseconds.
    pub fn wait_header_ms(self, name: impl Into<String>) -> Self {
        self.push_wait_header(name.into(), WaitFormat::Millis)
    }

    fn push_wait_header(mut self, name: String, format: WaitFormat) -> Self {
        self.wait_headers.push(WaitHeader { name, format });
        self
    }

    /// Sets whether non-idempotent methods retry too, off by default.
    pub fn any_method(mut self, any: bool) -> Self {
        self.any_method = any;
        self
    }

    #[cfg(feature = "html")]
    pub(crate) fn once(&self) -> Retry {
        Retry {
            count: 0,
            ..self.clone()
        }
    }

    pub(crate) fn count(&self) -> u32 {
        self.count
    }

    pub(crate) fn is_none(&self) -> bool {
        self.count() == 0
    }

    pub(crate) fn allows(&self, method: &http::Method) -> bool {
        self.any_method || crate::wire::is_idempotent(method)
    }

    fn wants(&self, attempt: &Attempt<'_>) -> bool {
        self.when
            .as_ref()
            .map_or_else(|| attempt.is_transient(), |when| when(attempt))
    }

    pub(crate) fn server_wait(&self, response: &Response) -> Option<Duration> {
        self.wait_headers
            .iter()
            .find_map(|wait| {
                response
                    .header(&wait.name)
                    .and_then(|value| wait.format.parse(value))
            })
            .or_else(|| retry_after(response.headers()))
    }

    pub(crate) fn plan(&self, result: &Result<Response>, attempt: u32) -> RetryPlan {
        self.plan_resend(result, attempt, true, || true)
    }

    pub(crate) fn plan_resend(
        &self,
        result: &Result<Response>,
        attempt: u32,
        method_allowed: bool,
        body_retryable: impl FnOnce() -> bool,
    ) -> RetryPlan {
        if self.is_none() {
            return RetryPlan::Stop;
        }
        let view = Attempt::sent(result);
        if attempt >= self.count {
            return match result {
                Ok(_) if self.wants(&view) => RetryPlan::Exhausted,
                _ => RetryPlan::Stop,
            };
        }
        if !self.wants(&view) || !(method_allowed || view.is_unsent()) || !body_retryable() {
            return RetryPlan::Stop;
        }
        match result.as_ref().ok().and_then(|r| self.server_wait(r)) {
            Some(wait) if wait > self.max_wait => RetryPlan::Declined,
            Some(wait) => RetryPlan::Backoff(wait),
            None => RetryPlan::Backoff(self.backoff.delay(attempt)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Backoff {
    min: Duration,
    max: Duration,
}

impl Backoff {
    pub(crate) fn exponential(min: Duration, max: Duration) -> Self {
        Self {
            min,
            max: max.max(min),
        }
    }

    #[must_use]
    pub(crate) fn delay(&self, attempt: u32) -> Duration {
        let ceiling = self.ceiling(attempt);
        ceiling / 2 + (ceiling / 2).mul_f64(cheap_jitter())
    }

    fn ceiling(&self, attempt: u32) -> Duration {
        self.min
            .checked_mul(1u32.checked_shl(attempt).unwrap_or(u32::MAX))
            .map_or(self.max, |delay| delay.min(self.max))
    }
}

fn cheap_jitter() -> f64 {
    use rand::Rng;
    rand::rng().random_range(0.0..=1.0)
}

#[cfg(test)]
mod tests;
