use std::env;
use std::fs;
use std::sync::Arc;
use std::time::Instant;

use reqwest::{Certificate, Client};

fn build(ca: Option<&[u8]>) -> Client {
    let mut b = Client::builder().http2_prior_knowledge();
    if let Some(ca) = ca {
        b = b.tls_certs_only([Certificate::from_der(ca).expect("CA certificate")]);
    } else {
        b = b.danger_accept_invalid_certs(true);
    }
    if let Ok(p) = std::env::var("PROXY")
        && !p.is_empty()
    {
        b = b.proxy(reqwest::Proxy::all(p).expect("proxy url"));
    }
    b.build().expect("reqwest client builds")
}

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

    if args.get(2).map(|s| s.as_str()) == Some("print") {
        let resp = build(ca.as_deref())
            .get(url.as_str())
            .send()
            .await
            .expect("print req");
        println!("{}", resp.text().await.expect("body"));
        return;
    }

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let resp = build(ca.as_deref())
            .get(url.as_str())
            .send()
            .await
            .expect("equiv req");
        let status = resp.status().as_u16();
        let body = resp.bytes().await.expect("body");
        println!(
            "EQUIV reqwest status={status} fnv={:016x} len={}",
            fnv1a(&body),
            body.len()
        );
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);

    let client = build(ca.as_deref());
    let response = client.get(url.as_str()).send().await.expect("warmup");
    assert_eq!(response.status().as_u16(), 200);
    assert!(
        &response.bytes().await.expect("warmup body")[..] == expected.as_ref(),
        "response body mismatch"
    );

    let mut warm_us: Vec<u32> = Vec::with_capacity(warm_n);
    let start = Instant::now();
    for _ in 0..warm_n {
        let t0 = Instant::now();
        let resp = client.get(url.as_str()).send().await.expect("warm req");
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            &resp.bytes().await.expect("body")[..] == expected.as_ref(),
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
    for _ in 0..conc_c {
        let c = client.clone();
        let u = url.clone();
        let expected = Arc::clone(&expected);
        handles.push(tokio::spawn(async move {
            for _ in 0..per {
                let resp = c.get(u.as_str()).send().await.expect("conc req");
                assert_eq!(resp.status().as_u16(), 200);
                assert!(
                    &resp.bytes().await.expect("body")[..] == expected.as_ref(),
                    "response body mismatch"
                );
            }
        }));
    }
    for h in handles {
        h.await.expect("join");
    }
    let conc_rps = (per * conc_c) as f64 / start.elapsed().as_secs_f64();

    let start = Instant::now();
    for _ in 0..cold_n {
        let resp = build(ca.as_deref())
            .get(url.as_str())
            .send()
            .await
            .expect("cold req");
        assert_eq!(resp.status().as_u16(), 200);
        assert!(
            &resp.bytes().await.expect("body")[..] == expected.as_ref(),
            "response body mismatch"
        );
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!(
        "RESULT reqwest warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps={cold_rps:.3}{warm_lat}"
    );
}
