use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};
use std::sync::{Arc, Mutex};
use std::task::Waker;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use super::host_limits::{HostLimits, HostStats, Rule};
use super::origin::Origin;
use crate::error::Error;

const MAX_SPACING: Duration = Duration::from_secs(365 * 24 * 60 * 60);

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

fn over_limit(slots: Option<&Arc<Semaphore>>, waiting: &AtomicUsize, limit: Option<usize>) -> bool {
    slots.is_some_and(|slots| slots.available_permits() == 0)
        && limit.is_some_and(|limit| waiting.load(Ordering::Relaxed) > limit)
}

async fn acquire(slots: Option<&Arc<Semaphore>>) -> Option<OwnedSemaphorePermit> {
    Arc::clone(slots?).acquire_owned().await.ok()
}

pub(crate) struct HostPass {
    _in_flight: Count,
    permit: Option<OwnedSemaphorePermit>,
    total: Option<OwnedSemaphorePermit>,
    gate: Arc<HostGate>,
    _also: Option<Box<HostPass>>,
}

impl Drop for HostPass {
    fn drop(&mut self) {
        drop(self.permit.take());
        drop(self.total.take());
        self.gate.room.freed();
    }
}

#[derive(Debug, Default)]
struct Room {
    watched: AtomicBool,
    wakers: Mutex<Vec<Waker>>,
}

impl Room {
    fn freed(&self) {
        fence(Ordering::SeqCst);
        if !self.watched.load(Ordering::SeqCst) {
            return;
        }
        let wakers = {
            let mut wakers = crate::util::lock(&self.wakers);
            self.watched.store(false, Ordering::SeqCst);
            std::mem::take(&mut *wakers)
        };
        for waker in wakers {
            waker.wake();
        }
    }

    #[cfg(feature = "html")]
    fn watch(&self, waker: &Waker) {
        let mut wakers = crate::util::lock(&self.wakers);
        if !wakers.iter().any(|held| held.will_wake(waker)) {
            wakers.push(waker.clone());
        }
        self.watched.store(true, Ordering::SeqCst);
        drop(wakers);
        fence(Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct Gates {
    by_origin: Mutex<HashMap<Origin, Arc<HostGate>>>,
    total: Option<Arc<Semaphore>>,
    total_waiting: Arc<AtomicUsize>,
    floors: Mutex<HashMap<Origin, Duration>>,
    floored: AtomicBool,
    room: Arc<Room>,
    #[cfg(feature = "html")]
    derived: std::sync::OnceLock<HostGates>,
}

impl Gates {
    fn new(max_total: Option<usize>) -> Self {
        Self {
            by_origin: Mutex::default(),
            total: max_total.map(|n| Arc::new(Semaphore::new(n))),
            total_waiting: Arc::default(),
            floors: Mutex::default(),
            floored: AtomicBool::new(false),
            room: Arc::default(),
            #[cfg(feature = "html")]
            derived: std::sync::OnceLock::new(),
        }
    }

    fn floor(&self, origin: &Origin) -> Option<Duration> {
        let floors = crate::util::lock(&self.floors);
        floors.get(origin).copied()
    }

    fn has_floors(&self) -> bool {
        self.floored.load(Ordering::Acquire)
    }

    #[cfg(feature = "html")]
    fn set_floor(&self, origin: Origin, gap: Option<Duration>) {
        let mut floors = crate::util::lock(&self.floors);
        match gap {
            Some(gap) => {
                floors.insert(origin, gap);
            }
            None => {
                floors.remove(&origin);
            }
        }
        self.floored.store(!floors.is_empty(), Ordering::Release);
    }

    fn gate(&self, origin: Origin, max_in_flight: Option<usize>) -> Arc<HostGate> {
        let mut map = crate::util::lock(&self.by_origin);
        if let Some(gate) = map.get(&origin) {
            return Arc::clone(gate);
        }
        let now = Instant::now();
        let floors = crate::util::lock(&self.floors);
        map.retain(|kept, gate| {
            Arc::strong_count(gate) > 1 || gate.busy_after(now, floors.get(kept).copied())
        });
        drop(floors);
        let gate = Arc::new(HostGate::new(max_in_flight, Arc::clone(&self.room)));
        map.insert(origin, Arc::clone(&gate));
        gate
    }
}

#[derive(Debug)]
struct HostGate {
    slots: Option<Arc<Semaphore>>,
    last: Mutex<Option<Instant>>,
    quiet: Mutex<Instant>,
    paused: Mutex<Instant>,
    in_flight: Arc<AtomicUsize>,
    waiting: Arc<AtomicUsize>,
    room: Arc<Room>,
}

impl HostGate {
    fn new(max_in_flight: Option<usize>, room: Arc<Room>) -> Self {
        Self {
            slots: max_in_flight.map(|n| Arc::new(Semaphore::new(n))),
            last: Mutex::new(None),
            quiet: Mutex::new(Instant::now()),
            paused: Mutex::new(Instant::now()),
            in_flight: Arc::default(),
            waiting: Arc::default(),
            room,
        }
    }

    fn commit(gates: &[(&HostGate, Option<Duration>)]) -> bool {
        let now = Instant::now();
        let mut locks = Vec::with_capacity(gates.len());
        for (gate, interval) in gates {
            if locks
                .iter()
                .any(|(held, _, _): &(&HostGate, _, _)| std::ptr::eq(*held, *gate))
            {
                continue;
            }
            let last = crate::util::lock(&gate.last);
            if spaced_until(*last, *interval, now) > now || gate.paused_until() > now {
                return false;
            }
            locks.push((*gate, last, *interval));
        }
        for (gate, mut last, interval) in locks {
            *last = Some(now);
            let mut quiet = crate::util::lock(&gate.quiet);
            *quiet = (*quiet).max(spaced_until(*last, interval, now));
        }
        true
    }

    fn ready_at(&self, interval: Option<Duration>, now: Instant) -> Option<Instant> {
        let last = *crate::util::lock(&self.last);
        let end = spaced_until(last, interval, now).max(self.paused_until());
        (end > now).then_some(end)
    }

    fn busy_after(&self, now: Instant, floor: Option<Duration>) -> bool {
        let last = *crate::util::lock(&self.last);
        *crate::util::lock(&self.quiet) > now
            || spaced_until(last, floor, now) > now
            || self.paused_until() > now
    }

    fn paused_until(&self) -> Instant {
        *crate::util::lock(&self.paused)
    }

    fn pause_until(&self, end: Instant) {
        let mut paused = crate::util::lock(&self.paused);
        *paused = (*paused).max(end);
    }
}

fn spaced_until(last: Option<Instant>, interval: Option<Duration>, now: Instant) -> Instant {
    match (last, interval) {
        (Some(last), Some(interval)) => last
            .checked_add(interval)
            .unwrap_or_else(|| now.checked_add(MAX_SPACING).unwrap_or(now)),
        _ => now,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HostGates {
    limits: HostLimits,
    gates: Arc<Gates>,
    floors: bool,
}

impl HostGates {
    pub(crate) fn new(limits: HostLimits) -> Self {
        Self {
            gates: Arc::new(Gates::new(limits.max_total())),
            limits,
            floors: true,
        }
    }

    #[cfg(feature = "html")]
    pub(crate) fn limits(&self) -> &HostLimits {
        &self.limits
    }

    #[cfg(feature = "html")]
    pub(crate) fn derived(&self, limits: impl FnOnce(&HostLimits) -> HostLimits) -> HostGates {
        self.gates
            .derived
            .get_or_init(|| HostGates::new(limits(&self.limits)))
            .clone()
    }

    #[cfg(feature = "html")]
    pub(crate) fn without_floors(mut self) -> Self {
        self.floors = false;
        self
    }

    pub(crate) fn observe(
        &self,
        url: &url::Url,
        status: http::StatusCode,
        wait: impl FnOnce() -> Option<Duration>,
    ) {
        let Some(length) = self.limits.pause_length(status, wait) else {
            return;
        };
        let Some(origin) = Origin::of(url) else {
            return;
        };
        let now = Instant::now();
        let end = now.checked_add(length).unwrap_or(now);
        self.gate(origin).pause_until(end);
    }

    #[cfg(feature = "html")]
    pub(crate) fn pause_until(&self, origin: Origin, until: Instant) {
        self.gate(origin).pause_until(until);
    }

    fn is_unlimited(&self) -> bool {
        self.limits.is_unlimited() && !(self.floors && self.gates.has_floors())
    }

    #[cfg(feature = "html")]
    pub(crate) fn set_floor(&self, url: &url::Url, gap: Option<Duration>) {
        if let Some(origin) = Origin::of(url) {
            self.gates.set_floor(origin, gap);
        }
    }

    #[cfg(feature = "html")]
    pub(crate) fn ready(&self, origin: &Origin) -> bool {
        self.ready_at(origin).is_none() && self.has_room(origin)
    }

    #[cfg(feature = "html")]
    pub(crate) fn has_room(&self, origin: &Origin) -> bool {
        let free = |slots: Option<&Arc<Semaphore>>| {
            slots.is_none_or(|slots| slots.available_permits() > 0)
        };
        let gate = crate::util::lock(&self.gates.by_origin)
            .get(origin)
            .cloned();
        free(self.gates.total.as_ref()) && gate.is_none_or(|gate| free(gate.slots.as_ref()))
    }

    #[cfg(feature = "html")]
    pub(crate) fn watch_room(&self, waker: &Waker) {
        self.gates.room.watch(waker);
    }

    #[cfg(feature = "html")]
    pub(crate) fn ready_at(&self, origin: &Origin) -> Option<Instant> {
        let gate = crate::util::lock(&self.gates.by_origin)
            .get(origin)
            .cloned()?;
        let interval = self.interval_for(origin, self.limits.rule_for(&origin.host));
        gate.ready_at(interval, Instant::now())
    }

    fn interval_for(&self, origin: &Origin, rule: Rule) -> Option<Duration> {
        let floor = self.floors.then(|| self.gates.floor(origin)).flatten();
        rule.interval.max(floor)
    }

    #[cfg(feature = "html")]
    pub(crate) fn max_in_flight_for(&self, origin: &Origin) -> Option<usize> {
        self.limits.rule_for(&origin.host).max_in_flight
    }

    fn gate(&self, origin: Origin) -> Arc<HostGate> {
        let rule = self.limits.rule_for(&origin.host);
        self.gates.gate(origin, rule.max_in_flight)
    }

    pub(crate) async fn admit_with(
        &self,
        extra: Option<&HostGates>,
        url: &url::Url,
    ) -> Result<Option<HostPass>, Error> {
        if std::iter::once(self)
            .chain(extra)
            .all(HostGates::is_unlimited)
        {
            return Ok(None);
        }
        let Some(origin) = Origin::of(url) else {
            return Ok(None);
        };
        let sides: Vec<Side<'_>> = std::iter::once(self)
            .chain(extra)
            .filter(|limits| !limits.is_unlimited())
            .map(|limits| limits.side(&origin))
            .collect();
        if sides.is_empty() {
            return Ok(None);
        }
        let waiting: Vec<Count> = sides
            .iter()
            .flat_map(|side| {
                [
                    Count::enter(&side.gate.waiting),
                    Count::enter(&side.limits.gates.total_waiting),
                ]
            })
            .collect();
        if sides.iter().any(Side::is_full) {
            return Err(Error::overloaded());
        }
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
                tokio::time::sleep_until(at).await;
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
        Ok(sides.into_iter().zip(permits).zip(totals).rev().fold(
            None,
            |also, ((side, permit), total)| {
                Some(HostPass {
                    _in_flight: Count::enter(&side.gate.in_flight),
                    permit,
                    total,
                    gate: side.gate,
                    _also: also.map(Box::new),
                })
            },
        ))
    }

    fn side(&self, origin: &Origin) -> Side<'_> {
        let rule = self.limits.rule_for(&origin.host);
        Side {
            limits: self,
            rule,
            gate: self.gates.gate(origin.clone(), rule.max_in_flight),
        }
    }

    pub(crate) fn stats(&self) -> Vec<HostStats> {
        crate::util::lock(&self.gates.by_origin)
            .iter()
            .map(|(origin, gate)| HostStats {
                origin: origin.to_string(),
                in_flight: gate.in_flight.load(Ordering::Relaxed),
                waiting: gate.waiting.load(Ordering::Relaxed),
            })
            .collect()
    }
}

struct Side<'a> {
    limits: &'a HostGates,
    rule: Rule,
    gate: Arc<HostGate>,
}

impl Side<'_> {
    fn is_full(&self) -> bool {
        let (host, total) = self.limits.limits.pending_limits();
        let gates = &self.limits.gates;
        over_limit(self.gate.slots.as_ref(), &self.gate.waiting, host)
            || over_limit(gates.total.as_ref(), &gates.total_waiting, total)
    }

    fn interval(&self, origin: &Origin) -> Option<Duration> {
        self.limits.interval_for(origin, self.rule)
    }
}
