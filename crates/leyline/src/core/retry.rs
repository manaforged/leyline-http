//! Idempotent retry with exponential backoff.

use std::time::Duration;

/// Trigger condition for a retry attempt.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum RetryTrigger {
    /// Connection-level IO errors (reset, EOF mid-response, refused, etc.).
    ConnectionError,
    /// A specific HTTP status code (e.g. 429, 502, 503, 504).
    Status(u16),
    /// Any 5xx status.
    ServerError,
    /// The request hit the per-request or session timeout.
    Timeout,
}

/// Retry policy.
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
    /// Whether to apply AWS-style full jitter (`backoff * uniform(0,1)`) to avoid thundering-herd alignment.
    pub jitter: bool,
    /// Set of triggers that should cause a retry.
    pub retry_on: Vec<RetryTrigger>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::none()
    }
}

impl RetryPolicy {
    /// A policy that performs no retries.
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

    /// Retry connection errors, 502/503/504, and timeouts. `max_retries` is 3, so 4 attempts in total (100ms → 1s).
    pub fn transient() -> Self {
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

    /// Compute the backoff duration for attempt `n` (0-indexed).
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
fn cheap_jitter() -> f64 {
    use rand::Rng;
    rand::thread_rng().gen_range(0.0..=1.0)
}

/// Parse a `Retry-After` header value into a delay (delta-seconds or IMF-fixdate).
pub(crate) fn parse_retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = parse_imf_fixdate(value)?;
    Some(
        when.duration_since(std::time::SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

fn parse_imf_fixdate(s: &str) -> Option<std::time::SystemTime> {
    let rest = s.split_once(", ")?.1;
    let mut parts = rest.split_whitespace();
    let day: u32 = parts.next()?.parse().ok()?;
    let month = month_num(parts.next()?)?;
    let year: i32 = parts.next()?.parse().ok()?;
    let hms = parts.next()?;
    let tz = parts.next()?;
    if !tz.eq_ignore_ascii_case("GMT") && !tz.eq_ignore_ascii_case("UTC") {
        return None;
    }
    let mut t = hms.split(':');
    let hour: u32 = t.next()?.parse().ok()?;
    let min: u32 = t.next()?.parse().ok()?;
    let sec: u32 = t.next()?.parse().ok()?;
    unix_from_ymd_hms(year, month, day, hour, min, sec)
}

fn month_num(month: &str) -> Option<u32> {
    Some(match month {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

fn unix_from_ymd_hms(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    min: u32,
    sec: u32,
) -> Option<std::time::SystemTime> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    let mut y = year;
    if month <= 2 {
        y -= 1;
    }
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u32;
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * shifted + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = i64::from(era) * 146097 + i64::from(doe) - 719468;
    let secs = days
        .checked_mul(86400)?
        .checked_add(i64::from(hour) * 3600 + i64::from(min) * 60 + i64::from(sec))?;
    if secs < 0 {
        return None;
    }
    Some(std::time::UNIX_EPOCH + Duration::from_secs(secs as u64))
}

/// Whether a method is idempotent per RFC 9110 §9.2.2 — safe to retry automatically without caller opt-in.
pub(crate) fn is_idempotent(method: &str) -> bool {
    ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"]
        .iter()
        .any(|m| method.eq_ignore_ascii_case(m))
}

#[cfg(test)]
mod tests;
