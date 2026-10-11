use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

pub(crate) use super::host_gate::HostPass;
use super::host_gate::{Count, Gates, acquire};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}://{}:{}", self.scheme, self.host, self.port)
    }
}

impl Origin {
    pub(crate) fn of(url: &url::Url) -> Option<Self> {
        Some(Self {
            scheme: url.scheme().to_ascii_lowercase(),
            host: url.host_str()?.trim_end_matches('.').to_ascii_lowercase(),
            port: url.port_or_known_default()?,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Rule {
    max_in_flight: Option<usize>,
    interval: Option<Duration>,
}

impl Rule {
    fn is_unlimited(self) -> bool {
        self.max_in_flight.is_none() && self.interval.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HostPattern {
    Exact(String),
    Subdomains(String),
}

impl HostPattern {
    fn parse(host: &str) -> Self {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
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

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HostLimits {
    rule: Rule,
    overrides: Vec<(HostPattern, Rule)>,
    max_total: Option<usize>,
    pause: Pause,
    gates: Arc<Gates>,
}

#[derive(Debug, Clone)]
struct Pause {
    statuses: Vec<http::StatusCode>,
    invalid: Option<u16>,
    default: Duration,
    max: Duration,
}

impl Pause {
    fn length(&self, status: http::StatusCode, wait: Option<Duration>) -> Option<Duration> {
        self.statuses
            .contains(&status)
            .then(|| wait.unwrap_or(self.default).min(self.max))
    }
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            rule: Rule::default(),
            overrides: Vec::new(),
            max_total: None,
            pause: Pause {
                statuses: Vec::new(),
                invalid: None,
                default: Duration::from_secs(60),
                max: Duration::from_secs(24 * 60 * 60),
            },
            gates: Arc::default(),
        }
    }
}

impl HostLimits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_in_flight(mut self, n: usize) -> Self {
        self.rule.max_in_flight = Some(n.max(1));
        self
    }

    pub fn per_second(mut self, rate: f64) -> Self {
        self.rule.interval = if rate > 0.0 {
            Duration::try_from_secs_f64(rate.recip()).ok()
        } else {
            None
        };
        self
    }

    pub fn host(mut self, host: &str, limits: HostLimits) -> Self {
        let pattern = HostPattern::parse(host);
        self.overrides.retain(|(held, _)| *held != pattern);
        self.overrides.push((pattern, limits.rule));
        self
    }

    pub fn max_total_in_flight(mut self, n: usize) -> Self {
        self.max_total = Some(n.max(1));
        self
    }

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

    pub fn pause_for(mut self, default: Duration) -> Self {
        self.pause.default = default;
        self
    }

    pub fn max_pause(mut self, max: Duration) -> Self {
        self.pause.max = max;
        self
    }

    pub(crate) fn config_error(&self) -> Option<String> {
        self.pause
            .invalid
            .map(|code| format!("HostLimits::pause_on: {code} is not a valid HTTP status code"))
    }

    pub(crate) fn armed(mut self) -> Self {
        self.gates = Arc::new(Gates::new(self.max_total));
        self
    }

    pub(crate) fn observe(&self, url: &url::Url, status: http::StatusCode, wait: Option<Duration>) {
        let Some(length) = self.pause.length(status, wait) else {
            return;
        };
        let Some(origin) = Origin::of(url) else {
            return;
        };
        let rule = self.rule_for(&origin.host);
        let now = Instant::now();
        let end = now.checked_add(length).unwrap_or(now);
        self.gates.gate(origin, rule.max_in_flight).pause_until(end);
    }

    pub(crate) fn is_unlimited(&self) -> bool {
        self.rule.is_unlimited()
            && self.overrides.is_empty()
            && self.max_total.is_none()
            && self.pause.statuses.is_empty()
            && !self.gates.has_floors()
    }

    #[cfg(feature = "html")]
    pub(crate) fn raise_floor(&self, url: &url::Url, gap: Duration) {
        if let Some(origin) = Origin::of(url) {
            self.gates.raise_floor(origin, gap);
        }
    }

    fn rule_for(&self, host: &str) -> Rule {
        self.overrides
            .iter()
            .filter_map(|(pattern, rule)| Some((pattern.rank(host)?, *rule)))
            .max_by_key(|(rank, _)| *rank)
            .map_or(self.rule, |(_, rule)| rule)
    }

    pub(crate) async fn admit(&self, url: &url::Url) -> Option<HostPass> {
        if self.is_unlimited() {
            return None;
        }
        let origin = Origin::of(url)?;
        let rule = self.rule_for(&origin.host);
        let interval = rule.interval.max(self.gates.floor(&origin));
        let gate = self.gates.gate(origin, rule.max_in_flight);
        let waiting = Count::enter(&gate.waiting);
        let permit = acquire(gate.slots.as_ref()).await;
        let total = loop {
            gate.wait_unpaused().await;
            gate.wait_spacing().await;
            let total = acquire(self.gates.total.as_ref()).await;
            if gate.try_admit(interval) {
                break total;
            }
        };
        drop(waiting);
        Some(HostPass {
            _in_flight: Count::enter(&gate.in_flight),
            _permit: permit,
            _total: total,
            _gate: gate,
        })
    }

    pub(crate) fn stats(&self) -> Vec<HostStats> {
        self.gates
            .by_origin
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(origin, gate)| HostStats {
                origin: origin.to_string(),
                in_flight: gate.in_flight.load(Ordering::Relaxed),
                waiting: gate.waiting.load(Ordering::Relaxed),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostStats {
    origin: String,
    in_flight: usize,
    waiting: usize,
}

impl HostStats {
    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    pub fn waiting(&self) -> usize {
        self.waiting
    }
}
