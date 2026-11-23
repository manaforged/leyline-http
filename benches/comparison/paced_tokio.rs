use std::future::Future;
use std::time::Instant;

use crate::paced::{Batch, Plan};

/// Runs one paced worker's open-loop schedule starting at `start`.
pub async fn run_worker<F, Fut>(plan: Plan, start: Instant, worker: usize, one: F) -> Batch
where
    F: Fn(usize) -> Fut,
    Fut: Future<Output = ()>,
{
    let first = plan.warmup_end(start);
    let mut batch = Batch::new(first, plan.measured_per_worker());
    let mut ready = start;
    for i in 0..plan.per_worker() {
        let intended = plan.intended(start, worker, i);
        tokio::time::sleep_until(intended.into()).await;
        let sent = Instant::now();
        one(worker).await;
        let done = Instant::now();
        batch.record(ready, intended, sent, done);
        ready = done;
    }
    batch
}

/// Runs the schedule on one Tokio task per worker; `one(worker)` sends and checks one request.
pub async fn run_tasks<F, Fut>(plan: Plan, one: F) -> Batch
where
    F: Fn(usize) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let start = plan.start();
    let handles: Vec<_> = (0..plan.workers)
        .map(|worker| {
            let one = one.clone();
            tokio::spawn(async move { run_worker(plan, start, worker, one).await })
        })
        .collect();
    let mut batches = Vec::with_capacity(handles.len());
    for handle in handles {
        batches.push(handle.await.expect("paced worker"));
    }
    Batch::merge(batches.into_iter())
}
