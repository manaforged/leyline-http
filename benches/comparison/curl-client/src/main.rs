use std::env;
use std::fs;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use curl::easy::{Easy2, Handler, HttpVersion, WriteError};
use curl::multi::{Easy2Handle, Multi};

struct Collector(Vec<u8>);

impl Handler for Collector {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
}

struct Setup {
    url: String,
    ca_file: Option<std::path::PathBuf>,
    expected: Arc<[u8]>,
    h1: bool,
}

fn pem(der: &[u8]) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b64 = Vec::with_capacity(der.len().div_ceil(3) * 4);
    for chunk in der.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        b64.push(TABLE[(n >> 18) as usize & 63]);
        b64.push(TABLE[(n >> 12) as usize & 63]);
        b64.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63]
        } else {
            b'='
        });
        b64.push(if chunk.len() > 2 {
            TABLE[n as usize & 63]
        } else {
            b'='
        });
    }
    let mut out = b"-----BEGIN CERTIFICATE-----\n".to_vec();
    for line in b64.chunks(64) {
        out.extend_from_slice(line);
        out.push(b'\n');
    }
    out.extend_from_slice(b"-----END CERTIFICATE-----\n");
    out
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

fn easy(setup: &Setup) -> Easy2<Collector> {
    let mut e = Easy2::new(Collector(Vec::new()));
    e.url(&setup.url).expect("url");
    if setup.h1 {
        e.http_version(HttpVersion::V11).expect("http version");
    } else {
        e.http_version(HttpVersion::V2TLS).expect("http version");
        e.pipewait(true).expect("pipewait");
    }
    if let Some(limit) = request_timeout() {
        e.timeout(limit).expect("timeout");
    }
    match &setup.ca_file {
        Some(ca) => e.cainfo(ca).expect("CA certificate"),
        None => {
            e.ssl_verify_peer(false).expect("verify peer");
            e.ssl_verify_host(false).expect("verify host");
        }
    }
    e
}

fn check(e: &mut Easy2<Collector>, setup: &Setup) {
    assert_eq!(e.response_code().expect("status"), 200);
    let expected = &setup.expected[..];
    assert!(e.get_ref().0 == expected, "response body mismatch");
    e.get_mut().0.clear();
}

fn multi(h1: bool) -> Multi {
    let mut m = Multi::new();
    m.pipelining(false, !h1).expect("multiplex");
    m.set_max_host_connections(if h1 { 0 } else { 1 })
        .expect("max host connections");
    m
}

fn drive(m: &Multi, setup: &Setup, workers: usize, per: usize) -> Vec<u32> {
    let mut lat = Vec::with_capacity(workers * per);
    if per == 0 {
        return lat;
    }
    let mut slots: Vec<Option<(Easy2Handle<Collector>, Instant, usize)>> =
        Vec::with_capacity(workers);
    for token in 0..workers {
        let mut h = m.add2(easy(setup)).expect("add transfer");
        h.set_token(token).expect("token");
        slots.push(Some((h, Instant::now(), per)));
    }
    let mut active = workers;
    let mut done = Vec::new();
    while active > 0 {
        m.perform().expect("multi perform");
        m.messages(|msg| {
            if let Some(result) = msg.result() {
                done.push((msg.token().expect("token"), result));
            }
        });
        for (token, result) in done.drain(..) {
            result.expect("transfer");
            let (h, t0, left) = slots[token].take().expect("slot");
            let mut e = m.remove2(h).expect("remove transfer");
            check(&mut e, setup);
            lat.push(t0.elapsed().as_micros() as u32);
            if left > 1 {
                let mut h = m.add2(e).expect("re-add transfer");
                h.set_token(token).expect("token");
                slots[token] = Some((h, Instant::now(), left - 1));
            } else {
                active -= 1;
            }
        }
        if active > 0 {
            m.wait(&mut [], Duration::from_millis(100))
                .expect("multi wait");
        }
    }
    lat
}

fn connection(setup: Arc<Setup>, jobs: Receiver<(usize, usize)>, results: Sender<Vec<u32>>) {
    let m = multi(setup.h1);
    for (workers, per) in jobs {
        results
            .send(drive(&m, &setup, workers, per))
            .expect("send results");
    }
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
    let ca_file = env::var_os("CMP_CA").map(|path| {
        let file = env::temp_dir().join(format!("curl-cmp-ca-{}.pem", std::process::id()));
        fs::write(&file, pem(&fs::read(path).expect("read CA certificate"))).expect("write CA PEM");
        file
    });
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
    let setup = Arc::new(Setup {
        url,
        ca_file,
        expected,
        h1: match env::var("CMP_PROTO").as_deref() {
            Ok("h1") => true,
            Ok("h2") | Err(_) => false,
            Ok(other) => panic!("CMP_PROTO must be h1 or h2, got {other}"),
        },
    });

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let e = easy(&setup);
        e.perform().expect("equiv req");
        let status = e.response_code().expect("status");
        let body = &e.get_ref().0;
        println!(
            "EQUIV curl status={status} fnv={:016x} len={}",
            fnv1a(body),
            body.len()
        );
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
        .max(1);
    let mut lanes = Vec::with_capacity(connections);
    for _ in 0..connections {
        let (job_tx, job_rx) = channel();
        let (res_tx, res_rx) = channel();
        let setup = Arc::clone(&setup);
        let thread = thread::spawn(move || connection(setup, job_rx, res_tx));
        lanes.push((job_tx, res_rx, thread));
    }
    for (jobs, results, _) in &lanes {
        jobs.send((1, 1)).expect("warmup");
        results.recv().expect("warmup result");
    }

    let start = Instant::now();
    lanes[0].0.send((1, warm_n)).expect("warm");
    let mut warm_us = lanes[0].1.recv().expect("warm result");
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
    for (k, (jobs, _, _)) in lanes.iter().enumerate() {
        let workers = (0..conc_c).filter(|i| i % connections == k).count();
        jobs.send((workers, per)).expect("conc");
    }
    let mut conc_us = Vec::with_capacity(per * conc_c);
    for (_, results, _) in &lanes {
        conc_us.extend(results.recv().expect("conc result"));
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

    for (jobs, _, thread) in lanes {
        drop(jobs);
        thread.join().expect("connection thread");
    }

    let start = Instant::now();
    for _ in 0..cold_n {
        let mut e = easy(&setup);
        e.fresh_connect(true).expect("fresh connect");
        e.forbid_reuse(true).expect("forbid reuse");
        e.perform().expect("cold req");
        check(&mut e, &setup);
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();
    if let Some(file) = &setup.ca_file {
        drop(fs::remove_file(file));
    }

    println!(
        "RESULT curl connections={connections} warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps={cold_rps:.3}{warm_lat}{conc_lat}"
    );
}
