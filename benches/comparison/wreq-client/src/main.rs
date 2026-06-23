//! wreq Chrome-137 comparison client: HTTP/2 over TLS, cert verification off.
//!
//! Args: <url> [warm_n|print] [cold_n] [conc_n] [conc_c]. See leyline-client for
//! the phase definitions. `print` mode does one GET and prints the body.
use std::time::Instant;

use wreq::Client;
use wreq_util::Emulation;

fn build() -> Client {
    let mut b = Client::builder()
        .emulation(Emulation::Chrome137)
        .cert_verification(false);
    // Optional egress proxy, mirroring leyline-client (`PROXY=http://…`).
    if let Ok(p) = std::env::var("PROXY") {
        if !p.is_empty() {
            b = b.proxy(wreq::Proxy::all(p).expect("proxy url"));
        }
    }
    b.build().expect("wreq client builds")
}

/// 64-bit FNV-1a body fingerprint — identical to leyline-client so the
/// equivalence gate compares like for like.
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
        let resp = build().get(url.as_str()).send().await.expect("print req");
        println!("{}", resp.text().await.expect("body"));
        return;
    }

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let resp = build().get(url.as_str()).send().await.expect("equiv req");
        let status = resp.status().as_u16();
        let body = resp.text().await.expect("body");
        println!(
            "EQUIV wreq status={status} fnv={:016x} len={}",
            fnv1a(body.as_bytes()),
            body.len()
        );
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);

    let client = build();
    let _ = client
        .get(url.as_str())
        .send()
        .await
        .expect("warmup")
        .bytes()
        .await;

    let start = Instant::now();
    for _ in 0..warm_n {
        let resp = client.get(url.as_str()).send().await.expect("warm req");
        let _ = resp.bytes().await.expect("body");
    }
    let warm_rps = warm_n as f64 / start.elapsed().as_secs_f64();

    let per = conc_n / conc_c;
    let start = Instant::now();
    let mut handles = Vec::with_capacity(conc_c);
    for _ in 0..conc_c {
        let c = client.clone();
        let u = url.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..per {
                let resp = c.get(u.as_str()).send().await.expect("conc req");
                let _ = resp.bytes().await.expect("body");
            }
        }));
    }
    for h in handles {
        h.await.expect("join");
    }
    let conc_rps = (per * conc_c) as f64 / start.elapsed().as_secs_f64();

    let start = Instant::now();
    for _ in 0..cold_n {
        let resp = build().get(url.as_str()).send().await.expect("cold req");
        let _ = resp.bytes().await.expect("body");
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!("RESULT wreq warm_rps={warm_rps:.0} conc_rps={conc_rps:.0} cold_rps={cold_rps:.0}");
}
