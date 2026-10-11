use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use super::host_limits::Origin;

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
}

#[derive(Debug, Default)]
pub(super) struct Gates {
    pub(super) by_origin: Mutex<HashMap<Origin, Arc<HostGate>>>,
    pub(super) total: Option<Arc<Semaphore>>,
    floors: Mutex<HashMap<Origin, Duration>>,
}

impl Gates {
    pub(super) fn new(max_total: Option<usize>) -> Self {
        Self {
            by_origin: Mutex::default(),
            total: max_total.map(|n| Arc::new(Semaphore::new(n))),
            floors: Mutex::default(),
        }
    }

    pub(super) fn floor(&self, origin: &Origin) -> Option<Duration> {
        let floors = self.floors.lock().unwrap_or_else(PoisonError::into_inner);
        floors.get(origin).copied()
    }

    pub(super) fn has_floors(&self) -> bool {
        !self
            .floors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    #[cfg(feature = "html")]
    pub(super) fn raise_floor(&self, origin: Origin, gap: Duration) {
        let mut floors = self.floors.lock().unwrap_or_else(PoisonError::into_inner);
        let floor = floors.entry(origin).or_insert(gap);
        *floor = (*floor).max(gap);
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
        map.retain(|_, gate| Arc::strong_count(gate) > 1 || gate.busy_after(now));
        let gate = Arc::new(HostGate::new(max_in_flight));
        map.insert(origin, Arc::clone(&gate));
        gate
    }
}

#[derive(Debug)]
pub(super) struct HostGate {
    pub(super) slots: Option<Arc<Semaphore>>,
    next: Mutex<Instant>,
    paused: Mutex<Instant>,
    pub(super) in_flight: Arc<AtomicUsize>,
    pub(super) waiting: Arc<AtomicUsize>,
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

    pub(super) fn try_admit(&self, interval: Option<Duration>) -> bool {
        let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        if *next > now || self.paused_until() > now {
            return false;
        }
        if let Some(interval) = interval {
            *next = now.checked_add(interval).unwrap_or(now);
        }
        true
    }

    pub(super) async fn wait_spacing(&self) {
        let next = *self.next.lock().unwrap_or_else(PoisonError::into_inner);
        tokio::time::sleep_until(next).await;
    }

    pub(super) fn busy_after(&self, now: Instant) -> bool {
        *self.next.lock().unwrap_or_else(PoisonError::into_inner) > now || self.paused_until() > now
    }

    pub(super) fn paused_until(&self) -> Instant {
        *self.paused.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn pause_until(&self, end: Instant) {
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        *paused = (*paused).max(end);
    }

    pub(super) async fn wait_unpaused(&self) {
        loop {
            let end = self.paused_until();
            if end <= Instant::now() {
                return;
            }
            tokio::time::sleep_until(end).await;
        }
    }
}
