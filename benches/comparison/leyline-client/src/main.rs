//! leyline Chrome-150 comparison client: HTTP/2 over TLS, cert verification off.
//!
//! Args: <url> [warm_n|print] [cold_n] [conc_n] [conc_c]. Phases:
//!   warm — warm_n sequential GETs on one reused, pooled session (latency).
//!   conc — conc_n GETs with conc_c in flight over the one multiplexed H2
//!          connection (throughput — the metric that matters for H2).
//!   cold — cold_n GETs each on a fresh session (handshake cost).
//! `print` mode does one GET and prints the body (JA4 capture off the clock).
use std::time::Instant;

use leyline::{Browser, Session};

fn build() -> Session {
    Session::builder()
        .browser(Browser::Chrome150)
        .http2()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("leyline session builds")
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let url = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "https://127.0.0.1:8443/".to_string());

    if args.get(2).map(|s| s.as_str()) == Some("print") {
        let resp = build().get(url.as_str()).send().await.expect("print req");
        println!("{}", resp.text());
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);

    let session = build();
    session.get(url.as_str()).send().await.expect("warmup");

    // warm: sequential on the reused session.
    let start = Instant::now();
    for _ in 0..warm_n {
        let resp = session.get(url.as_str()).send().await.expect("warm req");
        let _ = resp.text();
    }
    let warm_rps = warm_n as f64 / start.elapsed().as_secs_f64();

    // conc: conc_c workers share the one multiplexed connection.
    let per = conc_n / conc_c;
    let start = Instant::now();
    let mut handles = Vec::with_capacity(conc_c);
    for _ in 0..conc_c {
        let s = session.clone();
        let u = url.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..per {
                let resp = s.get(u.as_str()).send().await.expect("conc req");
                let _ = resp.text();
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
        let resp = build().get(url.as_str()).send().await.expect("cold req");
        let _ = resp.text();
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!("RESULT leyline warm_rps={warm_rps:.0} conc_rps={conc_rps:.0} cold_rps={cold_rps:.0}");
}
