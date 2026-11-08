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

/// One request: when its worker became free, when the schedule wanted it sent, when it was sent, and when it finished.
#[derive(Clone, Copy)]
pub struct Row {
    pub ready: Instant,
    pub intended: Instant,
    pub sent: Instant,
    pub done: Instant,
}

impl Row {
    /// Returns the time latency counts from: the scheduled time when the client was behind it, else the send.
    fn origin(&self) -> Instant {
        if self.ready > self.intended {
            self.intended
        } else {
            self.sent
        }
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

    /// Returns how many requests each worker sends, warm-up included.
    pub fn per_worker(&self) -> u64 {
        self.rate.saturating_mul(self.seconds + WARMUP_SECONDS) / self.workers as u64
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

/// Prints the PACED summary line and the slowest 0.1% of requests, leaving out the warm-up.
pub fn report(plan: &Plan, rows: Vec<Row>) {
    let Some(start) = rows.iter().map(|r| r.intended).min() else {
        panic!("paced run completed no requests");
    };
    let first = start + Duration::from_secs(WARMUP_SECONDS);
    let mut rows: Vec<Row> = rows.into_iter().filter(|r| r.intended >= first).collect();
    assert!(
        !rows.is_empty(),
        "paced run measured no requests after the warm-up"
    );
    let last = rows.iter().map(|r| r.done).max().unwrap_or(first);
    let elapsed = last.duration_since(first).as_secs_f64();
    let micros = |d: Duration| d.as_micros() as u32;
    let pct = |mut values: Vec<u32>, p: f64| {
        values.sort_unstable();
        values[((values.len() - 1) as f64 * p).round() as usize]
    };
    let service = |r: &Row| micros(r.done.duration_since(r.sent));
    let lag = |r: &Row| micros(r.sent.duration_since(r.origin()));
    let column = |f: &dyn Fn(&Row) -> u32| rows.iter().map(f).collect::<Vec<u32>>();
    let corrected = |r: &Row| micros(r.done.duration_since(r.origin()));
    println!(
        "PACED rate={} completed={} elapsed_s={elapsed:.3} rps={:.1} service_p50_us={} service_p99_us={} service_p999_us={} service_max_us={} lag_p50_us={} lag_p999_us={} corrected_p50_us={} corrected_p99_us={} corrected_p999_us={}",
        plan.rate,
        rows.len(),
        rows.len() as f64 / elapsed,
        pct(column(&service), 0.50),
        pct(column(&service), 0.99),
        pct(column(&service), 0.999),
        pct(column(&service), 1.0),
        pct(column(&lag), 0.50),
        pct(column(&lag), 0.999),
        pct(column(&corrected), 0.50),
        pct(column(&corrected), 0.99),
        pct(column(&corrected), 0.999),
    );
    rows.sort_unstable_by_key(|r| std::cmp::Reverse(service(r)));
    for r in rows.iter().take((rows.len() / 1000).max(1)) {
        println!(
            "SLOW at_us={} service_us={} lag_us={}",
            micros(r.intended.duration_since(first)),
            service(r),
            lag(r)
        );
    }
}
