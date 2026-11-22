use std::time::{Duration, Instant};

const LEAD: Duration = Duration::from_millis(50);
const WARMUP_SECONDS: u64 = 1;
const MIN_GAP: Duration = Duration::from_millis(2);

/// An open-loop schedule: `workers` senders share `rate` requests per second for a 1 s warm-up plus `seconds`.
#[derive(Clone, Copy)]
pub struct Plan {
    pub rate: u64,
    pub seconds: u64,
    pub workers: usize,
}

/// Compact post-warmup latency sample in microseconds (8 bytes).
#[derive(Clone, Copy)]
pub struct Sample {
    pub service_us: u32,
    pub lag_us: u32,
}

/// Collected post-warmup samples and timing window for one paced run.
pub struct Batch {
    pub first: Instant,
    pub last: Instant,
    pub samples: Vec<Sample>,
}

impl Batch {
    /// Creates an empty post-warmup collector starting at `first` with `capacity`.
    pub fn new(first: Instant, capacity: usize) -> Self {
        Self {
            first,
            last: first,
            samples: Vec::with_capacity(capacity),
        }
    }

    /// Records one completed request, discarding warm-up observations before `first`.
    #[inline]
    pub fn record(&mut self, ready: Instant, intended: Instant, sent: Instant, done: Instant) {
        if intended < self.first {
            return;
        }
        if done > self.last {
            self.last = done;
        }
        let origin = if ready > intended { intended } else { sent };
        self.samples.push(Sample {
            service_us: done.saturating_duration_since(sent).as_micros() as u32,
            lag_us: sent.saturating_duration_since(origin).as_micros() as u32,
        });
    }

    /// Merges per-worker or per-lane batches into one batch.
    pub fn merge(mut batches: impl Iterator<Item = Self>) -> Self {
        let Some(mut out) = batches.next() else {
            let now = Instant::now();
            return Self::new(now, 0);
        };
        for batch in batches {
            if batch.first < out.first {
                out.first = batch.first;
            }
            if batch.last > out.last {
                out.last = batch.last;
            }
            out.samples.extend_from_slice(&batch.samples);
        }
        out
    }
}

impl Plan {
    /// Reads `<url> paced <rate> <seconds> <workers>` from the command line.
    pub fn from_args(args: &[String]) -> Option<Plan> {
        if args.get(2).map(String::as_str) != Some("paced") {
            return None;
        }
        let field = |i: usize, name: &str| -> u64 {
            args.get(i)
                .and_then(|s| s.parse().ok())
                .filter(|&n| n > 0)
                .unwrap_or_else(|| panic!("paced needs a positive {name}"))
        };
        let plan = Plan {
            rate: field(3, "rate"),
            seconds: field(4, "seconds"),
            workers: field(5, "worker count") as usize,
        };
        assert!(
            plan.gap() >= MIN_GAP,
            "paced needs at least {MIN_GAP:?} between one worker's requests; raise the worker count or lower the rate"
        );
        Some(plan)
    }

    /// Returns the first send time, a short lead after now.
    pub fn start(&self) -> Instant {
        Instant::now() + LEAD
    }

    /// Returns the instant when the warm-up window ends.
    pub fn warmup_end(&self, start: Instant) -> Instant {
        start + Duration::from_secs(WARMUP_SECONDS)
    }

    /// Returns how many requests each worker sends, warm-up included.
    pub fn per_worker(&self) -> u64 {
        self.rate.saturating_mul(self.seconds + WARMUP_SECONDS) / self.workers as u64
    }

    /// Returns the approximate post-warmup sample count for one worker.
    pub fn measured_per_worker(&self) -> usize {
        (self.rate.saturating_mul(self.seconds) / self.workers as u64) as usize + 1
    }

    fn gap(&self) -> Duration {
        Duration::from_secs_f64(self.workers as f64 / self.rate as f64)
    }

    /// Returns when `worker` should send its request number `i`.
    pub fn intended(&self, start: Instant, worker: usize, i: u64) -> Instant {
        start
            + self.gap().mul_f64(worker as f64 / self.workers as f64)
            + self.gap().mul_f64(i as f64)
    }
}

/// Returns the connection count in `CMP_CONNECTIONS`, at least 1.
pub fn connections() -> usize {
    std::env::var("CMP_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1)
}

fn quantiles(buf: &mut [u32], qs: &[f64]) -> Vec<u32> {
    let last = buf.len() - 1;
    let mut indices: Vec<(usize, usize)> = qs
        .iter()
        .enumerate()
        .map(|(i, &q)| (i, (last as f64 * q).round() as usize))
        .collect();
    indices.sort_unstable_by_key(|&(_, idx)| idx);
    let mut out = vec![0u32; qs.len()];
    let mut hi = buf.len();
    for &(slot, idx) in indices.iter().rev() {
        let (_, val, _) = buf[..hi].select_nth_unstable(idx);
        out[slot] = *val;
        hi = idx + 1;
    }
    out
}

/// Prints the PACED summary line using one reused scratch buffer and linear-time selection.
pub fn report(plan: &Plan, batch: Batch) {
    let n = batch.samples.len();
    assert!(n > 0, "paced run measured no requests after the warm-up");
    let elapsed = batch.last.duration_since(batch.first).as_secs_f64();
    let mut buf = Vec::with_capacity(n);

    buf.extend(batch.samples.iter().map(|s| s.service_us));
    let svc_max = buf.iter().copied().max().unwrap_or(0);
    let svc = quantiles(&mut buf, &[0.50, 0.99, 0.999]);

    buf.clear();
    buf.extend(batch.samples.iter().map(|s| s.lag_us));
    let lag = quantiles(&mut buf, &[0.50, 0.999]);

    buf.clear();
    buf.extend(
        batch
            .samples
            .iter()
            .map(|s| s.service_us.saturating_add(s.lag_us)),
    );
    let corr = quantiles(&mut buf, &[0.50, 0.99, 0.999]);

    println!(
        "PACED rate={} completed={} elapsed_s={elapsed:.3} rps={:.1} service_p50_us={} service_p99_us={} service_p999_us={} service_max_us={} lag_p50_us={} lag_p999_us={} corrected_p50_us={} corrected_p99_us={} corrected_p999_us={}",
        plan.rate,
        n,
        n as f64 / elapsed,
        svc[0],
        svc[1],
        svc[2],
        svc_max,
        lag[0],
        lag[1],
        corr[0],
        corr[1],
        corr[2],
    );
}
