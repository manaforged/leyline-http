#[path = "../../paced.rs"]
mod paced;
#[path = "../../paced_tokio.rs"]
mod paced_tokio;

use std::env;
use std::fs;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use h2::client::SendRequest;
use http::{Request, Uri};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, aws_lc_rs};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;

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

struct Conn {
    send: SendRequest<Bytes>,
    task: JoinHandle<()>,
}

async fn connect(tls: &TlsConnector, uri: &Uri) -> Conn {
    let host = uri.host().expect("url host").trim_matches(['[', ']']);
    let port = uri.port_u16().unwrap_or(443);
    let tcp = TcpStream::connect((host, port)).await.expect("tcp connect");
    tcp.set_nodelay(true).expect("nodelay");
    let name = ServerName::try_from(host.to_string()).expect("server name");
    let stream = tls.connect(name, tcp).await.expect("tls handshake");
    assert_eq!(
        stream.get_ref().1.alpn_protocol(),
        Some(&b"h2"[..]),
        "ALPN did not select h2"
    );
    let (send, connection) = h2::client::handshake(stream).await.expect("h2 handshake");
    let task = tokio::spawn(async move {
        connection.await.expect("h2 connection");
    });
    Conn { send, task }
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

fn header_map() -> &'static http::HeaderMap {
    static MAP: std::sync::LazyLock<http::HeaderMap> = std::sync::LazyLock::new(|| {
        request_headers()
            .iter()
            .map(|(name, value)| {
                (
                    http::HeaderName::from_bytes(name.as_bytes()).expect("header name"),
                    http::HeaderValue::from_str(value).expect("header value"),
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

async fn fetch(send: SendRequest<Bytes>, uri: &Uri) -> (SendRequest<Bytes>, u16, Vec<u8>) {
    match request_timeout() {
        Some(limit) => tokio::time::timeout(limit, exchange(send, uri))
            .await
            .expect("request timed out"),
        None => exchange(send, uri).await,
    }
}

async fn exchange(send: SendRequest<Bytes>, uri: &Uri) -> (SendRequest<Bytes>, u16, Vec<u8>) {
    let mut send = send.ready().await.expect("ready");
    let mut req = Request::get(uri.clone()).body(()).expect("request");
    *req.headers_mut() = header_map().clone();
    let (resp, _) = send.send_request(req, true).expect("send request");
    let resp = resp.await.expect("response");
    let status = resp.status().as_u16();
    let mut body = resp.into_body();
    let mut buf = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.expect("body");
        body.flow_control()
            .release_capacity(chunk.len())
            .expect("release capacity");
        buf.extend_from_slice(&chunk);
    }
    (send, status, buf)
}

async fn check(send: SendRequest<Bytes>, uri: &Uri, expected: &[u8]) -> SendRequest<Bytes> {
    let (send, status, body) = fetch(send, uri).await;
    assert_eq!(status, 200);
    assert!(&body[..] == expected, "response body mismatch");
    send
}

async fn close(conn: Conn) {
    drop(conn.send);
    conn.task.await.expect("connection task");
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
    let mut config = tls(ca.as_deref());
    config.alpn_protocols = vec![b"h2".to_vec()];
    let tls = TlsConnector::from(Arc::new(config));

    if args.get(2).map(|s| s.as_str()) == Some("equiv") {
        let conn = connect(&tls, &uri).await;
        let (send, status, body) = fetch(conn.send.clone(), &uri).await;
        drop(send);
        close(conn).await;
        println!(
            "EQUIV h2 status={status} fnv={:016x} len={}",
            fnv1a(&body),
            body.len()
        );
        return;
    }

    if let Some(plan) = paced::Plan::from_args(&args) {
        let mut sends = Vec::new();
        for _ in 0..paced::connections() {
            let conn = connect(&tls, &uri).await;
            sends.push(check(conn.send.clone(), &uri, &expected).await);
        }
        let sends = Arc::new(sends);
        let rows = paced_tokio::run_tasks(plan, move |worker| {
            let send = sends[worker % sends.len()].clone();
            let (uri, expected) = (uri.clone(), Arc::clone(&expected));
            async move {
                check(send, &uri, &expected).await;
            }
        })
        .await;
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
        .max(1);
    let mut conns = Vec::with_capacity(connections);
    for _ in 0..connections {
        let conn = connect(&tls, &uri).await;
        check(conn.send.clone(), &uri, &expected).await;
        conns.push(conn);
    }

    let mut warm_us: Vec<u32> = Vec::with_capacity(warm_n);
    let mut send = conns[0].send.clone();
    let start = Instant::now();
    for _ in 0..warm_n {
        let t0 = Instant::now();
        send = check(send, &uri, &expected).await;
        warm_us.push(t0.elapsed().as_micros() as u32);
    }
    drop(send);
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
        let mut send = conns[index % connections].send.clone();
        let u = uri.clone();
        let expected = Arc::clone(&expected);
        handles.push(tokio::spawn(async move {
            let mut lat = Vec::with_capacity(per);
            for _ in 0..per {
                let t0 = Instant::now();
                send = check(send, &u, &expected).await;
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
        let conn = connect(&tls, &uri).await;
        drop(check(conn.send.clone(), &uri, &expected).await);
        close(conn).await;
    }
    let cold_rps = cold_n as f64 / start.elapsed().as_secs_f64();

    for conn in conns {
        close(conn).await;
    }
    println!(
        "RESULT h2 connections={connections} warm_rps={warm_rps:.3} conc_rps={conc_rps:.3} cold_rps={cold_rps:.3}{warm_lat}{conc_lat}"
    );
}
