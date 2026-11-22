use std::time::Instant;

use crate::paced::{Batch, Plan};

/// Runs the schedule on one thread per worker; `one(worker)` sends and checks one request.
pub fn run_threads(plan: &Plan, one: impl Fn(usize) + Sync) -> Batch {
    let start = plan.start();
    let first = plan.warmup_end(start);
    let cap = plan.measured_per_worker();
    let one = &one;
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..plan.workers)
            .map(|worker| {
                scope.spawn(move || {
                    let mut batch = Batch::new(first, cap);
                    let mut ready = start;
                    for i in 0..plan.per_worker() {
                        let intended = plan.intended(start, worker, i);
                        if let Some(wait) = intended.checked_duration_since(Instant::now()) {
                            std::thread::sleep(wait);
                        }
                        let sent = Instant::now();
                        one(worker);
                        let done = Instant::now();
                        batch.record(ready, intended, sent, done);
                        ready = done;
                    }
                    batch
                })
            })
            .collect();
        Batch::merge(workers.into_iter().map(|w| w.join().expect("paced worker")))
    })
}
