#[path = "../../paced.rs"]
mod paced;
#[path = "../../paced_tokio.rs"]
mod paced_tokio;

use std::env;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use hyper::{Uri, Version};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

type Http = Client<HttpsConnector<HttpConnector>, Empty<Bytes>>;

#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn tls(ca: Option<&[u8]>) -> ClientConfig {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("protocol versions");
    match ca {
        Some(ca) => {
            let mut roots = RootCertStore::empty();
            roots
                .add(CertificateDer::from(ca.to_vec()))
                .expect("CA certificate");
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        None => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(provider)))
            .with_no_client_auth(),
    }
}

fn build(ca: Option<&[u8]>) -> Http {
    let mut http = HttpConnector::new();
    http.set_nodelay(true);
    http.enforce_http(false);
    let https = HttpsConnectorBuilder::new()
        .with_tls_config(tls(ca))
        .https_only();
    if h1() {
        Client::builder(TokioExecutor::new()).build(https.enable_http1().wrap_connector(http))
    } else {
        Client::builder(TokioExecutor::new())
            .http2_only(true)
            .build(https.enable_http2().wrap_connector(http))
    }
}

fn h1() -> bool {
    match env::var("CMP_PROTO").as_deref() {
        Ok("h1") => true,
        Ok("h2") | Err(_) => false,
        Ok(other) => panic!("CMP_PROTO must be h1 or h2, got {other}"),
    }
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

fn header_map() -> &'static hyper::http::HeaderMap {
    static MAP: std::sync::LazyLock<hyper::http::HeaderMap> = std::sync::LazyLock::new(|| {
        request_headers()
            .iter()
            .map(|(name, value)| {
                (
                    hyper::http::HeaderName::from_bytes(name.as_bytes()).expect("header name"),
                    hyper::http::HeaderValue::from_str(value).expect("header value"),
                )
            })
            .collect()
    });
    &MAP
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

async fn fetch(client: &Http, uri: &Uri) -> (u16, Version, Bytes) {
    match request_timeout() {
        Some(limit) => tokio::time::timeout(limit, exchange(client, uri))
            .await
            .expect("request timed out"),
        None => exchange(client, uri).await,
    }
}

async fn exchange(client: &Http, uri: &Uri) -> (u16, Version, Bytes) {
    let mut req = hyper::Request::get(uri.clone())
        .body(Empty::new())
        .expect("request");
    *req.headers_mut() = header_map().clone();
    let resp = client.request(req).await.expect("request");
    let status = resp.status().as_u16();
    let version = resp.version();
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    (status, version, body)
}

async fn check(client: &Http, uri: &Uri, expected: &[u8], version: Version) {
    let (status, got, body) = fetch(client, uri).await;
    assert_eq!(status, 200);
    assert_eq!(got, version, "response HTTP version");
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
    let uri: Uri = url.parse().expect("url");

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let (status, _, body) = fetch(&build(ca.as_deref()), &uri).await;
        println!(
            "EQUIV hyper status={status} fnv={:016x} len={}",
            fnv1a(&body),
            body.len()
        );
        return;
    }

    if let Some(plan) = paced::Plan::from_args(&args) {
        let version = if h1() {
            Version::HTTP_11
        } else {
            Version::HTTP_2
        };
        let connections = paced::connections();
        let clients: Vec<Http> = (0..connections).map(|_| build(ca.as_deref())).collect();
        for client in &clients {
            check(client, &uri, &expected, version).await;
        }
        let clients = Arc::new(clients);
        let rows = paced_tokio::run_tasks(plan, move |worker| {
            let client = clients[worker % clients.len()].clone();
            let (uri, expected) = (uri.clone(), Arc::clone(&expected));
            async move { check(&client, &uri, &expected, version).await }
        })
        .await;
        paced::report(&plan, rows);
        return;
    }

    let warm_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let cold_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(200);
    let conc_n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let conc_c: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(64);
    let version = if h1() {
        Version::HTTP_11
    } else {
        Version::HTTP_2
    };

    let connections: usize = env::var("CMP_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
        .max(1);
    let clients: Vec<Http> = (0..connections).map(|_| build(ca.as_deref())).collect();
    for client in &clients {
        check(client, &uri, &expected, version).await;
    }
    let client = clients[0].clone();

    let mut warm_us: Vec<u32> = Vec::with_capacity(warm_n);
    let start = Instant::now();
    for _ in 0..warm_n {
        let t0 = Instant::now();
        check(&client, &uri, &expected, version).await;
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
        let c = clients[index % connections].clone();
        let u = uri.clone();
        let expected = Arc::clone(&expected);
        handles.push(tokio::spawn(async move {
            let mut lat = Vec::with_capacity(per);
            for _ in 0..per {
                let t0 = Instant::now();
                check(&c, &u, &expected, version).await;
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
        let fresh = build(ca.as_deref());
        check(&fresh, &uri, &expected, version).await;
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    println!(
        "RESULT hyper connections={connections} warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps={cold_rps:.3}{warm_lat}{conc_lat}"
    );
}
