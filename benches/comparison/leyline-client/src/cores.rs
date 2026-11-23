use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, OnceLock};
use std::time::Instant;

use leyline::Session;

use crate::paced::{Batch, Plan};
use crate::paced_tokio;

/// Returns the runtime count `CMP_CORES` asks for, capped at the connection count for HTTP/2.
pub fn threads(connections: usize) -> Option<usize> {
    let wanted = match std::env::var("CMP_CORES").ok()?.as_str() {
        "auto" => std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        n => n.parse().expect("CMP_CORES must be auto or a count"),
    };
    let threads = if leyline_cmp_client::h1() {
        wanted
    } else {
        wanted.min(connections)
    };
    Some(threads.max(1))
}

/// Runs `tasks` request loops over `threads` single-threaded runtimes, each with its own warmed session.
pub fn run(
    threads: usize,
    tasks: usize,
    per: usize,
    url: &str,
    ca: Option<&[u8]>,
    expected: &Arc<[u8]>,
) -> (f64, Vec<u32>) {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    if threads == cores && leyline_cmp_client::h1() {
        return run_shared(cores, tasks, per, url, ca, expected);
    }
    let ready = Arc::new(Barrier::new(threads + 1));
    let go = Arc::new(Barrier::new(threads + 1));
    let handles: Vec<_> = (0..threads)
        .map(|index| {
            let mine = tasks / threads + usize::from(index < tasks % threads);
            let (url, ca, expected) =
                (url.to_owned(), ca.map(<[u8]>::to_vec), Arc::clone(expected));
            let (ready, go) = (Arc::clone(&ready), Arc::clone(&go));
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                runtime.block_on(async move {
                    let session = crate::build(ca.as_deref(), None);
                    fetch(&session, &url, &expected).await;
                    ready.wait();
                    go.wait();
                    let mut set = tokio::task::JoinSet::new();
                    for _ in 0..mine {
                        let (s, u, e) = (session.clone(), url.clone(), Arc::clone(&expected));
                        set.spawn(async move {
                            let mut lat = Vec::with_capacity(per);
                            for _ in 0..per {
                                let t0 = Instant::now();
                                fetch(&s, &u, &e).await;
                                lat.push(t0.elapsed().as_micros() as u32);
                            }
                            lat
                        });
                    }
                    let mut lat = Vec::with_capacity(mine * per);
                    while let Some(part) = set.join_next().await {
                        lat.extend(part.expect("join"));
                    }
                    lat
                })
            })
        })
        .collect();
    ready.wait();
    let start = Instant::now();
    go.wait();
    let mut lat = Vec::with_capacity(tasks * per);
    for handle in handles {
        lat.extend(handle.join().expect("core thread"));
    }
    ((tasks * per) as f64 / start.elapsed().as_secs_f64(), lat)
}

fn run_shared(
    threads: usize,
    tasks: usize,
    per: usize,
    url: &str,
    ca: Option<&[u8]>,
    expected: &Arc<[u8]>,
) -> (f64, Vec<u32>) {
    let ready = Arc::new(Barrier::new(threads + 1));
    let go = Arc::new(Barrier::new(threads + 1));
    let next = Arc::new(AtomicUsize::new(0));
    let builder = leyline_cmp_client::session_builder(ca);
    let (url, expected) = (url.to_owned(), Arc::clone(expected));
    let (r, g) = (Arc::clone(&ready), Arc::clone(&go));
    let runner = std::thread::spawn(move || {
        builder
            .run_per_core(|session| {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let mine = tasks / threads + usize::from(index < tasks % threads);
                let (url, expected, ready, go) = (
                    url.clone(),
                    Arc::clone(&expected),
                    Arc::clone(&r),
                    Arc::clone(&g),
                );
                async move {
                    fetch(&session, &url, &expected).await;
                    ready.wait();
                    go.wait();
                    let mut set = tokio::task::JoinSet::new();
                    for _ in 0..mine {
                        let (s, u, e) = (session.clone(), url.clone(), Arc::clone(&expected));
                        set.spawn(async move {
                            let mut lat = Vec::with_capacity(per);
                            for _ in 0..per {
                                let t0 = Instant::now();
                                fetch(&s, &u, &e).await;
                                lat.push(t0.elapsed().as_micros() as u32);
                            }
                            lat
                        });
                    }
                    let mut lat = Vec::with_capacity(mine * per);
                    while let Some(part) = set.join_next().await {
                        lat.extend(part.expect("join"));
                    }
                    lat
                }
            })
            .expect("per-core sessions build")
    });
    ready.wait();
    let start = Instant::now();
    go.wait();
    let lat: Vec<u32> = runner.join().expect("per-core runner").concat();
    ((tasks * per) as f64 / start.elapsed().as_secs_f64(), lat)
}

async fn fetch(session: &Session, url: &str, expected: &[u8]) {
    let resp = session.get(url).send().await.expect("conc req");
    assert_eq!(resp.status().as_u16(), 200);
    assert!(
        resp.bytes().await.expect("body") == expected,
        "response body mismatch"
    );
}

async fn run_paced_core(
    session: Session,
    index: usize,
    threads: usize,
    plan: Plan,
    start: Arc<OnceLock<Instant>>,
    ready: Arc<Barrier>,
    go: Arc<Barrier>,
    url: String,
    expected: Arc<[u8]>,
) -> Option<Batch> {
    let mine: Vec<usize> = (index..plan.workers).step_by(threads).collect();
    if !mine.is_empty() {
        fetch(&session, &url, &expected).await;
    }
    ready.wait();
    go.wait();
    if mine.is_empty() {
        return None;
    }
    let start = *start.get().expect("paced start");
    let mut set = tokio::task::JoinSet::new();
    for worker in mine {
        let (s, u, e) = (session.clone(), url.clone(), Arc::clone(&expected));
        set.spawn(async move {
            paced_tokio::run_worker(plan, start, worker, move |_| {
                let (s, u, e) = (s.clone(), u.clone(), Arc::clone(&e));
                async move { fetch(&s, &u, &e).await }
            })
            .await
        });
    }
    let mut batches = Vec::new();
    while let Some(part) = set.join_next().await {
        batches.push(part.expect("paced worker join"));
    }
    Some(Batch::merge(batches.into_iter()))
}

/// Runs an open-loop paced schedule across `threads` single-threaded runtimes.
pub fn run_paced(
    threads: usize,
    plan: Plan,
    url: &str,
    ca: Option<&[u8]>,
    expected: &Arc<[u8]>,
) -> Batch {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let ready = Arc::new(Barrier::new(threads + 1));
    let go = Arc::new(Barrier::new(threads + 1));
    let start = Arc::new(OnceLock::new());
    if threads == cores && leyline_cmp_client::h1() {
        let next = Arc::new(AtomicUsize::new(0));
        let builder = leyline_cmp_client::session_builder(ca);
        let (url, expected) = (url.to_owned(), Arc::clone(expected));
        let (r, g, st) = (Arc::clone(&ready), Arc::clone(&go), Arc::clone(&start));
        let runner = std::thread::spawn(move || {
            builder
                .run_per_core(|session| {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    run_paced_core(
                        session,
                        index,
                        threads,
                        plan,
                        Arc::clone(&st),
                        Arc::clone(&r),
                        Arc::clone(&g),
                        url.clone(),
                        Arc::clone(&expected),
                    )
                })
                .expect("per-core sessions build")
        });
        ready.wait();
        let _ = start.set(plan.start());
        go.wait();
        return Batch::merge(
            runner
                .join()
                .expect("per-core runner")
                .into_iter()
                .flatten(),
        );
    }
    let handles: Vec<_> = (0..threads)
        .map(|index| {
            let (url, ca, expected) =
                (url.to_owned(), ca.map(<[u8]>::to_vec), Arc::clone(expected));
            let (r, g, st) = (Arc::clone(&ready), Arc::clone(&go), Arc::clone(&start));
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                runtime.block_on(async move {
                    let session = crate::build(ca.as_deref(), None);
                    run_paced_core(session, index, threads, plan, st, r, g, url, expected).await
                })
            })
        })
        .collect();
    ready.wait();
    let _ = start.set(plan.start());
    go.wait();
    Batch::merge(
        handles
            .into_iter()
            .filter_map(|h| h.join().expect("core paced thread")),
    )
}
