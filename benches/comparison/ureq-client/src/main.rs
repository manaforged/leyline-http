#[path = "../../paced.rs"]
mod paced;
#[path = "../../paced_threads.rs"]
mod paced_threads;

use std::env;
use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::http::Version;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

fn build(ca: Option<&[u8]>, idle: usize) -> Agent {
    let tls = match ca {
        Some(ca) => TlsConfig::builder()
            .root_certs(RootCerts::Specific(Arc::new(vec![
                Certificate::from_der(ca).to_owned(),
            ])))
            .build(),
        None => TlsConfig::builder().disable_verification(true).build(),
    };
    Agent::new_with_config(
        Agent::config_builder()
            .tls_config(tls)
            .max_idle_connections(idle)
            .max_idle_connections_per_host(idle)
            .timeout_global(request_timeout())
            .build(),
    )
}

fn request_timeout() -> Option<Duration> {
    static LIMIT: std::sync::LazyLock<Option<Duration>> =
        std::sync::LazyLock::new(|| match env::var("CMP_TIMEOUT").as_deref() {
            Err(_) | Ok("default" | "none") => None,
            Ok(v) => Some(
                v.strip_suffix('s')
                    .and_then(|n| n.parse().ok())
                    .filter(|&n: &u64| n > 0)
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| {
                        panic!("CMP_TIMEOUT must be default, none or <N>s, got {v}")
                    }),
            ),
        });
    *LIMIT
}

fn request_headers() -> &'static [(String, String)] {
    static HEADERS: std::sync::LazyLock<Vec<(String, String)>> = std::sync::LazyLock::new(|| {
        env::var_os("CMP_HEADERS")
            .map(|path| {
                fs::read_to_string(path)
                    .expect("read request headers")
                    .lines()
                    .map(|line| {
                        let (name, value) = line.split_once('\t').expect("header name and value");
                        (name.to_owned(), value.to_owned())
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    &HEADERS
}

fn fetch(agent: &Agent, url: &str) -> (u16, Version, Vec<u8>) {
    let mut req = agent.get(url);
    for (name, value) in request_headers() {
        req = req.header(name, value);
    }
    let mut resp = req.call().expect("request");
    let status = resp.status().as_u16();
    let version = resp.version();
    let body = resp
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_to_vec()
        .expect("body");
    (status, version, body)
}

fn check(agent: &Agent, url: &str, expected: &[u8]) {
    let (status, version, body) = fetch(agent, url);
    assert_eq!(status, 200);
    assert_eq!(version, Version::HTTP_11, "response HTTP version");
    assert!(body == expected, "response body mismatch");
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn main() {
    if env::var("CMP_PROTO").as_deref() != Ok("h1") {
        eprintln!("ureq speaks only HTTP/1.1; set CMP_PROTO=h1");
        std::process::exit(1);
    }
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
    let args: Vec<String> = env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://127.0.0.1:8443/".to_string());

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let (status, _, body) = fetch(&build(ca.as_deref(), 1), &url);
        println!(
            "EQUIV ureq status={status} fnv={:016x} len={}",
            fnv1a(&body),
            body.len()
        );
        return;
    }

    if args.get(2).map(|s| s.as_str()) == Some("window") {
        let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).expect("seconds");
        let streams: usize = args.get(4).and_then(|s| s.parse().ok()).expect("streams");
        let agents: Vec<Agent> = (0..streams)
            .map(|_| build(ca.as_deref(), streams))
            .collect();
        for agent in &agents {
            check(agent, &url, &expected);
        }
        let open = Instant::now() + Duration::from_millis(300);
        let close = open + Duration::from_secs_f64(seconds);
        let handles: Vec<_> = agents
            .into_iter()
            .map(|agent| {
                let (url, expected) = (url.clone(), Arc::clone(&expected));
                thread::spawn(move || {
                    let mut counted = 0u64;
                    loop {
                        check(&agent, &url, &expected);
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
        let counts: Vec<u64> = handles
            .into_iter()
            .map(|h| h.join().expect("join"))
            .collect();
        let total: u64 = counts.iter().sum();
        println!(
            "WINDOW rps={:.1} tasks={} task_min={} task_max={}",
            total as f64 / seconds,
            counts.len(),
            counts.iter().min().copied().unwrap_or(0),
            counts.iter().max().copied().unwrap_or(0)
        );
        return;
    }

    if let Some(plan) = paced::Plan::from_args(&args) {
        let agents: Vec<Agent> = (0..paced::connections().max(plan.workers))
            .map(|_| build(ca.as_deref(), plan.workers))
            .collect();
        for agent in &agents {
            check(agent, &url, &expected);
        }
        let rows = paced_threads::run_threads(&plan, |worker| {
            check(&agents[worker % agents.len()], &url, &expected)
        });
        paced::report(&plan, rows);
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);

    let connections: usize = env::var("CMP_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(conc_c)
        .max(1);
    let agents: Vec<Agent> = (0..connections)
        .map(|_| build(ca.as_deref(), conc_c.max(1)))
        .collect();
    for agent in &agents {
        check(agent, &url, &expected);
    }
    let agent = &agents[0];

    let mut warm_us: Vec<u32> = Vec::with_capacity(warm_n);
    let start = Instant::now();
    for _ in 0..warm_n {
        let t0 = Instant::now();
        check(agent, &url, &expected);
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
        let a = agents[index % connections].clone();
        let u = url.clone();
        let expected = Arc::clone(&expected);
        handles.push(thread::spawn(move || {
            let mut lat = Vec::with_capacity(per);
            for _ in 0..per {
                let t0 = Instant::now();
                check(&a, &u, &expected);
                lat.push(t0.elapsed().as_micros() as u32);
            }
            lat
        }));
    }
    let mut conc_us = Vec::with_capacity(per * conc_c);
    for h in handles {
        conc_us.extend(h.join().expect("join"));
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
        let fresh = build(ca.as_deref(), 1);
        check(&fresh, &url, &expected);
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!(
        "RESULT ureq connections={connections} warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps={cold_rps:.3}{warm_lat}{conc_lat}"
    );
}
