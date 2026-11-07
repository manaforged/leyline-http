use std::time::Instant;

use crate::paced::{Plan, Row};

/// Runs the schedule on one thread per worker; `one(worker)` sends and checks one request.
pub fn run_threads(plan: &Plan, one: impl Fn(usize) + Sync) -> Vec<Row> {
    let start = plan.start();
    let one = &one;
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..plan.workers)
            .map(|worker| {
                scope.spawn(move || {
                    let mut ready = start;
                    (0..plan.per_worker())
                        .map(|i| {
                            let intended = plan.intended(start, worker, i);
                            if let Some(wait) = intended.checked_duration_since(Instant::now()) {
                                std::thread::sleep(wait);
                            }
                            let sent = Instant::now();
                            one(worker);
                            let row = Row {
                                ready,
                                intended,
                                sent,
                                done: Instant::now(),
                            };
                            ready = row.done;
                            row
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("paced worker"))
            .collect()
    })
}
