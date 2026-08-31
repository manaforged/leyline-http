//! leyline Chrome-150 comparison client: HTTP/2 over TLS, cert verification off.
//!
//! Args: <url> [warm_n|print] [cold_n] [conc_n] [conc_c]. Phases:
//!   warm — warm_n sequential GETs on one reused, pooled session (latency).
//!   conc — conc_n GETs with conc_c in flight over the one multiplexed H2
//!          connection (throughput — the metric that matters for H2).
//!   cold — cold_n GETs each on a fresh session (handshake cost).
//! `print` mode does one GET and prints the body (JA4 capture off the clock).
#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::time::Instant;

use leyline::Session;

fn build() -> Session {
    let mut b = Session::builder()
        .chrome()
        .http2()
        .danger_accept_invalid_certs(true);
    // Optional egress proxy (`PROXY=http://user:pass@host:port` or socks5://…)
    // so the paired comparison can run through a real proxy against a remote
    // target, not just the loopback server.
    if let Ok(p) = std::env::var("PROXY") {
        if !p.is_empty() {
            b = b.proxy(p);
        }
    }
    b.build().expect("leyline session builds")
}

/// 64-bit FNV-1a — a dependency-free body fingerprint for the equivalence
/// gate. The paired harness asserts leyline and wreq see the same status +
/// body hash for the same URL, so the rps numbers compare equal work.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://127.0.0.1:8443/".to_string());

    if args.get(2).map(|s| s.as_str()) == Some("print") {
        let resp = build().get(url.as_str()).await.expect("print req");
        println!("{}", resp.text().unwrap());
        return;
    }

    // Equivalence gate: one GET, emit status + body fingerprint so the
    // orchestrator can assert leyline and wreq fetched identical bytes.
    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let resp = build().get(url.as_str()).await.expect("equiv req");
        let status = resp.status();
        let body = resp.text().unwrap();
        println!(
            "EQUIV leyline status={status} fnv={:016x} len={}",
            fnv1a(body.as_bytes()),
            body.len()
        );
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);

    let session = build();
    session.get(url.as_str()).await.expect("warmup");

    // warm: sequential on the reused session.
    let mut warm_us: Vec<u32> = Vec::with_capacity(warm_n);
    let start = Instant::now();
    for _ in 0..warm_n {
        let t0 = Instant::now();
        let resp = session.get(url.as_str()).await.expect("warm req");
        let _ = resp.text().unwrap();
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

    // conc: conc_c workers share the one multiplexed connection.
    let per = conc_n / conc_c;
    let start = Instant::now();
    let mut handles = Vec::with_capacity(conc_c);
    for _ in 0..conc_c {
        let s = session.clone();
        let u = url.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..per {
                let resp = s.get(u.as_str()).await.expect("conc req");
                let _ = resp.text().unwrap();
            }
        }));
    }
    for h in handles {
        h.await.expect("join");
    }
    let conc_rps = (per * conc_c) as f64 / start.elapsed().as_secs_f64();

    // cold: fresh session (new TLS handshake) per request.
    let start = Instant::now();
    for _ in 0..cold_n {
        let resp = build().get(url.as_str()).await.expect("cold req");
        let _ = resp.text().unwrap();
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!("RESULT leyline warm_rps={warm_rps:.0} conc_rps={conc_rps:.0} cold_rps={cold_rps:.0}{warm_lat}");
}
