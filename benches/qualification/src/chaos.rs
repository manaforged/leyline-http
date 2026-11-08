use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use leyline::{Retry, Session, Timeout};
use serde_json::{Value, json};

use crate::origin::{Origin, Planner};
use crate::plan::{Expected, ID_HEADER, body_for, chaos_plan, expected, mix};
use crate::{Options, Verdict, probe};

const GRACE: Duration = Duration::from_secs(1);
const STALE_RESEND: u32 = 1;
const FD_SLACK: usize = 8;
const TASK_SLACK: usize = 2;
const MIN_RECOVERY: f64 = 0.8;

#[derive(Default)]
struct Tally {
    sent: AtomicU64,
    ok: AtomicU64,
    failed: AtomicU64,
    timed_out: AtomicU64,
    hung: AtomicU64,
    stale_posts: AtomicU64,
}

#[derive(Debug)]
enum Outcome {
    Body(u16, String),
    Timeout,
    Failure,
    Hang,
}

struct Run {
    session: Session,
    origin: Origin,
    options: Options,
    tally: Tally,
    next_id: AtomicU64,
}

async fn fetch(run: &Run, id: u64, post: bool) -> Outcome {
    let url = run.origin.url.as_str();
    let request = if post {
        run.session.post(url).body("payload")
    } else {
        run.session.get(url)
    };
    let exchange = async {
        let response = request.header(ID_HEADER, id.to_string()).send().await?;
        let status = response.status().as_u16();
        let text = response.text().await?;
        Ok::<_, leyline::Error>((status, text))
    };
    match tokio::time::timeout(run.options.deadline + GRACE, exchange).await {
        Err(_) => Outcome::Hang,
        Ok(Ok((status, text))) => Outcome::Body(status, text),
        Ok(Err(error)) if error.is_timeout() => Outcome::Timeout,
        Ok(Err(_)) => Outcome::Failure,
    }
}

fn judge(run: &Run, id: u64, post: bool, faulted: bool, outcome: &Outcome, verdict: &Verdict) {
    let planned = chaos_plan(run.options.seed, id, run.options.fault_percent);
    if post && faulted && matches!(outcome, Outcome::Failure) && run.origin.state.attempts(id) == 0
    {
        run.tally.stale_posts.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let want = if faulted {
        expected(planned)
    } else {
        Expected::Success
    };
    let tag = format!(
        "id={id} post={post} fault={:?} sticky={} want={want:?}",
        planned.fault, planned.sticky
    );
    let good = match (outcome, want) {
        (Outcome::Hang, _) => {
            run.tally.hung.fetch_add(1, Ordering::Relaxed);
            verdict.fail(format!("hang: no outcome by the deadline: {tag}"));
            return;
        }
        (Outcome::Body(200, text), _) if *text != body_for(id) => {
            verdict.fail(format!("cross-talk or corrupt body {text:?}: {tag}"));
            return;
        }
        (Outcome::Body(200, _), Expected::Success | Expected::Either) => true,
        (Outcome::Body(503, _), Expected::Status503 | Expected::Either) => true,
        (Outcome::Timeout, Expected::Timeout | Expected::Either) => true,
        (Outcome::Failure, Expected::Failure | Expected::Either) => true,
        _ => false,
    };
    if !good {
        verdict.fail(format!("wrong outcome {outcome:?}: {tag}"));
    }
}

fn check_attempts(run: &Run, id: u64, post: bool, verdict: &Verdict) {
    let seen = run.origin.state.attempts(id);
    let limit = if post {
        1
    } else {
        1 + run.options.retries + STALE_RESEND
    };
    if seen > limit {
        let planned = chaos_plan(run.options.seed, id, run.options.fault_percent);
        verdict.fail(format!(
            "attempts: id={id} post={post} reached the origin {seen} times, limit {limit}, fault={:?}",
            planned.fault
        ));
    }
}

fn count(run: &Run, outcome: &Outcome) {
    let counter = match outcome {
        Outcome::Body(..) => &run.tally.ok,
        Outcome::Timeout => &run.tally.timed_out,
        Outcome::Failure => &run.tally.failed,
        Outcome::Hang => return,
    };
    counter.fetch_add(1, Ordering::Relaxed);
}

async fn worker(run: Arc<Run>, verdict: Arc<Verdict>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let id = run.next_id.fetch_add(1, Ordering::Relaxed);
        let post = mix(id ^ 0x55) % 10 == 0;
        let faulted = run.origin.state.faults_on.load(Ordering::SeqCst);
        run.tally.sent.fetch_add(1, Ordering::Relaxed);
        let outcome = fetch(&run, id, post).await;
        count(&run, &outcome);
        judge(&run, id, post, faulted, &outcome, &verdict);
        check_attempts(&run, id, post, &verdict);
    }
}

async fn phase(run: &Arc<Run>, verdict: &Arc<Verdict>, seconds: u64) -> f64 {
    let stop = Arc::new(AtomicBool::new(false));
    let before = run.tally.ok.load(Ordering::Relaxed);
    let start = Instant::now();
    let workers: Vec<_> = (0..run.options.concurrency)
        .map(|_| {
            tokio::spawn(worker(
                Arc::clone(run),
                Arc::clone(verdict),
                Arc::clone(&stop),
            ))
        })
        .collect();
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    stop.store(true, Ordering::Relaxed);
    for w in workers {
        w.await.expect("worker");
    }
    (run.tally.ok.load(Ordering::Relaxed) - before) as f64 / start.elapsed().as_secs_f64()
}

fn session(options: &Options) -> Session {
    Session::builder()
        .retry(
            Retry::new(options.retries)
                .backoff(Duration::from_millis(5), Duration::from_millis(20)),
        )
        .timeout(Timeout::new().total(options.deadline))
        .pool_idle_timeout(Duration::from_secs(1))
        .build()
        .expect("session")
}

pub async fn run(options: &Options, verdict: &Verdict) -> Value {
    let (seed, percent) = (options.seed, options.fault_percent);
    let planner: Planner = Arc::new(move |id, _| chaos_plan(seed, id, percent));
    let run = Arc::new(Run {
        session: session(options),
        origin: Origin::start(options.origin_threads, planner),
        options: options.clone(),
        tally: Tally::default(),
        next_id: AtomicU64::new(1),
    });
    let shared = Arc::new(Verdict::default());
    let start_levels = probe::levels();
    let slice = options.seconds.max(6) / 6;
    let baseline = phase(&run, &shared, slice).await;
    run.origin.state.faults_on.store(true, Ordering::SeqCst);
    let chaos = phase(&run, &shared, slice * 4).await;
    run.origin.state.faults_on.store(false, Ordering::SeqCst);
    let recovery = phase(&run, &shared, slice).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let end_levels = probe::levels();
    let open = run.origin.state.open.load(Ordering::SeqCst);
    if recovery < baseline * MIN_RECOVERY {
        shared.fail(format!(
            "recovery: {recovery:.0} req/s after faults, baseline {baseline:.0}"
        ));
    }
    if open != 0 {
        shared.fail(format!(
            "leak: origin still sees {open} open connections after the idle timeout"
        ));
    }
    for wrong in probe::drift(start_levels, end_levels, FD_SLACK, TASK_SLACK) {
        shared.fail(wrong);
    }
    for message in shared.report() {
        verdict.fail(message);
    }
    json!({
        "total_failures": shared.count(),
        "rule_post": "a POST reaches the origin once; while faults run, a POST may fail only if the origin never saw it, on a reused connection another fault closed",
        "rule_get_attempts": "1 + retries + 1 resend on a stale reused connection",
        "baseline_rps": baseline, "chaos_ok_rps": chaos, "recovery_rps": recovery,
        "sent": run.tally.sent.load(Ordering::Relaxed),
        "ok": run.tally.ok.load(Ordering::Relaxed),
        "failed": run.tally.failed.load(Ordering::Relaxed),
        "timed_out": run.tally.timed_out.load(Ordering::Relaxed),
        "hung": run.tally.hung.load(Ordering::Relaxed),
        "stale_posts": run.tally.stale_posts.load(Ordering::Relaxed),
        "accepted": run.origin.state.accepted.load(Ordering::Relaxed),
        "start": probe::describe(start_levels), "end": probe::describe(end_levels),
        "origin_open": open,
    })
}
