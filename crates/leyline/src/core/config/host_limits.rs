use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

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
            host: url.host_str()?.to_ascii_lowercase(),
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

#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct HostLimits {
    rule: Rule,
    overrides: Vec<(HostPattern, Rule)>,
    max_total: Option<usize>,
    pause: Pause,
    gates: Arc<Gates>,
}

#[derive(Debug, Clone, Default)]
struct Pause {
    statuses: Vec<u16>,
    default: Option<Duration>,
}

const DEFAULT_PAUSE: Duration = Duration::from_secs(60);
const MAX_WAIT: Duration = Duration::from_secs(24 * 60 * 60);

fn later(from: Instant, wait: Duration) -> Instant {
    from + wait.min(MAX_WAIT)
}

impl Pause {
    fn length(&self, status: u16, retry_after: Option<&str>) -> Option<Duration> {
        if !self.statuses.contains(&status) {
            return None;
        }
        retry_after
            .and_then(|value| crate::core::retry::parse_retry_after(value.trim()))
            .or(self.default)
            .or(Some(DEFAULT_PAUSE))
    }
}

impl HostLimits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn max_in_flight(mut self, n: usize) -> Self {
        self.rule.max_in_flight = Some(n.max(1));
        self.reset()
    }

    pub fn per_second(mut self, rate: f64) -> Self {
        self.rule.interval = if rate > 0.0 {
            Duration::try_from_secs_f64(rate.recip()).ok()
        } else {
            None
        };
        self.reset()
    }

    pub fn host(mut self, host: &str, limits: HostLimits) -> Self {
        let pattern = HostPattern::parse(host);
        self.overrides.retain(|(held, _)| *held != pattern);
        self.overrides.push((pattern, limits.rule));
        self.reset()
    }

    pub fn max_total_in_flight(mut self, n: usize) -> Self {
        self.max_total = Some(n.max(1));
        self.reset()
    }

    pub fn pause_on(mut self, statuses: impl IntoIterator<Item = u16>) -> Self {
        self.pause.statuses = statuses.into_iter().collect();
        self.reset()
    }

    pub fn pause_for(mut self, default: Duration) -> Self {
        self.pause.default = Some(default);
        self.reset()
    }

    pub(crate) fn observe(&self, url: &url::Url, status: u16, retry_after: Option<&str>) {
        let Some(length) = self.pause.length(status, retry_after) else {
            return;
        };
        let Some(origin) = Origin::of(url) else {
            return;
        };
        let rule = self.rule_for(&origin.host);
        self.gates
            .gate(origin, rule.max_in_flight)
            .pause_until(later(Instant::now(), length));
    }

    fn reset(mut self) -> Self {
        self.gates = Arc::new(Gates::new(self.max_total));
        self
    }

    pub(crate) fn is_unlimited(&self) -> bool {
        self.rule.is_unlimited()
            && self.overrides.is_empty()
            && self.max_total.is_none()
            && self.pause.statuses.is_empty()
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
        let gate = self.gates.gate(origin, rule.max_in_flight);
        let waiting = Count::enter(&gate.waiting);
        let permit = acquire(gate.slots.as_ref()).await;
        gate.wait_unpaused().await;
        if let Some(interval) = rule.interval {
            tokio::time::sleep_until(gate.reserve(interval)).await;
        }
        let total = acquire(self.gates.total.as_ref()).await;
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

struct Count(Arc<AtomicUsize>);

impl Count {
    fn enter(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(counter))
    }
}

impl Drop for Count {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn acquire(slots: Option<&Arc<Semaphore>>) -> Option<OwnedSemaphorePermit> {
    Arc::clone(slots?).acquire_owned().await.ok()
}

pub(crate) struct HostPass {
    _in_flight: Count,
    _permit: Option<OwnedSemaphorePermit>,
    _total: Option<OwnedSemaphorePermit>,
    _gate: Arc<HostGate>,
}

#[derive(Debug, Default)]
struct Gates {
    by_origin: Mutex<HashMap<Origin, Arc<HostGate>>>,
    total: Option<Arc<Semaphore>>,
}

impl Gates {
    fn new(max_total: Option<usize>) -> Self {
        Self {
            by_origin: Mutex::default(),
            total: max_total.map(|n| Arc::new(Semaphore::new(n))),
        }
    }

    fn gate(&self, origin: Origin, max_in_flight: Option<usize>) -> Arc<HostGate> {
        let mut map = self
            .by_origin
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(gate) = map.get(&origin) {
            return Arc::clone(gate);
        }
        let now = Instant::now();
        map.retain(|_, gate| Arc::strong_count(gate) > 1 || gate.busy_after(now));
        let gate = Arc::new(HostGate::new(max_in_flight));
        map.insert(origin, Arc::clone(&gate));
        gate
    }
}

#[derive(Debug)]
struct HostGate {
    slots: Option<Arc<Semaphore>>,
    next: Mutex<Instant>,
    paused: Mutex<Instant>,
    in_flight: Arc<AtomicUsize>,
    waiting: Arc<AtomicUsize>,
}

impl HostGate {
    fn new(max_in_flight: Option<usize>) -> Self {
        Self {
            slots: max_in_flight.map(|n| Arc::new(Semaphore::new(n))),
            next: Mutex::new(Instant::now()),
            paused: Mutex::new(Instant::now()),
            in_flight: Arc::default(),
            waiting: Arc::default(),
        }
    }

    fn reserve(&self, interval: Duration) -> Instant {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        let slot = (*next).max(Instant::now());
        *next = later(slot, interval);
        slot
    }

    fn busy_after(&self, now: Instant) -> bool {
        *self.next.lock().unwrap_or_else(PoisonError::into_inner) > now || self.paused_until() > now
    }

    fn paused_until(&self) -> Instant {
        *self.paused.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn pause_until(&self, end: Instant) {
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        *paused = (*paused).max(end);
    }

    async fn wait_unpaused(&self) {
        loop {
            let end = self.paused_until();
            if end <= Instant::now() {
                return;
            }
            tokio::time::sleep_until(end).await;
        }
    }
}
