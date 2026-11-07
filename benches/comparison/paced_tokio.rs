use std::future::Future;
use std::time::Instant;

use crate::paced::{Plan, Row};

/// Runs the schedule on one Tokio task per worker; `one(worker)` sends and checks one request.
pub async fn run_tasks<F, Fut>(plan: Plan, one: F) -> Vec<Row>
where
    F: Fn(usize) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let start = plan.start();
    let handles: Vec<_> = (0..plan.workers)
        .map(|worker| {
            let one = one.clone();
            tokio::spawn(async move {
                let mut rows = Vec::with_capacity(plan.per_worker() as usize);
                let mut ready = start;
                for i in 0..plan.per_worker() {
                    let intended = plan.intended(start, worker, i);
                    tokio::time::sleep_until(intended.into()).await;
                    let sent = Instant::now();
                    one(worker).await;
                    let done = Instant::now();
                    rows.push(Row {
                        ready,
                        intended,
                        sent,
                        done,
                    });
                    ready = done;
                }
                rows
            })
        })
        .collect();
    let mut rows = Vec::new();
    for handle in handles {
        rows.extend(handle.await.expect("paced worker"));
    }
    rows
}
