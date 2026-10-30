use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::{Event, Trace};
use crate::Kind;

const LATENCY_BOUNDS_MS: [u64; 9] = [10, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000];
const LATENCY_BUCKETS: usize = LATENCY_BOUNDS_MS.len() + 1;
const STATUS_CLASSES: usize = 5;
const CATEGORIES: usize = Kind::ALL.len();

#[derive(Clone, Copy)]
enum BodyKind {
    Complete,
    Failed,
    Dropped,
}

impl BodyKind {
    const ALL: [Self; 3] = [Self::Complete, Self::Failed, Self::Dropped];

    fn of(outcome: Result<(), Option<&std::io::Error>>) -> Self {
        match outcome {
            Ok(()) => Self::Complete,
            Err(Some(_)) => Self::Failed,
            Err(None) => Self::Dropped,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Failed => "failed",
            Self::Dropped => "dropped",
        }
    }
}

/// Counts requests, attempts, statuses, errors, body outcomes, and latency across every session it is passed to.
#[derive(Debug, Default)]
pub struct Metrics {
    requests: AtomicU64,
    attempts: AtomicU64,
    status: [AtomicU64; STATUS_CLASSES],
    errors: [AtomicU64; CATEGORIES],
    bodies: [AtomicU64; BodyKind::ALL.len()],
    latency: [AtomicU64; LATENCY_BUCKETS],
}

fn bump(counter: &AtomicU64, by: u64) {
    counter.fetch_add(by, Ordering::Relaxed);
}

fn read(counter: &AtomicU64) -> u64 {
    counter.load(Ordering::Relaxed)
}

fn load<const N: usize>(counters: &[AtomicU64; N]) -> [u64; N] {
    std::array::from_fn(|i| read(&counters[i]))
}

fn latency_bucket(elapsed: Duration) -> usize {
    let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    LATENCY_BOUNDS_MS
        .iter()
        .position(|bound| ms <= *bound)
        .unwrap_or(LATENCY_BOUNDS_MS.len())
}

impl Metrics {
    /// Returns a new set of zeroed counters, shared so one clone can go to `trace` and one can be read.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Returns the number of requests, counting each `send` once.
    pub fn requests(&self) -> u64 {
        read(&self.requests)
    }

    /// Returns the number of attempts, retries included.
    pub fn attempts(&self) -> u64 {
        read(&self.attempts)
    }

    /// Returns the number of responses whose status is in the `class`xx range, such as 4 for 4xx.
    pub fn status_class(&self, class: u8) -> u64 {
        usize::from(class)
            .checked_sub(1)
            .and_then(|i| self.status.get(i))
            .map(read)
            .unwrap_or(0)
    }

    /// Returns the number of requests that failed with the given error kind.
    pub fn errors(&self, category: Kind) -> u64 {
        read(&self.errors[category as usize])
    }

    /// Returns the number of requests that failed with any error.
    pub fn errors_total(&self) -> u64 {
        load(&self.errors).iter().sum()
    }

    /// Returns the number of streamed bodies read to the end.
    pub fn bodies_complete(&self) -> u64 {
        read(&self.bodies[BodyKind::Complete as usize])
    }

    /// Returns the number of streamed bodies whose read failed.
    pub fn bodies_failed(&self) -> u64 {
        read(&self.bodies[BodyKind::Failed as usize])
    }

    /// Returns the number of streamed bodies dropped before the end.
    pub fn bodies_dropped(&self) -> u64 {
        read(&self.bodies[BodyKind::Dropped as usize])
    }

    /// Returns each latency bucket as its upper bound and count, with `None` for the bucket over 10 seconds.
    pub fn latency(&self) -> Vec<(Option<Duration>, u64)> {
        LATENCY_BOUNDS_MS
            .iter()
            .map(|ms| Some(Duration::from_millis(*ms)))
            .chain(std::iter::once(None))
            .zip(load(&self.latency))
            .collect()
    }
}

impl Trace for Metrics {
    fn event(&self, event: &Event<'_>) {
        match *event {
            Event::Summary {
                attempts,
                elapsed,
                status,
                outcome,
                ..
            } => {
                bump(&self.requests, 1);
                bump(&self.attempts, u64::from(attempts));
                bump(&self.latency[latency_bucket(elapsed)], 1);
                if let Some(class) = status
                    .map(|s| usize::from(s.as_u16() / 100))
                    .filter(|class| (1..=STATUS_CLASSES).contains(class))
                {
                    bump(&self.status[class - 1], 1);
                }
                if let Err(error) = outcome {
                    bump(&self.errors[error.kind() as usize], 1);
                }
            }
            Event::Body { outcome, .. } => {
                bump(&self.bodies[BodyKind::of(outcome) as usize], 1);
            }
            _ => {}
        }
    }
}

impl fmt::Display for Metrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "requests={} attempts={}",
            self.requests(),
            self.attempts()
        )?;
        for (i, count) in load(&self.status).iter().enumerate() {
            write!(f, " status_{}xx={count}", i + 1)?;
        }
        for category in Kind::ALL {
            let count = self.errors(category);
            if count > 0 {
                write!(f, " error_{}={count}", category.as_str())?;
            }
        }
        for kind in BodyKind::ALL {
            write!(
                f,
                " body_{}={}",
                kind.label(),
                read(&self.bodies[kind as usize])
            )?;
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
