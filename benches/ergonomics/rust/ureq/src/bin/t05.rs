use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use ureq::Agent;

fn main() {
    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let next = AtomicUsize::new(0);
    let statuses = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..10 {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= 100 {
                        break;
                    }
                    let response = agent
                        .get(format!("http://127.0.0.1:18743/t05/item/{i}"))
                        .call()
                        .unwrap();
                    statuses.lock().unwrap().push(response.status().as_u16());
                }
            });
        }
    });
    let statuses = statuses.into_inner().unwrap();
    let ok = statuses.iter().filter(|status| **status == 200).count();
    let missing = statuses.iter().filter(|status| **status == 404).count();
    println!("{ok} {missing}");
}
