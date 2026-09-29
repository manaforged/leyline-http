use std::time::Duration;

pub(crate) const GATEWAY_STATUSES: [u16; 3] = [502, 503, 504];

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
        }
    }

    pub fn transient() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(1),
            max_retry_after: Duration::MAX,
            backoff_factor: 2.0,
            jitter: true,
            retry_on: [RetryTrigger::ConnectionError, RetryTrigger::Status(429)]
                .into_iter()
                .chain(GATEWAY_STATUSES.map(RetryTrigger::Status))
                .chain([RetryTrigger::Timeout])
                .collect(),
            allow_non_idempotent: false,
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

    pub(crate) fn delay(&self, attempt: u32) -> Duration {
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

fn cheap_jitter() -> f64 {
    use rand::Rng;
    rand::rng().random_range(0.0..=1.0)
}

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

#[cfg(test)]
mod tests;
