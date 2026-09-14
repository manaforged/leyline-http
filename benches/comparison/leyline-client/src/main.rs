#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::env;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use leyline::trace::{Connect, Dns, Head, Tls, Trace};
use leyline::{Browser, Session};

fn build(ca: Option<&[u8]>, stages: Option<Arc<Stages>>) -> Session {
    let browser = match std::env::var("LEYLINE_CHROME").as_deref() {
        Ok("bare") => None,
        Ok("149") => Some(Browser::Chrome149),
        _ => Some(Browser::Chrome152),
    };
    let mut b = Session::builder().http2();
    if let Some(ca) = ca {
        b = b
            .without_system_roots()
            .without_env_roots()
            .add_root_certificate_der(ca);
    } else {
        b = b.danger_accept_invalid_certs(true);
    }
    if let Some(browser) = browser {
        b = b.browser(browser);
    }
    if let Some(path) = env::var_os("CMP_HEADERS") {
        let headers = fs::read_to_string(path).expect("read request headers");
        b = b.extra_headers(
            headers
                .lines()
                .map(|line| line.split_once('\t').expect("header name and value")),
        );
    }
    if let Ok(p) = std::env::var("PROXY")
        && !p.is_empty()
    {
        b = b.proxy(p);
    }
    if let Some(stages) = stages {
        b = b.trace(stages);
    }
    b.build().expect("leyline session builds")
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[derive(Default)]
struct Stages {
    dns: AtomicU64,
    tcp: AtomicU64,
    tls: AtomicU64,
    head: AtomicU64,
}

impl Trace for Stages {
    fn dns(&self, ev: &Dns<'_>) {
        self.dns.fetch_add(
            u64::try_from(ev.elapsed.as_nanos()).expect("duration"),
            Ordering::Relaxed,
        );
    }

    fn connect(&self, ev: &Connect<'_>) {
        self.tcp.fetch_add(
            u64::try_from(ev.elapsed.as_nanos()).expect("duration"),
            Ordering::Relaxed,
        );
    }

    fn tls(&self, ev: &Tls<'_>) {
        self.tls.fetch_add(
            u64::try_from(ev.elapsed.as_nanos()).expect("duration"),
            Ordering::Relaxed,
        );
    }

    fn head(&self, ev: &Head<'_>) {
        self.head.fetch_add(
            u64::try_from(ev.elapsed.as_nanos()).expect("duration"),
            Ordering::Relaxed,
        );
    }
}

#[tokio::main]
async fn main() {
    let ca = env::var_os("CMP_CA").map(|path| fs::read(path).expect("read CA certificate"));
    let expected: Arc<[u8]> = match env::var_os("CMP_BODY") {
        Some(size) => (0..size
            .to_str()
            .expect("CMP_BODY is UTF-8")
            .parse::<usize>()
            .expect("CMP_BODY is a byte count"))
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>()
            .into(),
        None => Arc::from(b"ok-10byte!".as_slice()),
    };
    let args: Vec<String> = std::env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://127.0.0.1:8443/".to_string());

    if args.get(2).map(|s| s.as_str()) == Some("trace") {
        let n: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1000);
        assert!(n > 0);
        let stages = Arc::new(Stages::default());
        let mut build_ns = 0u128;
        let started = Instant::now();
        for _ in 0..n {
            let started = Instant::now();
            let session = build(ca.as_deref(), Some(Arc::clone(&stages)));
            build_ns += started.elapsed().as_nanos();
            let mut response = session.get(url.as_str()).await.expect("trace request");
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

    if args.get(2).map(|s| s.as_str()) == Some("print") {
        let mut resp = build(ca.as_deref(), None)
            .get(url.as_str())
            .await
            .expect("print req");
        println!("{}", resp.text().await.expect("body"));
        return;
    }

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let mut resp = build(ca.as_deref(), None)
            .get(url.as_str())
            .await
            .expect("equiv req");
        let status = resp.status().as_u16();
        let body = resp.bytes().await.expect("body");
        println!(
            "EQUIV leyline status={status} fnv={:016x} len={}",
            fnv1a(body),
            body.len()
        );
        return;
    }

    if args.get(2).map(|s| s.as_str()) == Some("paced") {
        let rate: u64 = args.get(3).and_then(|s| s.parse().ok()).expect("rate");
        let seconds: u64 = args.get(4).and_then(|s| s.parse().ok()).expect("seconds");
        let concurrency: usize = args.get(5).and_then(|s| s.parse().ok()).expect("concurrency");
        assert!(rate > 0 && seconds > 0 && concurrency > 0);
        let session = build(ca.as_deref(), None);
        let mut r = session.get(url.as_str()).await.expect("paced warmup");
        assert_eq!(r.status().as_u16(), 200);
        assert!(r.bytes().await.expect("paced warmup body") == expected.as_ref(), "response body mismatch");
        let (tx, rx) = tokio::sync::mpsc::channel::<tokio::time::Instant>(concurrency);
        let rx = Arc::new(tokio::sync::Mutex::new(rx));
        let mut handles = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let s = session.clone();
            let u = url.clone();
            let expected = Arc::clone(&expected);
            let rx = Arc::clone(&rx);
            handles.push(tokio::spawn(async move {
                let mut service = Vec::new();
                let mut corrected = Vec::new();
                loop {
                    let intended = {
                        let mut guard = rx.lock().await;
                        match guard.recv().await {
                            Some(t) => t,
                            None => break,
                        }
                    };
                    let actual = tokio::time::Instant::now();
                    let mut resp = s.get(u.as_str()).await.expect("paced req");
                    assert_eq!(resp.status().as_u16(), 200);
                    assert!(resp.bytes().await.expect("paced body") == expected.as_ref(), "response body mismatch");
                    let done = tokio::time::Instant::now();
                    service.push(done.duration_since(actual).as_micros() as u32);
                    corrected.push(done.duration_since(intended).as_micros() as u32);
                }
                (service, corrected)
            }));
        }
        let start = tokio::time::Instant::now();
        let total = rate.saturating_mul(seconds);
        for i in 0..total {
            let due = start + std::time::Duration::from_secs_f64(i as f64 / rate as f64);
            tokio::time::sleep_until(due).await;
            if tx.send(due).await.is_err() {
                break;
            }
        }
        drop(tx);
        let mut service = Vec::new();
        let mut corrected = Vec::new();
        for handle in handles {
            let (s, c) = handle.await.expect("join");
            service.extend(s);
            corrected.extend(c);
        }
        let elapsed = start.elapsed().as_secs_f64();
        let pct = |values: &mut Vec<u32>, p: f64| {
            values.sort_unstable();
            values[((values.len() - 1) as f64 * p).round() as usize]
        };
        println!(
            "PACED rate={rate} completed={} elapsed_s={elapsed:.3} rps={:.1} service_p50_us={} service_p90_us={} service_p99_us={} corrected_p50_us={} corrected_p90_us={} corrected_p99_us={} corrected_p999_us={}",
            corrected.len(),
            corrected.len() as f64 / elapsed,
            pct(&mut service, 0.50),
            pct(&mut service, 0.90),
            pct(&mut service, 0.99),
            pct(&mut corrected, 0.50),
            pct(&mut corrected, 0.90),
            pct(&mut corrected, 0.99),
            pct(&mut corrected, 0.999),
        );
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);

    let connections: usize = env::var("CMP_CONNECTIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(1).max(1);
    let sessions: Vec<Session> = (0..connections).map(|_| build(ca.as_deref(), None)).collect();
    let session = sessions[0].clone();
    let mut response = session.get(url.as_str()).await.expect("warmup");
    assert_eq!(response.status().as_u16(), 200);
    assert!(
        response.bytes().await.expect("warmup body") == expected.as_ref(),
        "response body mismatch"
    );
    for extra in sessions.iter().skip(1) {
        let mut response = extra.get(url.as_str()).await.expect("connection warmup");
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
        let mut resp = session.get(url.as_str()).await.expect("warm req");
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
                let mut resp = s.get(u.as_str()).await.expect("conc req");
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
    let conc_rps = (per * conc_c) as f64 / start.elapsed().as_secs_f64();
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
    for _ in 0..cold_n {
        let mut resp = build(ca.as_deref(), None)
            .get(url.as_str())
            .await
            .expect("cold req");
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
