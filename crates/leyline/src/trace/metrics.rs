use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::{BodyEnd, BodyOutcome, Summary, Trace};
use crate::ErrorCategory;

const LATENCY_BOUNDS_MS: [u64; 9] = [10, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000];
const LATENCY_BUCKETS: usize = LATENCY_BOUNDS_MS.len() + 1;
const STATUS_CLASSES: usize = 5;
const CATEGORIES: usize = ErrorCategory::ALL.len();
const BODY_LABELS: [&str; 3] = ["complete", "failed", "dropped"];

#[derive(Debug, Default)]
pub struct Metrics {
    requests: AtomicU64,
    attempts: AtomicU64,
    status: [AtomicU64; STATUS_CLASSES],
    errors: [AtomicU64; CATEGORIES],
    bodies: [AtomicU64; BODY_LABELS.len()],
    latency: [AtomicU64; LATENCY_BUCKETS],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetricsSnapshot {
    requests: u64,
    attempts: u64,
    status: [u64; STATUS_CLASSES],
    errors: [u64; CATEGORIES],
    bodies: [u64; BODY_LABELS.len()],
    latency: [u64; LATENCY_BUCKETS],
}

fn bump(counter: &AtomicU64, by: u64) {
    counter.fetch_add(by, Ordering::Relaxed);
}

fn load<const N: usize>(counters: &[AtomicU64; N]) -> [u64; N] {
    std::array::from_fn(|i| counters[i].load(Ordering::Relaxed))
}

fn latency_bucket(elapsed: Duration) -> usize {
    let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    LATENCY_BOUNDS_MS
        .iter()
        .position(|bound| ms <= *bound)
        .unwrap_or(LATENCY_BOUNDS_MS.len())
}

fn body_index(outcome: &BodyOutcome<'_>) -> usize {
    match outcome {
        BodyOutcome::Complete => 0,
        BodyOutcome::Failed(_) => 1,
        BodyOutcome::Dropped => 2,
    }
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            attempts: self.attempts.load(Ordering::Relaxed),
            status: load(&self.status),
            errors: load(&self.errors),
            bodies: load(&self.bodies),
            latency: load(&self.latency),
        }
    }
}

impl Trace for Metrics {
    fn summary(&self, ev: &Summary<'_>) {
        bump(&self.requests, 1);
        bump(&self.attempts, u64::from(ev.attempts));
        bump(&self.latency[latency_bucket(ev.elapsed)], 1);
        if let Some(class) = ev
            .status
            .map(|s| usize::from(s.as_u16() / 100))
            .filter(|class| (1..=STATUS_CLASSES).contains(class))
        {
            bump(&self.status[class - 1], 1);
        }
        if let Err(error) = ev.outcome {
            bump(&self.errors[error.category() as usize], 1);
        }
    }

    fn body(&self, ev: &BodyEnd<'_>) {
        bump(&self.bodies[body_index(&ev.outcome)], 1);
    }
}

impl MetricsSnapshot {
    pub fn requests(&self) -> u64 {
        self.requests
    }

    pub fn attempts(&self) -> u64 {
        self.attempts
    }

    pub fn status_class(&self, class: u8) -> u64 {
        usize::from(class)
            .checked_sub(1)
            .and_then(|i| self.status.get(i))
            .copied()
            .unwrap_or(0)
    }

    pub fn errors(&self, category: ErrorCategory) -> u64 {
        self.errors[category as usize]
    }

    pub fn errors_total(&self) -> u64 {
        self.errors.iter().sum()
    }

    pub fn bodies_complete(&self) -> u64 {
        self.bodies[0]
    }

    pub fn bodies_failed(&self) -> u64 {
        self.bodies[1]
    }

    pub fn bodies_dropped(&self) -> u64 {
        self.bodies[2]
    }

    pub fn latency(&self) -> Vec<(Option<Duration>, u64)> {
        LATENCY_BOUNDS_MS
            .iter()
            .map(|ms| Some(Duration::from_millis(*ms)))
            .chain(std::iter::once(None))
            .zip(self.latency)
            .collect()
    }
}

impl fmt::Display for MetricsSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "requests={} attempts={}", self.requests, self.attempts)?;
        for (i, count) in self.status.iter().enumerate() {
            write!(f, " status_{}xx={count}", i + 1)?;
        }
        for category in ErrorCategory::ALL {
            let count = self.errors(category);
            if count > 0 {
                write!(f, " error_{}={count}", category.as_str())?;
            }
        }
        for (label, count) in BODY_LABELS.iter().zip(self.bodies) {
            write!(f, " body_{label}={count}")?;
        }
        for (bound, count) in self.latency() {
            match bound {
                Some(bound) => write!(f, " latency_le_{}ms={count}", bound.as_millis())?,
                None => write!(f, " latency_le_inf={count}")?,
            }
        }
        Ok(())
    }
}
