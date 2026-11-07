use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use leyline::{Session, Timeout};
use serde_json::{Value, json};

use crate::origin::{Origin, Planner};
use crate::plan::{Fault, ID_HEADER, Planned, body_for, mix};
use crate::{Options, Verdict, probe};

const PER_HOST: usize = 8;
const FD_SLACK: usize = 8;
const TASK_SLACK: usize = 2;
const MAX_DELAY_US: u64 = 3000;

fn planner(seed: u64) -> Planner {
    Arc::new(move |id, _| Planned {
        fault: Fault::Delay(Duration::from_micros(mix(seed ^ id) % MAX_DELAY_US)),
        sticky: true,
    })
}

struct Run {
    session: Session,
    url: String,
    seed: u64,
    next_id: AtomicU64,
    cancelled: AtomicU64,
    completed: AtomicU64,
}

async fn exchange(session: Session, url: String, id: u64) -> Result<String, leyline::Error> {
    let response = session.get(&url).header(ID_HEADER, id.to_string()).send().await?;
    response.text().await
}

async fn cancelled_by_timeout(run: &Run, id: u64, wait: Duration) {
    let future = exchange(run.session.clone(), run.url.clone(), id);
    if tokio::time::timeout(wait, future).await.is_err() {
        run.cancelled.fetch_add(1, Ordering::Relaxed);
    }
}

async fn cancelled_by_select(run: &Run, id: u64, wait: Duration) {
    tokio::select! {
        _ = exchange(run.session.clone(), run.url.clone(), id) => {}
        () = tokio::time::sleep(wait) => { run.cancelled.fetch_add(1, Ordering::Relaxed); }
    }
}

async fn cancelled_by_abort(run: &Run, id: u64, wait: Duration) {
    let task = tokio::spawn(exchange(run.session.clone(), run.url.clone(), id));
    tokio::time::sleep(wait).await;
    task.abort();
    if matches!(task.await, Err(e) if e.is_cancelled()) {
        run.cancelled.fetch_add(1, Ordering::Relaxed);
    }
}

async fn checked(run: &Run, id: u64, verdict: &Verdict) {
    match exchange(run.session.clone(), run.url.clone(), id).await {
        Ok(text) if text == body_for(id) => {
            run.completed.fetch_add(1, Ordering::Relaxed);
        }
        Ok(text) => verdict.fail(format!("cross-talk: id={id} received {text:?} after cancellations")),
        Err(error) => verdict.fail(format!("id={id} failed after cancellations: {:?}", error.kind())),
    }
}

async fn worker(run: Arc<Run>, verdict: Arc<Verdict>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let id = run.next_id.fetch_add(1, Ordering::Relaxed);
        let roll = mix(run.seed ^ mix(id));
        let wait = Duration::from_micros((roll >> 8) % MAX_DELAY_US);
        match roll % 4 {
            0 => cancelled_by_timeout(&run, id, wait).await,
            1 => cancelled_by_select(&run, id, wait).await,
            2 => cancelled_by_abort(&run, id, wait).await,
            _ => checked(&run, id, &verdict).await,
        }
    }
}

async fn permits_survive(run: &Arc<Run>, verdict: &Verdict) -> u128 {
    let start = Instant::now();
    let tasks: Vec<_> = (0..PER_HOST)
        .map(|_| {
            let id = run.next_id.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(exchange(run.session.clone(), run.url.clone(), id))
        })
        .collect();
    for task in tasks {
        let done = tokio::time::timeout(Duration::from_secs(5), task).await;
        if !matches!(done, Ok(Ok(Ok(_)))) {
            verdict.fail(format!("permits: a request could not finish after the storm: {done:?}"));
        }
    }
    start.elapsed().as_millis()
}

pub async fn run(options: &Options, verdict: &Verdict) -> Value {
    let origin = Origin::start(options.origin_threads, planner(options.seed));
    origin.state.faults_on.store(true, Ordering::SeqCst);
    let session = Session::builder()
        .retry(0)
        .timeout(Timeout::new().total(Duration::from_secs(5)))
        .max_connections_per_host(PER_HOST)
        .pool_idle_timeout(Duration::from_secs(1))
        .build()
        .expect("session");
    let run = Arc::new(Run {
        session,
        url: origin.url.clone(),
        seed: options.seed,
        next_id: AtomicU64::new(1),
        cancelled: AtomicU64::new(0),
        completed: AtomicU64::new(0),
    });
    let start_levels = probe::levels();
    let shared = Arc::new(Verdict::default());
    let stop = Arc::new(AtomicBool::new(false));
    let workers: Vec<_> = (0..options.concurrency)
        .map(|_| tokio::spawn(worker(Arc::clone(&run), Arc::clone(&shared), Arc::clone(&stop))))
        .collect();
    tokio::time::sleep(Duration::from_secs(options.seconds)).await;
    stop.store(true, Ordering::Relaxed);
    for w in workers {
        w.await.expect("worker");
    }
    let after_storm_ms = permits_survive(&run, &shared).await;
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let end_levels = probe::levels();
    let open = origin.state.open.load(Ordering::SeqCst);
    if open != 0 {
        shared.fail(format!("leak: origin still sees {open} open connections after the idle timeout"));
    }
    if run.completed.load(Ordering::Relaxed) == 0 || run.cancelled.load(Ordering::Relaxed) == 0 {
        shared.fail("the run proved nothing: no requests completed or none were cancelled".into());
    }
    for wrong in probe::drift(start_levels, end_levels, FD_SLACK, TASK_SLACK) {
        shared.fail(wrong);
    }
    for message in shared.report() {
        verdict.fail(message);
    }
    json!({
        "cancelled": run.cancelled.load(Ordering::Relaxed),
        "completed": run.completed.load(Ordering::Relaxed),
        "after_storm_ms": after_storm_ms,
        "accepted": origin.state.accepted.load(Ordering::Relaxed),
        "start": probe::describe(start_levels), "end": probe::describe(end_levels),
        "origin_open": open,
    })
}
