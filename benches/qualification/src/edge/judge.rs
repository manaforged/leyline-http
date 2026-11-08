use std::sync::atomic::Ordering;

use hdrhistogram::Histogram;
use serde_json::{Value, json};

use super::{Latency, MIN_ACHIEVED, MIN_RECOVERY, Phase, Shared};
use crate::Verdict;

pub(super) fn merged(shards: &[Latency]) -> Histogram<u64> {
    let mut all = Histogram::new_with_bounds(1, 60_000_000, 3).expect("histogram");
    for shard in shards {
        all.add(&*shard.lock().unwrap()).expect("merge");
    }
    all
}

pub(super) fn summary(h: &Histogram<u64>) -> Value {
    json!({"count": h.len(), "p50_us": h.value_at_quantile(0.5), "p99_us": h.value_at_quantile(0.99),
           "p999_us": h.value_at_quantile(0.999), "max_us": h.max()})
}

pub(super) fn reuse(windows: &[(u64, u64)], from: usize, to: usize) -> Option<f64> {
    let (a, b) = (windows.get(from)?, windows.get(to)?);
    let served = b.1.checked_sub(a.1)?;
    (served > 0).then(|| 1.0 - (b.0 - a.0) as f64 / served as f64)
}

pub(super) fn tail_growth(samples: &[(f64, u64, u64, u64)], seconds: f64) -> Option<f64> {
    let mean = |from: f64, to: f64| {
        let picked: Vec<u64> = samples
            .iter()
            .filter(|s| s.0 >= from * seconds && s.0 < to * seconds)
            .map(|s| s.1)
            .collect();
        (!picked.is_empty()).then(|| picked.iter().sum::<u64>() as f64 / picked.len() as f64)
    };
    Some(mean(0.85, 1.0)? / mean(0.75, 0.85)?)
}

pub(super) struct Limits {
    pub(super) ratio: f64,
    pub(super) min_reuse: f64,
    pub(super) min_goodput: f64,
    pub(super) max_rss_kib: u64,
    pub(super) max_rss_growth: f64,
}

pub(super) fn judge_goodput(shared: &Shared, limits: &Limits, verdict: &Verdict) {
    let window = 0.3 * shared.layout.seconds;
    for (phase, name) in [(Phase::Fault, "during load"), (Phase::After, "after load")] {
        let rate = shared.good[phase as usize].load(Ordering::Relaxed) as f64 / window;
        if rate == 0.0 || rate < limits.min_goodput {
            verdict.fail(format!(
                "goodput: {rate:.0} good req/s {name}, floor {:.0}",
                limits.min_goodput
            ));
        }
    }
}

pub(super) fn judge_shock(shared: &Shared, seconds: [f64; 2], verdict: &Verdict) {
    let [before, after] = [Phase::Before, Phase::After]
        .map(|p| shared.good[p as usize].load(Ordering::Relaxed) as f64);
    let (before_rate, after_rate) = (before / seconds[0], after / seconds[1]);
    if after_rate < before_rate * MIN_RECOVERY {
        verdict.fail(format!(
            "recovery: {after_rate:.0} good req/s after the shock, {before_rate:.0} before it"
        ));
    }
}

pub(super) fn judge(
    shared: &Shared,
    limits: &Limits,
    achieved: f64,
    before_reuse: Option<f64>,
    growth: Option<f64>,
    rss_peak: u64,
    verdict: &Verdict,
) {
    let ratio = limits.ratio;
    let layout = &shared.layout;
    for message in shared.failures.lock().unwrap().iter() {
        verdict.fail(message.clone());
    }
    let outlived = shared.outlived.load(Ordering::Relaxed);
    if outlived > 0 {
        verdict.fail(format!(
            "deadline: {outlived} requests outlived their deadline"
        ));
    }
    if limits.max_rss_kib > 0 && rss_peak > limits.max_rss_kib {
        verdict.fail(format!(
            "memory: RSS peak {rss_peak} KiB, limit {} KiB",
            limits.max_rss_kib
        ));
    }
    if layout.overload {
        judge_goodput(shared, limits, verdict);
        judge_shock(
            shared,
            [0.3 * layout.seconds, 0.3 * layout.seconds],
            verdict,
        );
        return;
    }
    if achieved < layout.rate * MIN_ACHIEVED {
        verdict.fail(format!("rate: achieved {achieved:.0} req/s of {:.0} offered; this run says nothing about that rate", layout.rate));
    }
    if before_reuse.is_some_and(|r| r < limits.min_reuse) {
        verdict.fail(format!(
            "reuse: {:.4} before faults, floor {}",
            before_reuse.unwrap_or(0.0),
            limits.min_reuse
        ));
    }
    let before = merged(&shared.healthy[Phase::Before as usize]).value_at_quantile(0.99);
    let during = merged(&shared.healthy[Phase::Fault as usize]).value_at_quantile(0.99);
    if during as f64 > before as f64 * ratio {
        verdict.fail(format!("isolation: healthy p99 {during} us during faults, {before} us before, ratio limit {ratio}"));
    }
    let after = merged(&shared.healthy[Phase::After as usize]).value_at_quantile(0.99);
    if after as f64 > before as f64 * ratio {
        verdict.fail(format!(
            "recovery: healthy p99 {after} us after faults, {before} us before, ratio limit {ratio}"
        ));
    }
    if growth.is_some_and(|g| g > limits.max_rss_growth) {
        verdict.fail(format!(
            "memory: RSS grew {:.3}x over the last 25% of the run",
            growth.unwrap_or(0.0)
        ));
    }
}
