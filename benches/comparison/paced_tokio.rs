use std::future::Future;
use std::time::Instant;

use crate::paced::{Batch, Plan};

/// Runs the schedule on one Tokio task per worker; `one(worker)` sends and checks one request.
pub async fn run_tasks<F, Fut>(plan: Plan, one: F) -> Batch
where
    F: Fn(usize) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let start = plan.start();
    let first = plan.warmup_end(start);
    let cap = plan.measured_per_worker();
    let handles: Vec<_> = (0..plan.workers)
        .map(|worker| {
            let one = one.clone();
            tokio::spawn(async move {
                let mut batch = Batch::new(first, cap);
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
            })
        })
        .collect();
    let mut batches = Vec::with_capacity(handles.len());
    for handle in handles {
        batches.push(handle.await.expect("paced worker"));
    }
    Batch::merge(batches.into_iter())
}
