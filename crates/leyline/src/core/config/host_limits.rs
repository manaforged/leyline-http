use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use super::host::normalize_host;
pub(crate) use super::host_gate::HostPass;
use super::host_gate::{Count, Gates, HostGate, acquire};
pub(crate) use super::origin::Origin;

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

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HostLimits {
    rule: Rule,
    overrides: Vec<(HostPattern, Rule)>,
    max_total: Option<usize>,
    pause: Pause,
    gates: Arc<Gates>,
    floors: bool,
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
            floors: true,
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
            && !(self.floors && self.gates.has_floors())
    }

    #[cfg(feature = "html")]
    pub(crate) fn set_floor(&self, url: &url::Url, gap: Option<Duration>) {
        if let Some(origin) = Origin::of(url) {
            self.gates.set_floor(origin, gap);
        }
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

    #[cfg(feature = "html")]
    pub(crate) fn without_floors(mut self) -> Self {
        self.floors = false;
        self
    }

    #[cfg(feature = "html")]
    pub(crate) fn ready(&self, origin: &Origin) -> bool {
        self.ready_at(origin).is_none() && self.has_room(origin)
    }

    #[cfg(feature = "html")]
    fn has_room(&self, origin: &Origin) -> bool {
        let free = |slots: Option<&Arc<tokio::sync::Semaphore>>| {
            slots.is_none_or(|slots| slots.available_permits() > 0)
        };
        let gate = self
            .gates
            .by_origin
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(origin)
            .cloned();
        free(self.gates.total.as_ref()) && gate.is_none_or(|gate| free(gate.slots.as_ref()))
    }

    #[cfg(feature = "html")]
    pub(crate) fn ready_at(&self, origin: &Origin) -> Option<Instant> {
        let gate = self
            .gates
            .by_origin
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(origin)
            .cloned()?;
        let interval = self.interval_for(origin, self.rule_for(&origin.host));
        gate.ready_at(interval, Instant::now())
    }

    fn interval_for(&self, origin: &Origin, rule: Rule) -> Option<Duration> {
        let floor = self.floors.then(|| self.gates.floor(origin)).flatten();
        rule.interval.max(floor)
    }

    #[cfg(feature = "html")]
    pub(crate) fn max_in_flight_for(&self, origin: &Origin) -> Option<usize> {
        self.rule_for(&origin.host).max_in_flight
    }

    fn rule_for(&self, host: &str) -> Rule {
        self.overrides
            .iter()
            .filter_map(|(pattern, rule)| Some((pattern.rank(host)?, *rule)))
            .max_by_key(|(rank, _)| *rank)
            .map_or(self.rule, |(_, rule)| rule)
    }

    #[cfg(feature = "websocket")]
    pub(crate) async fn admit(&self, url: &url::Url) -> Option<HostPass> {
        self.admit_with(None, url).await
    }

    pub(crate) async fn admit_with(
        &self,
        extra: Option<&HostLimits>,
        url: &url::Url,
    ) -> Option<HostPass> {
        let origin = Origin::of(url)?;
        let sides: Vec<Side<'_>> = std::iter::once(self)
            .chain(extra)
            .filter(|limits| !limits.is_unlimited())
            .map(|limits| limits.side(&origin))
            .collect();
        if sides.is_empty() {
            return None;
        }
        let waiting: Vec<Count> = sides
            .iter()
            .map(|side| Count::enter(&side.gate.waiting))
            .collect();
        let mut permits = Vec::with_capacity(sides.len());
        for side in &sides {
            permits.push(acquire(side.gate.slots.as_ref()).await);
        }
        let totals = loop {
            let now = Instant::now();
            let due = sides
                .iter()
                .filter_map(|side| side.gate.ready_at(side.interval(&origin), now))
                .max();
            if let Some(at) = due {
                HostGate::wait_until(at).await;
                continue;
            }
            let mut totals = Vec::with_capacity(sides.len());
            for side in &sides {
                totals.push(acquire(side.limits.gates.total.as_ref()).await);
            }
            let clocks: Vec<_> = sides
                .iter()
                .map(|side| (side.gate.as_ref(), side.interval(&origin)))
                .collect();
            if HostGate::commit(&clocks) {
                break totals;
            }
        };
        drop(waiting);
        sides.into_iter().zip(permits).zip(totals).rev().fold(
            None,
            |also, ((side, permit), total)| {
                Some(HostPass {
                    _in_flight: Count::enter(&side.gate.in_flight),
                    _permit: permit,
                    _total: total,
                    _gate: side.gate,
                    _also: also.map(Box::new),
                })
            },
        )
    }

    fn side(&self, origin: &Origin) -> Side<'_> {
        let rule = self.rule_for(&origin.host);
        Side {
            limits: self,
            rule,
            gate: self.gates.gate(origin.clone(), rule.max_in_flight),
        }
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

struct Side<'a> {
    limits: &'a HostLimits,
    rule: Rule,
    gate: Arc<HostGate>,
}

impl Side<'_> {
    fn interval(&self, origin: &Origin) -> Option<Duration> {
        self.limits.interval_for(origin, self.rule)
    }
}
