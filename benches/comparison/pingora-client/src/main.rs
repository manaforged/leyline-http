use std::env;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use pingora_core::connectors::ConnectorOptions;
use pingora_core::connectors::http::Connector;
use pingora_core::protocols::ALPN;
use pingora_core::protocols::http::client::HttpSession;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::utils::tls::{WrappedX509, parse_x509};
use pingora_http::RequestHeader;
use rustls::crypto::aws_lc_rs;

const WINDOW_WARM: Duration = Duration::from_millis(300);

struct Target {
    peer: HttpPeer,
    request: RequestHeader,
}

fn h1() -> bool {
    match env::var("CMP_PROTO").as_deref() {
        Ok("h1") => true,
        Ok("h2") | Err(_) => false,
        Ok(other) => panic!("CMP_PROTO must be h1 or h2, got {other}"),
    }
}

fn version() -> http::Version {
    if h1() {
        http::Version::HTTP_11
    } else {
        http::Version::HTTP_2
    }
}

fn request_headers() -> Vec<(String, String)> {
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

fn target(url: &str, ca: Option<Vec<u8>>) -> Target {
    let uri: http::Uri = url.parse().expect("url");
    let host = uri.host().expect("url host");
    let port = uri.port_u16().unwrap_or(443);
    let authority = uri.authority().expect("url authority").as_str();
    let path = uri.path_and_query().map_or("/", |p| p.as_str());
    let mut peer = HttpPeer::new(
        (host.trim_matches(['[', ']']), port),
        true,
        host.trim_matches(['[', ']']).to_owned(),
    );
    peer.options.alpn = if h1() { ALPN::H1 } else { ALPN::H2 };
    peer.options.max_h2_streams = usize::MAX;
    match ca {
        Some(der) => {
            peer.options.ca = Some(Arc::from(vec![WrappedX509::new(der, |raw| {
                parse_x509(raw)
            })]));
        }
        None => {
            peer.options.verify_cert = false;
            peer.options.verify_hostname = false;
        }
    }
    let headers = request_headers();
    let mut request = RequestHeader::build_no_case("GET", path.as_bytes(), Some(headers.len() + 1))
        .expect("request");
    for (name, value) in headers {
        request
            .append_header(name, value.as_str())
            .expect("request header");
    }
    if h1() {
        request
            .insert_header(http::header::HOST, authority)
            .expect("host header");
    } else {
        request.set_uri(uri.clone());
    }
    Target { peer, request }
}

fn build(pool: usize) -> Connector {
    Connector::new(Some(ConnectorOptions::new(pool.max(1))))
}

async fn fetch(connector: &Connector, target: &Target) -> (u16, http::Version, BytesMut) {
    match request_timeout() {
        Some(limit) => tokio::time::timeout(limit, exchange(connector, target))
            .await
            .expect("request timed out"),
        None => exchange(connector, target).await,
    }
}

async fn exchange(connector: &Connector, target: &Target) -> (u16, http::Version, BytesMut) {
    let (mut session, _) = connector
        .get_http_session(&target.peer)
        .await
        .expect("http session");
    let request = Box::new(target.request.clone());
    if let HttpSession::H2(h2) = &mut session {
        h2.write_request_header(request, true)
            .expect("write request");
    } else {
        session
            .write_request_header(request)
            .await
            .expect("write request");
    }
    session
        .read_response_header()
        .await
        .expect("response header");
    let head = session.response_header().expect("response header");
    let status = head.status.as_u16();
    let got = head.version;
    let mut body = BytesMut::new();
    while let Some(chunk) = session.read_response_body().await.expect("body") {
        body.extend_from_slice(&chunk);
    }
    connector
        .release_http_session(session, &target.peer, None)
        .await;
    (status, got, body)
}

async fn check(connector: &Connector, target: &Target, expected: &[u8]) {
    let (status, got, body) = fetch(connector, target).await;
    assert_eq!(status, 200);
    assert_eq!(got, version(), "response HTTP version");
    assert!(&body[..] == expected, "response body mismatch");
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
    aws_lc_rs::default_provider()
        .install_default()
        .expect("crypto provider");
    tokio::spawn(run()).await.expect("client task");
}

async fn run() {
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
    let target = Arc::new(target(&url, ca));
    let connections: usize = env::var("CMP_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let (status, _, body) = fetch(&build(1), &target).await;
        println!(
            "EQUIV pingora status={status} fnv={:016x} len={}",
            fnv1a(&body),
            body.len()
        );
        return;
    }

    if args.get(2).map(|s| s.as_str()) == Some("window") {
        let seconds: f64 = args.get(3).and_then(|s| s.parse().ok()).expect("seconds");
        let streams: usize = args.get(4).and_then(|s| s.parse().ok()).expect("streams");
        let connectors: Vec<Arc<Connector>> = (0..connections)
            .map(|_| Arc::new(build(streams.div_ceil(connections))))
            .collect();
        for connector in &connectors {
            check(connector, &target, &expected).await;
        }
        let open = Instant::now() + WINDOW_WARM;
        let close = open + Duration::from_secs_f64(seconds);
        let handles: Vec<_> = (0..streams)
            .map(|index| {
                let connector = Arc::clone(&connectors[index % connections]);
                let target = Arc::clone(&target);
                let expected = Arc::clone(&expected);
                tokio::spawn(async move {
                    let mut counted = 0u64;
                    loop {
                        check(&connector, &target, &expected).await;
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

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);

    let connectors: Vec<Arc<Connector>> = (0..connections)
        .map(|_| Arc::new(build(conc_c.div_ceil(connections))))
        .collect();
    for connector in &connectors {
        check(connector, &target, &expected).await;
    }
    let connector = Arc::clone(&connectors[0]);

    let mut warm_us: Vec<u32> = Vec::with_capacity(warm_n);
    let start = Instant::now();
    for _ in 0..warm_n {
        let t0 = Instant::now();
        check(&connector, &target, &expected).await;
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
        let c = Arc::clone(&connectors[index % connections]);
        let target = Arc::clone(&target);
        let expected = Arc::clone(&expected);
        handles.push(tokio::spawn(async move {
            let mut lat = Vec::with_capacity(per);
            for _ in 0..per {
                let t0 = Instant::now();
                check(&c, &target, &expected).await;
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
        check(&build(1), &target, &expected).await;
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!(
        "RESULT pingora connections={connections} warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps={cold_rps:.3}{warm_lat}{conc_lat}"
    );
}
