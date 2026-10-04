use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use http::StatusCode;

use crate::core::block::BlockRules;
use crate::core::config::ProxyConfig;
use crate::core::error::{Error, ErrorCategory, Kind, Result};
use crate::core::response::Response;

mod wait;

pub use wait::WaitFormat;
pub(crate) use wait::parse_retry_after;
use wait::{RETRY_AFTER, WaitHeader};

const GATEWAY_STATUSES: [StatusCode; 3] = [
    StatusCode::BAD_GATEWAY,
    StatusCode::SERVICE_UNAVAILABLE,
    StatusCode::GATEWAY_TIMEOUT,
];

pub(crate) fn is_gateway_status(code: u16) -> bool {
    GATEWAY_STATUSES
        .iter()
        .any(|status| status.as_u16() == code)
}

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum RetryTrigger {
    ConnectionError,
    Status(u16),
    ServerError,
    Timeout,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RetryPolicy {
    pub(crate) max_retries: u32,
    pub(crate) initial_backoff: Duration,
    pub(crate) max_backoff: Duration,
    pub(crate) max_retry_after: Duration,
    pub(crate) backoff_factor: f64,
    pub(crate) jitter: bool,
    pub(crate) retry_on: Vec<RetryTrigger>,
    pub(crate) allow_non_idempotent: bool,
    pub(crate) proxies: Vec<ProxyConfig>,
    pub(crate) retry_if: Vec<RetryIf>,
    pub(crate) wait_headers: Vec<WaitHeader>,
    pub(crate) skip_blocks: Option<BlockRules>,
    pub(crate) retry_unsent: bool,
}

type ResponsePredicate = dyn Fn(&Response) -> bool + Send + Sync + std::panic::RefUnwindSafe;

#[derive(Clone)]
pub(crate) struct RetryIf(Arc<ResponsePredicate>);

impl fmt::Debug for RetryIf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RetryIf(..)")
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::none()
    }
}

impl RetryPolicy {
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            initial_backoff: Duration::from_millis(0),
            max_backoff: Duration::from_millis(0),
            max_retry_after: Duration::MAX,
            backoff_factor: 1.0,
            jitter: false,
            retry_on: Vec::new(),
            allow_non_idempotent: false,
            proxies: Vec::new(),
            retry_if: Vec::new(),
            wait_headers: Vec::new(),
            skip_blocks: None,
            retry_unsent: false,
        }
    }

    pub fn transient() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(1),
            max_retry_after: Duration::from_secs(60),
            backoff_factor: 2.0,
            jitter: true,
            retry_on: [
                RetryTrigger::ConnectionError,
                RetryTrigger::Status(StatusCode::TOO_MANY_REQUESTS.as_u16()),
            ]
            .into_iter()
            .chain(GATEWAY_STATUSES.map(|status| RetryTrigger::Status(status.as_u16())))
            .chain([RetryTrigger::Timeout])
            .collect(),
            allow_non_idempotent: false,
            proxies: Vec::new(),
            retry_if: Vec::new(),
            wait_headers: Vec::new(),
            skip_blocks: None,
            retry_unsent: false,
        }
    }

    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    pub fn initial_backoff(mut self, d: Duration) -> Self {
        self.initial_backoff = d;
        self
    }

    pub fn max_backoff(mut self, d: Duration) -> Self {
        self.max_backoff = d;
        self
    }

    pub fn backoff_factor(mut self, factor: f64) -> Self {
        self.backoff_factor = factor;
        self
    }

    pub fn jitter(mut self, on: bool) -> Self {
        self.jitter = on;
        self
    }

    pub fn max_retry_after(mut self, d: Duration) -> Self {
        self.max_retry_after = d;
        self
    }

    pub fn on_status(mut self, code: u16) -> Self {
        self.retry_on.push(RetryTrigger::Status(code));
        self
    }

    pub fn retry_on(mut self, triggers: impl IntoIterator<Item = RetryTrigger>) -> Self {
        self.retry_on = triggers.into_iter().collect();
        self
    }

    pub fn allow_non_idempotent(mut self, allow: bool) -> Self {
        self.allow_non_idempotent = allow;
        self
    }

    pub fn rotate_proxies<P: Into<ProxyConfig>>(
        mut self,
        proxies: impl IntoIterator<Item = P>,
    ) -> Self {
        self.proxies = proxies.into_iter().map(Into::into).collect();
        self
    }

    pub fn retry_if<F>(mut self, f: F) -> Self
    where
        F: Fn(&Response) -> bool + Send + Sync + std::panic::RefUnwindSafe + 'static,
    {
        self.retry_if.push(RetryIf(Arc::new(f)));
        self
    }

    pub fn wait_header(mut self, name: impl Into<String>, format: WaitFormat) -> Self {
        self.wait_headers.push(WaitHeader {
            name: name.into(),
            format,
        });
        self
    }

    pub fn skip_blocks(mut self, rules: BlockRules) -> Self {
        match &mut self.skip_blocks {
            Some(held) => held.extend(rules),
            None => self.skip_blocks = Some(rules),
        }
        self
    }

    pub fn retry_unsent(mut self, enabled: bool) -> Self {
        self.retry_unsent = enabled;
        self
    }

    pub(crate) fn matches_response(&self, response: &Response) -> bool {
        if self.is_block(response) {
            return false;
        }
        self.matches_status(response.status().as_u16())
            || self.retry_if.iter().any(|RetryIf(f)| f(response))
    }

    fn is_block(&self, response: &Response) -> bool {
        self.skip_blocks
            .as_ref()
            .is_some_and(|rules| rules.check(response).is_some())
    }

    pub(crate) fn retries_unsent(&self, result: &Result<Response>) -> bool {
        self.retry_unsent && result.as_ref().err().is_some_and(unsent)
    }

    pub(crate) fn server_wait(&self, response: &Response) -> Option<Duration> {
        self.wait_in(|name| response.header(name))
    }

    fn wait_in<'a>(&self, header: impl Fn(&str) -> Option<&'a str>) -> Option<Duration> {
        self.wait_headers
            .iter()
            .find_map(|wait| header(&wait.name).and_then(|value| wait.format.parse(value)))
            .or_else(|| header(RETRY_AFTER).and_then(parse_retry_after))
    }

    pub(crate) fn proxy_for_retry(&self, retry: u32) -> Option<&ProxyConfig> {
        let index = usize::try_from(retry.checked_sub(1)?).ok()?;
        self.proxies.get(index.checked_rem(self.proxies.len())?)
    }

    pub(crate) fn is_none(&self) -> bool {
        self.max_retries == 0
    }

    pub(crate) fn matches_status(&self, status: u16) -> bool {
        for t in &self.retry_on {
            match *t {
                RetryTrigger::Status(s) if s == status => return true,
                RetryTrigger::ServerError if (500..600).contains(&status) => return true,
                _ => {}
            }
        }
        false
    }

    pub(crate) fn matches_connection_error(&self) -> bool {
        self.retry_on
            .iter()
            .any(|t| matches!(t, RetryTrigger::ConnectionError))
    }

    pub(crate) fn matches_timeout(&self) -> bool {
        self.retry_on
            .iter()
            .any(|t| matches!(t, RetryTrigger::Timeout))
    }

    #[must_use]
    pub fn backoff(&self, attempt: u32) -> Duration {
        let base = self.initial_backoff.as_secs_f64();
        let raw = base * self.backoff_factor.powi(attempt as i32);
        let capped = raw.min(self.max_backoff.as_secs_f64());
        let jittered = if self.jitter {
            capped * cheap_jitter()
        } else {
            capped
        };
        Duration::try_from_secs_f64(jittered.max(0.0)).unwrap_or(self.max_backoff)
    }
}

fn unsent(err: &Error) -> bool {
    if err.follows_response() {
        return false;
    }
    match err.category() {
        ErrorCategory::Dns | ErrorCategory::Connect | ErrorCategory::Tls | ErrorCategory::Proxy => {
            true
        }
        ErrorCategory::Timeout => err.kind() == Kind::Connect,
        ErrorCategory::Protocol => err.is_refused_stream(),
        _ => false,
    }
}

fn cheap_jitter() -> f64 {
    use rand::Rng;
    rand::rng().random_range(0.0..=1.0)
}

#[cfg(test)]
mod tests;
