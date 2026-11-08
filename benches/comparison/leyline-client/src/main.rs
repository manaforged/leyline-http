#[path = "../../paced.rs"]
mod paced;
#[path = "../../paced_tokio.rs"]
mod paced_tokio;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod cores;

use std::env;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use leyline::Session;
use leyline::trace::{Event, Trace};
use leyline_cmp_client::{
    WINDOW_WARM, expected_body, fnv1a, session_builder, version, window_line,
};

fn build(ca: Option<&[u8]>, stages: Option<Arc<Stages>>) -> Session {
    let mut b = session_builder(ca);
    if let Some(stages) = stages {
        b = b.trace(stages);
    }
    b.build().expect("leyline session builds")
}

#[derive(Default)]
struct Stages {
    dns: AtomicU64,
    tcp: AtomicU64,
    tls: AtomicU64,
    head: AtomicU64,
}

impl Trace for Stages {
    fn event(&self, event: &Event<'_>) {
        let (stage, elapsed) = match event {
            Event::Dns { elapsed, .. } => (&self.dns, elapsed),
            Event::Connect { elapsed, .. } => (&self.tcp, elapsed),
            Event::Tls { elapsed, .. } => (&self.tls, elapsed),
            Event::Head { elapsed, .. } => (&self.head, elapsed),
            _ => return,
        };
        stage.fetch_add(
            u64::try_from(elapsed.as_nanos()).expect("duration"),
            Ordering::Relaxed,
        );
    }
}

fn main() {
    let mut runtime = tokio::runtime::Builder::new_multi_thread();
    runtime.enable_all();
    if let Some(interval) = env::var("CMP_EVENT_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        runtime.event_interval(interval);
    }
    let runtime = runtime.build().expect("client runtime");
    runtime.block_on(async { tokio::spawn(run()).await.expect("client task") });
}

async fn run() {
    let ca = env::var_os("CMP_CA").map(|path| fs::read(path).expect("read CA certificate"));
    let expected = expected_body();
    let args: Vec<String> = std::env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://127.0.0.1:8443/".to_string());

    let mode = args.get(2).cloned().unwrap_or_default();
    match mode.as_str() {
        "trace" => trace_mode(ca, expected, args, url).await,
        "print" => print_mode(ca, expected, args, url).await,
        "equiv" => equiv_mode(ca, expected, args, url).await,
        "paced" => paced_mode(ca, expected, args, url).await,
        "window" => window_mode(ca, expected, args, url).await,
        _ => bench_mode(ca, expected, args, url).await,
    }
}

async fn trace_mode(ca: Option<Vec<u8>>, expected: Arc<[u8]>, args: Vec<String>, url: String) {
    let n: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1000);
    assert!(n > 0);
    let stages = Arc::new(Stages::default());
    let mut build_ns = 0u128;
    let started = Instant::now();
    for _ in 0..n {
        let started = Instant::now();
        let session = build(ca.as_deref(), Some(Arc::clone(&stages)));
        build_ns += started.elapsed().as_nanos();
        let response = session
            .get(url.as_str())
            .send()
            .await
            .expect("trace request");
        assert_eq!(response.status().as_u16(), 200);
        assert!(
            response.bytes().await.expect("trace body") == expected.as_ref(),
            "response body mismatch"
        );
    }
    let total_ns = started.elapsed().as_nanos() / u128::from(n);
    println!(
        "TRACE n={n} total_ns={total_ns} build_ns={} dns_ns={} tcp_ns={} tls_ns={} head_ns={}",
        build_ns / u128::from(n),
        stages.dns.load(Ordering::Relaxed) / n,
        stages.tcp.load(Ordering::Relaxed) / n,
        stages.tls.load(Ordering::Relaxed) / n,
        stages.head.load(Ordering::Relaxed) / n
    );
    return;
}

async fn print_mode(ca: Option<Vec<u8>>, _expected: Arc<[u8]>, _args: Vec<String>, url: String) {
    let resp = build(ca.as_deref(), None)
        .get(url.as_str())
        .send()
        .await
        .expect("print req");
    println!("{}", resp.text().await.expect("body"));
    return;
}

async fn equiv_mode(ca: Option<Vec<u8>>, _expected: Arc<[u8]>, _args: Vec<String>, url: String) {
    let resp = build(ca.as_deref(), None)
        .get(url.as_str())
        .send()
        .await
        .expect("equiv req");
    let status = resp.status().as_u16();
    let body = resp.bytes().await.expect("body");
    println!(
        "EQUIV leyline status={status} fnv={:016x} len={}",
        fnv1a(&body),
        body.len()
    );
    return;
}

async fn paced_mode(ca: Option<Vec<u8>>, expected: Arc<[u8]>, args: Vec<String>, url: String) {
    let plan = paced::Plan::from_args(&args).expect("paced arguments");
    let sessions: Vec<Session> = (0..paced::connections())
        .map(|_| build(ca.as_deref(), None))
        .collect();
    for session in &sessions {
        let resp = session
            .get(url.as_str())
            .send()
            .await
            .expect("paced warmup");
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            resp.bytes().await.expect("paced warmup body") == expected.as_ref(),
            "response body mismatch"
        );
    }
    let sessions = Arc::new(sessions);
    let rows = paced_tokio::run_tasks(plan, move |worker| {
        let session = sessions[worker % sessions.len()].clone();
        let (url, expected) = (url.clone(), Arc::clone(&expected));
        async move {
            let resp = session.get(url.as_str()).send().await.expect("paced req");
            assert_eq!(resp.status().as_u16(), 200);
            assert!(
                resp.bytes().await.expect("paced body") == expected.as_ref(),
                "response body mismatch"
            );
        }
    })
    .await;
    paced::report(&plan, rows);
}

async fn window_mode(ca: Option<Vec<u8>>, expected: Arc<[u8]>, args: Vec<String>, url: String) {
    let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).expect("seconds");
    let streams: usize = args.get(4).and_then(|s| s.parse().ok()).expect("streams");
    let connections: usize = env::var("CMP_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    let sessions: Vec<Session> = (0..connections)
        .map(|_| build(ca.as_deref(), None))
        .collect();
    for session in &sessions {
        let resp = session
            .get(url.as_str())
            .send()
            .await
            .expect("window warmup");
        assert!(resp.bytes().await.expect("warmup body") == expected.as_ref());
    }
    let open = Instant::now() + WINDOW_WARM;
    let close = open + std::time::Duration::from_secs_f64(seconds);
    let handles: Vec<_> = (0..streams)
        .map(|index| {
            let s = sessions[index % connections].clone();
            let u = url.clone();
            let expected = Arc::clone(&expected);
            tokio::spawn(async move {
                let mut counted = 0u64;
                loop {
                    let resp = s.get(u.as_str()).send().await.expect("window req");
                    assert_eq!(resp.status().as_u16(), 200);
                    assert!(resp.bytes().await.expect("body") == expected.as_ref());
                    let now = Instant::now();
                    if now >= close {
                        return counted;
                    }
                    if now >= open {
                        counted += 1;
                    }
                }
            })
        })
        .collect();
    let mut counts = Vec::with_capacity(streams);
    for handle in handles {
        counts.push(handle.await.expect("join"));
    }
    let metrics = tokio::runtime::Handle::current().metrics();
    let busy: Vec<String> = (0..metrics.num_workers())
        .map(|w| {
            metrics
                .worker_total_busy_duration(w)
                .as_millis()
                .to_string()
        })
        .collect();
    println!(
        "{} worker_busy_ms={}",
        window_line(&counts, seconds),
        busy.join(",")
    );
    return;
}

async fn bench_mode(ca: Option<Vec<u8>>, expected: Arc<[u8]>, args: Vec<String>, url: String) {
    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);

    let connections: usize = env::var("CMP_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    let sessions: Vec<Session> = (0..connections)
        .map(|_| build(ca.as_deref(), None))
        .collect();
    let session = sessions[0].clone();
    let response = session.get(url.as_str()).send().await.expect("warmup");
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.version(), version(), "response HTTP version");
    assert!(
        response.bytes().await.expect("warmup body") == expected.as_ref(),
        "response body mismatch"
    );
    for extra in sessions.iter().skip(1) {
        let response = extra
            .get(url.as_str())
            .send()
            .await
            .expect("connection warmup");
        assert_eq!(response.status().as_u16(), 200);
        assert!(
            response.bytes().await.expect("connection warmup body") == expected.as_ref(),
            "response body mismatch"
        );
    }

    let mut warm_us: Vec<u32> = Vec::with_capacity(warm_n);
    let start = Instant::now();
    for _ in 0..warm_n {
        let t0 = Instant::now();
        let resp = session.get(url.as_str()).send().await.expect("warm req");
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            resp.bytes().await.expect("body") == expected.as_ref(),
            "response body mismatch"
        );
        warm_us.push(t0.elapsed().as_micros() as u32);
    }
    let warm_rps = warm_n as f64 / start.elapsed().as_secs_f64();
    warm_us.sort_unstable();
    let pct = |p: f64| warm_us[((warm_us.len() - 1) as f64 * p).round() as usize];
    let warm_lat = format!(
        " warm_p50_us={} warm_p90_us={} warm_p99_us={}",
        pct(0.50),
        pct(0.90),
        pct(0.99)
    );

    let per = conc_n / conc_c;
    let (conc_rps, mut conc_us) = match cores::threads(connections) {
        Some(threads) => {
            drop((session, sessions));
            tokio::task::block_in_place(|| {
                cores::run(threads, conc_c, per, &url, ca.as_deref(), &expected)
            })
        }
        None => {
            let start = Instant::now();
            let mut handles = Vec::with_capacity(conc_c);
            for index in 0..conc_c {
                let s = sessions[index % connections].clone();
                let u = url.clone();
                let expected = Arc::clone(&expected);
                handles.push(tokio::spawn(async move {
                    let mut lat = Vec::with_capacity(per);
                    for _ in 0..per {
                        let t0 = Instant::now();
                        let resp = s.get(u.as_str()).send().await.expect("conc req");
                        assert_eq!(resp.status().as_u16(), 200);
                        assert!(
                            resp.bytes().await.expect("body") == expected.as_ref(),
                            "response body mismatch"
                        );
                        lat.push(t0.elapsed().as_micros() as u32);
                    }
                    lat
                }));
            }
            let mut conc_us = Vec::with_capacity(per * conc_c);
            for h in handles {
                conc_us.extend(h.await.expect("join"));
            }
            (
                (per * conc_c) as f64 / start.elapsed().as_secs_f64(),
                conc_us,
            )
        }
    };
    conc_us.sort_unstable();
    let cpct = |p: f64| conc_us[((conc_us.len() - 1) as f64 * p).round() as usize];
    let conc_lat = format!(
        " conc_p50_us={} conc_p90_us={} conc_p99_us={} conc_p999_us={}",
        cpct(0.50),
        cpct(0.90),
        cpct(0.99),
        cpct(0.999)
    );

    let start = Instant::now();
    let reconnect = env::var_os("CMP_RECONNECT").map(|_| build(ca.as_deref(), None));
    for _ in 0..cold_n {
        let fresh;
        let client = match &reconnect {
            Some(client) => client,
            None => {
                fresh = build(ca.as_deref(), None);
                &fresh
            }
        };
        let resp = client.get(url.as_str()).send().await.expect("cold req");
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            resp.bytes().await.expect("body") == expected.as_ref(),
            "response body mismatch"
        );
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!(
        "RESULT leyline connections={connections} warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps={cold_rps:.3}{warm_lat}{conc_lat}"
    );
}
