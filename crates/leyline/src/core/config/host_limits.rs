mod statuses;

use std::time::Duration;

use crate::error::{Error, Result};
use crate::net::normalize_host;

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct Rule {
    pub(super) max_in_flight: Option<usize>,
    pub(super) interval: Option<Duration>,
}

impl Rule {
    fn is_unlimited(self) -> bool {
        self.max_in_flight.is_none() && self.interval.is_none()
    }

    fn max_in_flight(mut self, n: usize) -> Self {
        self.max_in_flight = Some(n.max(1));
        self
    }

    fn per_second(mut self, rate: f64) -> Self {
        self.interval = if rate.is_finite() && rate > 0.0 {
            Duration::try_from_secs_f64(rate.recip()).ok()
        } else {
            None
        };
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum HostPattern {
    Exact(String),
    Subdomains(String),
}

impl HostPattern {
    fn parse(host: &str) -> Self {
        let host = normalize_host(host);
        match host.strip_prefix("*.") {
            Some(domain) => Self::Subdomains(domain.to_owned()),
            None => Self::Exact(host),
        }
    }

    fn rank(&self, host: &str) -> Option<usize> {
        match self {
            Self::Exact(exact) => (exact == host).then_some(usize::MAX),
            Self::Subdomains(domain) => host
                .strip_suffix(domain.as_str())
                .is_some_and(|rest| rest.ends_with('.') && rest.len() > 1)
                .then_some(domain.len()),
        }
    }
}

/// Per-host concurrency, rate, and pause limits for a session or crawl.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[must_use]
pub struct HostLimits {
    rule: Rule,
    overrides: Vec<(HostPattern, Rule)>,
    max_total: Option<usize>,
    #[serde(default)]
    max_pending: Option<usize>,
    #[serde(default)]
    max_total_pending: Option<usize>,
    pause: Pause,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Pause {
    #[serde(with = "statuses")]
    statuses: Vec<http::StatusCode>,
    invalid: Option<u16>,
    default: Duration,
    max: Duration,
}

impl Pause {
    fn length(
        &self,
        status: http::StatusCode,
        wait: impl FnOnce() -> Option<Duration>,
    ) -> Option<Duration> {
        self.statuses
            .contains(&status)
            .then(|| wait().unwrap_or(self.default).min(self.max))
    }
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            rule: Rule::default(),
            overrides: Vec::new(),
            max_total: None,
            max_pending: None,
            max_total_pending: None,
            pause: Pause {
                statuses: Vec::new(),
                invalid: None,
                default: Duration::from_secs(60),
                max: Duration::from_secs(24 * 60 * 60),
            },
        }
    }
}

impl HostLimits {
    /// Returns limits with no per-host cap, no total cap, and a 60-second default pause.
    pub fn new() -> Self {
        Self::default()
    }

    /// Limits the requests in flight to each host, with a floor of 1.
    pub fn max_in_flight(mut self, n: usize) -> Self {
        self.rule = self.rule.max_in_flight(n);
        self
    }

    /// Limits each host to `rate` requests per second; a rate that is not a positive finite number removes the limit.
    pub fn per_second(mut self, rate: f64) -> Self {
        self.rule = self.rule.per_second(rate);
        self
    }

    /// Limits the requests in flight to a host or `*.domain` pattern, overriding the default.
    pub fn host_max_in_flight(self, host: &str, n: usize) -> Self {
        self.host_rule(host, |rule| rule.max_in_flight(n))
    }

    /// Limits the request rate to a host or `*.domain` pattern, overriding the default.
    pub fn host_per_second(self, host: &str, rate: f64) -> Self {
        self.host_rule(host, |rule| rule.per_second(rate))
    }

    fn host_rule(mut self, host: &str, set: impl FnOnce(Rule) -> Rule) -> Self {
        let pattern = HostPattern::parse(host);
        match self.overrides.iter_mut().find(|(held, _)| *held == pattern) {
            Some((_, rule)) => *rule = set(*rule),
            None => self.overrides.push((pattern, set(Rule::default()))),
        }
        self
    }

    /// Limits the requests in flight across all hosts, with a floor of 1.
    pub fn max_total_in_flight(mut self, n: usize) -> Self {
        self.max_total = Some(n.max(1));
        self
    }

    /// Fails a request with `Kind::Overloaded` when `n` requests already wait for a `max_in_flight` slot on its host.
    pub fn max_pending(mut self, n: usize) -> Self {
        self.max_pending = Some(n);
        self
    }

    /// Fails a request with `Kind::Overloaded` when `n` requests already wait for a `max_total_in_flight` slot.
    pub fn max_total_pending(mut self, n: usize) -> Self {
        self.max_total_pending = Some(n);
        self
    }

    /// Pauses a host when a response has one of these status codes, replacing any earlier list.
    pub fn pause_on(mut self, statuses: impl IntoIterator<Item = u16>) -> Self {
        self.pause.statuses.clear();
        self.pause.invalid = None;
        for code in statuses {
            match http::StatusCode::from_u16(code) {
                Ok(status) => self.pause.statuses.push(status),
                Err(_) => {
                    self.pause.invalid.get_or_insert(code);
                }
            }
        }
        self
    }

    /// Sets the pause length used when a pausing response gives no wait, 60 seconds by default.
    pub fn pause_for(mut self, default: Duration) -> Self {
        self.pause.default = default;
        self
    }

    /// Caps every pause length, 24 hours by default.
    pub fn max_pause(mut self, max: Duration) -> Self {
        self.pause.max = max;
        self
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let invalid_count = |count: Option<usize>| {
            count.is_some_and(|count| count == 0 || count > tokio::sync::Semaphore::MAX_PERMITS)
        };
        if invalid_count(self.max_total)
            || invalid_count(self.rule.max_in_flight)
            || self
                .overrides
                .iter()
                .any(|(_, rule)| invalid_count(rule.max_in_flight))
        {
            return Err(Error::config(
                "host concurrency limit is outside the supported range",
            ));
        }
        match self.pause.invalid {
            Some(code) => Err(Error::config(format!(
                "HostLimits::pause_on: {code} is not a valid HTTP status code"
            ))),
            None => Ok(()),
        }
    }

    pub(super) fn pending_limits(&self) -> (Option<usize>, Option<usize>) {
        (self.max_pending, self.max_total_pending)
    }

    pub(super) fn max_total(&self) -> Option<usize> {
        self.max_total
    }

    pub(super) fn pause_length(
        &self,
        status: http::StatusCode,
        wait: impl FnOnce() -> Option<Duration>,
    ) -> Option<Duration> {
        self.pause.length(status, wait)
    }

    pub(super) fn is_unlimited(&self) -> bool {
        self.rule.is_unlimited()
            && self.overrides.is_empty()
            && self.max_total.is_none()
            && self.pause.statuses.is_empty()
    }

    #[cfg(feature = "html")]
    pub(crate) fn filled_from(mut self, defaults: &HostLimits) -> Self {
        for rule in
            std::iter::once(&mut self.rule).chain(self.overrides.iter_mut().map(|(_, rule)| rule))
        {
            rule.max_in_flight = rule.max_in_flight.or(defaults.rule.max_in_flight);
            rule.interval = rule.interval.or(defaults.rule.interval);
        }
        if self.pause.statuses.is_empty() {
            self.pause.statuses.clone_from(&defaults.pause.statuses);
        }
        self
    }

    pub(super) fn rule_for(&self, host: &str) -> Rule {
        self.overrides
            .iter()
            .filter_map(|(pattern, rule)| Some((pattern.rank(host)?, *rule)))
            .max_by_key(|(rank, _)| *rank)
            .map_or(self.rule, |(_, rule)| rule)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostStats {
    pub(super) origin: String,
    pub(super) in_flight: usize,
    pub(super) waiting: usize,
}

impl HostStats {
    pub(crate) fn origin(&self) -> &str {
        &self.origin
    }

    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight
    }

    pub(crate) fn waiting(&self) -> usize {
        self.waiting
    }
}
