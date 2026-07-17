//! Idempotent retry with exponential backoff.
//!
//! `reqwest` doesn't retry, and neither does Leyline by default. Under
//! load, networks fault — a dropped TCP connection mid-response, a 502 from
//! a reloading upstream, a transient 503. The happy path for an HTTP
//! client that wants to survive real-world traffic is to retry *safe*
//! operations with exponential backoff.
//!
//! This module provides [`RetryPolicy`] — a small, explicit opt-in.
//! Default behaviour is zero retries (see [`RetryPolicy::none`]). Callers opt
//! in per-request via [`crate::RequestBuilder::retry`] or for every request in
//! a session via [`crate::SessionBuilder::retry`].
//!
//! # Idempotence
//!
//! Retrying a non-idempotent request (`POST` / `PATCH`) is dangerous:
//! the server may have committed the first attempt even though the
//! client saw a connection error, so the retry would double-apply the
//! side effect. We guard against that by default — `POST`/`PATCH`
//! requests only retry when the caller explicitly calls
//! [`crate::RequestBuilder::allow_non_idempotent_retry`].
//!
//! # Streaming bodies
//!
//! [`crate::Body::Stream`] cannot be replayed — the `Stream` may have
//! been polled to completion on the first attempt. When a retry would
//! fire against a streaming body, the retry engine returns a clear
//! error pointing the caller at [`crate::Body::Bytes`] instead. It
//! never silently drops the retry.

use std::time::Duration;

/// Trigger condition for a retry attempt.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum RetryTrigger {
    /// Connection-level IO errors (reset, EOF mid-response, refused, etc.).
    ConnectionError,
    /// A specific HTTP status code (e.g. 429, 502, 503, 504).
    Status(u16),
    /// Any 5xx status. Implies [`RetryTrigger::Status`] for 500..=599.
    ServerError,
    /// The request hit the per-request or session timeout.
    Timeout,
}

/// Retry policy. Configure it with [`crate::RequestBuilder::retry`] or
/// [`crate::SessionBuilder::retry`]. New sessions use [`RetryPolicy::none`].
///
/// ```rust,ignore
/// use std::time::Duration;
/// use leyline::{RetryPolicy, RetryTrigger};
///
/// let policy = RetryPolicy::default()
///     .with_max_retries(5)
///     .with_backoff(Duration::from_millis(200), Duration::from_secs(10))
///     .on_status(429);
///
/// session.get(url).retry(policy).send().await?;
/// ```
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts (0 = no retry).
    pub max_retries: u32,
    /// Initial backoff before the first retry.
    pub initial_backoff: Duration,
    /// Upper bound on the exponential backoff.
    pub max_backoff: Duration,
    /// Exponential factor applied between attempts.
    pub backoff_factor: f64,
    /// Whether to apply AWS-style full jitter (`backoff * uniform(0,1)`)
    /// to avoid thundering-herd alignment. Off forces strict exponential.
    pub jitter: bool,
    /// Set of triggers that should cause a retry.
    pub retry_on: Vec<RetryTrigger>,
}

impl Default for RetryPolicy {
    /// Conservative defaults: 3 retries, 100ms → 1s exponential, 2.0
    /// factor, jitter on, retry on connection errors + 502/503/504 +
    /// timeout.
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(1),
            backoff_factor: 2.0,
            jitter: true,
            retry_on: vec![
                RetryTrigger::ConnectionError,
                RetryTrigger::Status(502),
                RetryTrigger::Status(503),
                RetryTrigger::Status(504),
                RetryTrigger::Timeout,
            ],
        }
    }
}

impl RetryPolicy {
    /// A policy that performs no retries. This is the default for
    /// fresh [`crate::RequestBuilder`]s — users opt into retries
    /// explicitly.
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            initial_backoff: Duration::from_millis(0),
            max_backoff: Duration::from_millis(0),
            backoff_factor: 1.0,
            jitter: false,
            retry_on: Vec::new(),
        }
    }

    /// Override the maximum number of retries.
    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Override the initial and maximum backoff durations.
    pub fn with_backoff(mut self, initial: Duration, max: Duration) -> Self {
        self.initial_backoff = initial;
        self.max_backoff = max;
        self
    }

    /// Add an HTTP status code to the retry trigger set.
    pub fn on_status(mut self, code: u16) -> Self {
        self.retry_on.push(RetryTrigger::Status(code));
        self
    }

    /// Returns true if the policy never retries.
    pub(crate) fn is_none(&self) -> bool {
        self.max_retries == 0
    }

    /// Should we retry on this status code?
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

    /// Should we retry on a connection-level error?
    pub(crate) fn matches_connection_error(&self) -> bool {
        self.retry_on
            .iter()
            .any(|t| matches!(t, RetryTrigger::ConnectionError))
    }

    /// Should we retry on a timeout?
    pub(crate) fn matches_timeout(&self) -> bool {
        self.retry_on
            .iter()
            .any(|t| matches!(t, RetryTrigger::Timeout))
    }

    /// Compute the backoff duration for attempt `n` (0-indexed). The
    /// first retry uses `initial_backoff`, the second uses
    /// `initial_backoff * backoff_factor`, and so on, capped at
    /// `max_backoff`.
    pub(crate) fn backoff(&self, attempt: u32) -> Duration {
        let base = self.initial_backoff.as_secs_f64();
        let raw = base * self.backoff_factor.powi(attempt as i32);
        let capped = raw.min(self.max_backoff.as_secs_f64());
        let jittered = if self.jitter {
            capped * cheap_jitter()
        } else {
            capped
        };
        Duration::from_secs_f64(jittered.max(0.0))
    }
}

/// Return a uniform jitter factor in the range `[0.0, 1.0]`.
///
/// AWS's "full jitter" pattern: the actual sleep is
/// `uniform(0, cap) * backoff(attempt)`, so synchronised callers
/// decorrelate maximally instead of re-aligning on the backoff midpoint.
/// The earlier `[0.5, 1.5]` window still clustered callers within a 3×
/// spread of `initial_backoff`.
fn cheap_jitter() -> f64 {
    use rand::Rng;
    rand::thread_rng().gen_range(0.0..=1.0)
}

/// Parse a `Retry-After` header value into a delay. Honors the
/// delta-seconds form (`Retry-After: 120`). The HTTP-date form is not parsed
/// here; callers fall back to the policy's own exponential backoff for it.
pub(crate) fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Whether a method is idempotent per RFC 9110 §9.2.2 — safe to retry
/// automatically without caller opt-in.
pub(crate) fn is_idempotent(method: &str) -> bool {
    ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_retries_5xx_subset() {
        let p = RetryPolicy::default();
        assert!(p.matches_status(502));
        assert!(p.matches_status(503));
        assert!(p.matches_status(504));
        assert!(!p.matches_status(500));
        assert!(!p.matches_status(400));
    }

    #[test]
    fn server_error_trigger_matches_all_5xx() {
        let p = RetryPolicy {
            retry_on: vec![RetryTrigger::ServerError],
            ..RetryPolicy::default()
        };
        assert!(p.matches_status(500));
        assert!(p.matches_status(599));
        assert!(!p.matches_status(400));
    }

    #[test]
    fn backoff_grows_and_caps() {
        let p = RetryPolicy {
            max_retries: 10,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_millis(800),
            backoff_factor: 2.0,
            jitter: false,
            retry_on: vec![RetryTrigger::ConnectionError],
        };
        assert!(p.backoff(0).as_millis() <= 100);
        assert!(p.backoff(1).as_millis() <= 200);
        assert!(p.backoff(10).as_millis() <= 800);
    }

    #[test]
    fn none_is_no_retry() {
        let p = RetryPolicy::none();
        assert!(p.is_none());
        assert_eq!(p.max_retries, 0);
    }

    #[test]
    fn retry_after_parses_delta_seconds_only() {
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after("  5 "), Some(Duration::from_secs(5)));
        assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
        // HTTP-date form is not parsed here — falls through to backoff.
        assert_eq!(parse_retry_after("Wed, 21 Oct 2025 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after("soon"), None);
    }

    #[test]
    fn idempotent_matches_rfc_set() {
        for m in ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"] {
            assert!(is_idempotent(m), "{m}");
            assert!(is_idempotent(&m.to_lowercase()), "{m}");
        }
        for m in ["POST", "PATCH"] {
            assert!(!is_idempotent(m), "{m}");
        }
    }
}
