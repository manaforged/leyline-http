use std::env;
use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use leyline::blocking::Session;
use leyline_cmp_client::{
    WINDOW_WARM, expected_body, fnv1a, session_builder, version, window_line,
};

fn build(ca: Option<&[u8]>) -> Session {
    Session::new(session_builder(ca)).expect("leyline blocking session builds")
}

fn fetch(session: &Session, url: &str) -> (u16, leyline::http::Version, impl AsRef<[u8]>) {
    session
        .block_on(async {
            let response = session.get(url).send().await?;
            let (status, version) = (response.status().as_u16(), response.version());
            Ok::<_, leyline::Error>((status, version, response.bytes().await?))
        })
        .expect("request")
}

fn check(session: &Session, url: &str, expected: &[u8]) {
    let (status, got, body) = fetch(session, url);
    assert_eq!(status, 200);
    assert_eq!(got, version(), "response HTTP version");
    assert!(body.as_ref() == expected, "response body mismatch");
}

fn main() {
    let ca = env::var_os("CMP_CA").map(|path| fs::read(path).expect("read CA certificate"));
    let expected = expected_body();
    let args: Vec<String> = env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://127.0.0.1:8443/".to_string());

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let session = build(ca.as_deref());
        let (status, _, body) = fetch(&session, &url);
        println!(
            "EQUIV leyline-blocking status={status} fnv={:016x} len={}",
            fnv1a(body.as_ref()),
            body.as_ref().len()
        );
        return;
    }

    if args.get(2).map(|s| s.as_str()) == Some("window") {
        let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).expect("seconds");
        let streams: usize = args.get(4).and_then(|s| s.parse().ok()).expect("streams");
        let ready = Arc::new(std::sync::Barrier::new(streams + 1));
        let opened = Arc::new(std::sync::OnceLock::<Instant>::new());
        let handles: Vec<_> = (0..streams)
            .map(|_| {
                let (ca, url, expected, ready, opened) = (
                    ca.clone(),
                    url.clone(),
                    Arc::clone(&expected),
                    Arc::clone(&ready),
                    Arc::clone(&opened),
                );
                thread::spawn(move || {
                    let session = build(ca.as_deref());
                    check(&session, &url, &expected);
                    ready.wait();
                    let open = *opened.wait();
                    let close = open + std::time::Duration::from_secs_f64(seconds);
                    let mut counted = 0u64;
                    loop {
                        check(&session, &url, &expected);
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
        ready.wait();
        opened.set(Instant::now() + WINDOW_WARM).expect("set once");
        let counts: Vec<u64> = handles
            .into_iter()
            .map(|h| h.join().expect("join"))
            .collect();
        println!("{}", window_line(&counts, seconds));
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args
        .get(5)
        .and_then(|s| s.parse().ok())
        .unwrap_or(64)
        .max(1);

    let session = build(ca.as_deref());
    check(&session, &url, &expected);
    let mut warm_us: Vec<u32> = Vec::with_capacity(warm_n);
    let start = Instant::now();
    for _ in 0..warm_n {
        let t0 = Instant::now();
        check(&session, &url, &expected);
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

    drop(session);

    let per = conc_n / conc_c;
    let ready = Arc::new(std::sync::Barrier::new(conc_c + 1));
    let handles: Vec<_> = (0..conc_c)
        .map(|_| {
            let (ca, url, expected, ready) = (
                ca.clone(),
                url.clone(),
                Arc::clone(&expected),
                Arc::clone(&ready),
            );
            thread::spawn(move || {
                let session = build(ca.as_deref());
                check(&session, &url, &expected);
                ready.wait();
                let mut lat = Vec::with_capacity(per);
                for _ in 0..per {
                    let t0 = Instant::now();
                    check(&session, &url, &expected);
                    lat.push(t0.elapsed().as_micros() as u32);
                }
                lat
            })
        })
        .collect();
    ready.wait();
    let start = Instant::now();
    let mut conc_us = Vec::with_capacity(per * conc_c);
    for handle in handles {
        conc_us.extend(handle.join().expect("join"));
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
        check(&build(ca.as_deref()), &url, &expected);
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!(
        "RESULT leyline-blocking connections={conc_c} warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps={cold_rps:.3}{warm_lat}{conc_lat}"
    );
}
