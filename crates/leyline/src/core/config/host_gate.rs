use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use super::host_limits::Origin;

const MAX_SPACING: Duration = Duration::from_secs(365 * 24 * 60 * 60);

pub(super) struct Count(Arc<AtomicUsize>);

impl Count {
    pub(super) fn enter(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(counter))
    }
}

impl Drop for Count {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) async fn acquire(slots: Option<&Arc<Semaphore>>) -> Option<OwnedSemaphorePermit> {
    Arc::clone(slots?).acquire_owned().await.ok()
}

pub(crate) struct HostPass {
    pub(super) _in_flight: Count,
    pub(super) _permit: Option<OwnedSemaphorePermit>,
    pub(super) _total: Option<OwnedSemaphorePermit>,
    pub(super) _gate: Arc<HostGate>,
    pub(super) _also: Option<Box<HostPass>>,
}

#[derive(Debug, Default)]
pub(super) struct Gates {
    pub(super) by_origin: Mutex<HashMap<Origin, Arc<HostGate>>>,
    pub(super) total: Option<Arc<Semaphore>>,
    floors: Mutex<HashMap<Origin, Duration>>,
    floored: AtomicBool,
}

impl Gates {
    pub(super) fn new(max_total: Option<usize>) -> Self {
        Self {
            by_origin: Mutex::default(),
            total: max_total.map(|n| Arc::new(Semaphore::new(n))),
            floors: Mutex::default(),
            floored: AtomicBool::new(false),
        }
    }

    pub(super) fn floor(&self, origin: &Origin) -> Option<Duration> {
        let floors = self.floors.lock().unwrap_or_else(PoisonError::into_inner);
        floors.get(origin).copied()
    }

    pub(super) fn has_floors(&self) -> bool {
        self.floored.load(Ordering::Acquire)
    }

    #[cfg(feature = "html")]
    pub(super) fn set_floor(&self, origin: Origin, gap: Option<Duration>) {
        let mut floors = self.floors.lock().unwrap_or_else(PoisonError::into_inner);
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

    pub(super) fn gate(&self, origin: Origin, max_in_flight: Option<usize>) -> Arc<HostGate> {
        let mut map = self
            .by_origin
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(gate) = map.get(&origin) {
            return Arc::clone(gate);
        }
        let now = Instant::now();
        let floors = self.floors.lock().unwrap_or_else(PoisonError::into_inner);
        map.retain(|kept, gate| {
            Arc::strong_count(gate) > 1 || gate.busy_after(now, floors.get(kept).copied())
        });
        drop(floors);
        let gate = Arc::new(HostGate::new(max_in_flight));
        map.insert(origin, Arc::clone(&gate));
        gate
    }
}

#[derive(Debug)]
pub(super) struct HostGate {
    pub(super) slots: Option<Arc<Semaphore>>,
    last: Mutex<Option<Instant>>,
    quiet: Mutex<Instant>,
    paused: Mutex<Instant>,
    pub(super) in_flight: Arc<AtomicUsize>,
    pub(super) waiting: Arc<AtomicUsize>,
}

impl HostGate {
    fn new(max_in_flight: Option<usize>) -> Self {
        Self {
            slots: max_in_flight.map(|n| Arc::new(Semaphore::new(n))),
            last: Mutex::new(None),
            quiet: Mutex::new(Instant::now()),
            paused: Mutex::new(Instant::now()),
            in_flight: Arc::default(),
            waiting: Arc::default(),
        }
    }

    pub(super) fn commit(gates: &[(&HostGate, Option<Duration>)]) -> bool {
        let now = Instant::now();
        let mut locks = Vec::with_capacity(gates.len());
        for (gate, interval) in gates {
            if locks
                .iter()
                .any(|(held, _, _): &(&HostGate, _, _)| std::ptr::eq(*held, *gate))
            {
                continue;
            }
            let last = gate.last.lock().unwrap_or_else(PoisonError::into_inner);
            if spaced_until(*last, *interval, now) > now || gate.paused_until() > now {
                return false;
            }
            locks.push((*gate, last, *interval));
        }
        for (gate, mut last, interval) in locks {
            *last = Some(now);
            let mut quiet = gate.quiet.lock().unwrap_or_else(PoisonError::into_inner);
            *quiet = (*quiet).max(spaced_until(*last, interval, now));
        }
        true
    }

    pub(super) fn ready_at(&self, interval: Option<Duration>, now: Instant) -> Option<Instant> {
        let last = *self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let end = spaced_until(last, interval, now).max(self.paused_until());
        (end > now).then_some(end)
    }

    pub(super) fn busy_after(&self, now: Instant, floor: Option<Duration>) -> bool {
        let last = *self.last.lock().unwrap_or_else(PoisonError::into_inner);
        *self.quiet.lock().unwrap_or_else(PoisonError::into_inner) > now
            || spaced_until(last, floor, now) > now
            || self.paused_until() > now
    }

    pub(super) fn paused_until(&self) -> Instant {
        *self.paused.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn pause_until(&self, end: Instant) {
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        *paused = (*paused).max(end);
    }

    pub(super) async fn wait_until(at: Instant) {
        tokio::time::sleep_until(at).await;
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
