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
    if let Ok(p) = std::env::var("PROXY") {
        if !p.is_empty() {
            b = b.proxy(p);
        }
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

    let start = Instant::now();
    for _ in 0..cold_n {
        let resp = build().get(url.as_str()).await.expect("cold req");
        let _ = resp.text().unwrap();
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!("RESULT leyline warm_rps={warm_rps:.0} conc_rps={conc_rps:.0} cold_rps={cold_rps:.0}{warm_lat}");
}
