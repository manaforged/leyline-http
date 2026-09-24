use std::time::Duration;

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
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub max_retry_after: Duration,
    pub backoff_factor: f64,
    pub jitter: bool,
    pub retry_on: Vec<RetryTrigger>,
    pub allow_non_idempotent: bool,
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
            retry_on: vec![
                RetryTrigger::ConnectionError,
                RetryTrigger::Status(429),
                RetryTrigger::Status(502),
                RetryTrigger::Status(503),
                RetryTrigger::Status(504),
                RetryTrigger::Timeout,
            ],
            allow_non_idempotent: false,
        }
    }

    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    pub fn with_backoff(mut self, initial: Duration, max: Duration) -> Self {
        self.initial_backoff = initial;
        self.max_backoff = max;
        self
    }

    pub fn with_max_retry_after(mut self, max: Duration) -> Self {
        self.max_retry_after = max;
        self
    }

    pub fn on_status(mut self, code: u16) -> Self {
        self.retry_on.push(RetryTrigger::Status(code));
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
