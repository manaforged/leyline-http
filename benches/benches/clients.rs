//! TLS head-to-head: Leyline vs `reqwest` 0.13 (rustls) against one in-process
//! origin.
//!
//! The bench owns the server. It generates an `rcgen` self-signed leaf
//! (SAN `localhost` + `127.0.0.1`), then stands up two loopback TLS listeners
//! backed by hyper 1.x over `tokio-rustls`:
//!
//!   * an ALPN `http/1.1` listener (`http1::Builder`, keep-alive on), and
//!   * an ALPN `h2` listener (`http2::Builder`).
//!
//! ALPN on the server side is what pins each scenario to one protocol, so no
//! client-side version override is needed. Both clients trust the leaf through
//! their own API: `danger_accept_invalid_certs(true)`. That is a deliberate
//! bench shortcut — it removes trust-store work from both stacks equally and
//! keeps the delta on the client HTTP path.
//!
//! Before any timing, `verify` fetches every scenario once per client and
//! asserts the response body is byte-equal to the server's fixture (and that
//! the negotiated version is the one the listener advertised). A mismatch
//! panics and the bench fails.
//!
//! Scenarios:
//!   * `clients::{leyline,reqwest}_h1_16k` — 200 sequential keep-alive GETs of
//!     a 16 KiB body on one reused client.
//!   * `clients::{leyline,reqwest}_h2_32x16k` — 32 concurrent GETs of the same
//!     16 KiB body, multiplexed over one H2 connection.
//!   * `clients::{leyline,reqwest}_stream_4mib` — one 4 MiB body consumed
//!     chunk by chunk through each client's streaming API.
//!
//! `wreq` is not here: `btls-sys` (wreq's BoringSSL) and `leyline-bssl-sys`
//! both declare `links = "boringssl"`, and Cargo refuses to put two such
//! packages in one dependency graph. The out-of-process wreq comparison lives
//! in `benches/comparison/`.

use std::convert::Infallible;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use futures_util::StreamExt;
use futures_util::future::join_all;
use http_body_util::Full;
use hyper::server::conn::{http1, http2};
use hyper::service::service_fn;
use hyper::{Request as Inbound, Response as Outbound};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::ServerConfig;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio_rustls::TlsAcceptor;

use leyline::{HttpVersion, Session};

/// Body served on `/small`.
const SMALL: usize = 16 * 1024;

/// Body served on `/big`.
const BIG: usize = 4 * 1024 * 1024;

/// Sequential GETs per H1 sample.
const SERIAL: usize = 200;

/// In-flight GETs per H2 sample.
const CONC: usize = 32;

/// Deterministic fixture bytes; the equivalence assertions compare against these.
fn fixture(len: usize) -> Bytes {
    Bytes::from((0..len).map(|i| (i % 251) as u8).collect::<Vec<u8>>())
}

static SMALL_BODY: LazyLock<Bytes> = LazyLock::new(|| fixture(SMALL));
static BIG_BODY: LazyLock<Bytes> = LazyLock::new(|| fixture(BIG));

/// Serve the 4 MiB fixture on `/big`, the 16 KiB fixture everywhere else.
async fn respond(req: Inbound<hyper::body::Incoming>) -> Result<Outbound<Full<Bytes>>, Infallible> {
    let body = if req.uri().path() == "/big" {
        BIG_BODY.clone()
    } else {
        SMALL_BODY.clone()
    };
    Ok(Outbound::new(Full::new(body)))
}

/// Self-signed acceptor advertising exactly one ALPN protocol.
fn tls(alpn: &[u8]) -> TlsAcceptor {
    let leaf =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("self-signed leaf");
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf.signing_key.serialize_der()));
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![leaf.cert.der().clone()], key)
        .expect("server config");
    config.alpn_protocols = vec![alpn.to_vec()];
    TlsAcceptor::from(Arc::new(config))
}

/// Bind a loopback TLS origin, spawn it, and return its base URL.
async fn origin(h2: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let acceptor = tls(if h2 { b"h2" } else { b"http/1.1" });
    drop(tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                return;
            };
            drop(sock.set_nodelay(true));
            let acceptor = acceptor.clone();
            drop(tokio::spawn(async move {
                let Ok(stream) = acceptor.accept(sock).await else {
                    return;
                };
                let io = TokioIo::new(stream);
                if h2 {
                    drop(
                        http2::Builder::new(TokioExecutor::new())
                            .serve_connection(io, service_fn(respond))
                            .await,
                    );
                } else {
                    drop(
                        http1::Builder::new()
                            .keep_alive(true)
                            .serve_connection(io, service_fn(respond))
                            .await,
                    );
                }
            }));
        }
    }));
    format!("https://127.0.0.1:{}/", addr.port())
}

/// One buffered GET through Leyline, body returned for comparison.
async fn ley(session: &Session, url: &str) -> (Bytes, HttpVersion) {
    let mut resp = session.get(url).await.expect("leyline request");
    let version = resp.version();
    let body = Bytes::copy_from_slice(resp.bytes().await.expect("leyline body"));
    (body, version)
}

/// One streamed GET through Leyline, chunks concatenated.
async fn ley_stream(session: &Session, url: &str) -> Bytes {
    let resp = session.get(url).stream().await.expect("leyline request");
    let mut stream = resp.into_stream().expect("leyline stream");
    let mut out = Vec::with_capacity(BIG);
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.expect("leyline chunk"));
    }
    Bytes::from(out)
}

/// One buffered GET through reqwest, body returned for comparison.
async fn req(client: &reqwest13::Client, url: &str) -> (Bytes, reqwest13::Version) {
    let resp = client.get(url).send().await.expect("reqwest request");
    let version = resp.version();
    let body = resp.bytes().await.expect("reqwest body");
    (body, version)
}

/// One streamed GET through reqwest, chunks concatenated.
async fn req_stream(client: &reqwest13::Client, url: &str) -> Bytes {
    let resp = client.get(url).send().await.expect("reqwest request");
    let mut stream = resp.bytes_stream();
    let mut out = Vec::with_capacity(BIG);
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.expect("reqwest chunk"));
    }
    Bytes::from(out)
}

/// Assert every client returns the server fixture byte for byte, on the
/// protocol the listener advertised. Runs before any measurement.
async fn verify(session: &Session, client: &reqwest13::Client, h1: &str, h2: &str, big: &str) {
    let (body, version) = ley(session, h1).await;
    assert_eq!(body, *SMALL_BODY, "leyline h1 16k body mismatch");
    assert_eq!(version, HttpVersion::Http1_1, "leyline h1 version");

    let (body, version) = req(client, h1).await;
    assert_eq!(body, *SMALL_BODY, "reqwest h1 16k body mismatch");
    assert_eq!(version, reqwest13::Version::HTTP_11, "reqwest h1 version");

    let (body, version) = ley(session, h2).await;
    assert_eq!(body, *SMALL_BODY, "leyline h2 16k body mismatch");
    assert_eq!(version, HttpVersion::Http2, "leyline h2 version");

    let (body, version) = req(client, h2).await;
    assert_eq!(body, *SMALL_BODY, "reqwest h2 16k body mismatch");
    assert_eq!(version, reqwest13::Version::HTTP_2, "reqwest h2 version");

    assert_eq!(
        ley_stream(session, big).await,
        *BIG_BODY,
        "leyline 4 MiB stream body mismatch"
    );
    assert_eq!(
        req_stream(client, big).await,
        *BIG_BODY,
        "reqwest 4 MiB stream body mismatch"
    );
}

fn bench(c: &mut Criterion) {
    drop(rustls::crypto::aws_lc_rs::default_provider().install_default());
    let rt = Runtime::new().expect("tokio runtime");
    let (h1, h2) = rt.block_on(async { (origin(false).await, origin(true).await) });
    let small1 = format!("{h1}small");
    let small2 = format!("{h2}small");
    let big = format!("{h1}big");

    let session = Session::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("leyline session");
    let client = reqwest13::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("reqwest client");

    rt.block_on(verify(&session, &client, &small1, &small2, &big));

    let mut g = c.benchmark_group("clients");
    g.throughput(Throughput::Elements(SERIAL as u64));
    g.sample_size(20);
    g.measurement_time(Duration::from_secs(10));
    g.bench_function("leyline_h1_16k", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    for _ in 0..SERIAL {
                        black_box(ley(&session, &small1).await);
                    }
                    total += start.elapsed();
                }
                total
            })
        })
    });
    g.bench_function("reqwest_h1_16k", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    for _ in 0..SERIAL {
                        black_box(req(&client, &small1).await);
                    }
                    total += start.elapsed();
                }
                total
            })
        })
    });
    g.finish();

    let mut g = c.benchmark_group("clients");
    g.throughput(Throughput::Elements(CONC as u64));
    g.sample_size(30);
    g.measurement_time(Duration::from_secs(10));
    g.bench_function("leyline_h2_32x16k", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let mut tasks = Vec::with_capacity(CONC);
                    for _ in 0..CONC {
                        let s = session.clone();
                        let u = small2.clone();
                        tasks.push(tokio::spawn(async move { ley(&s, &u).await }));
                    }
                    let done = join_all(tasks).await;
                    total += start.elapsed();
                    for r in done {
                        black_box(r.expect("join"));
                    }
                }
                total
            })
        })
    });
    g.bench_function("reqwest_h2_32x16k", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let mut tasks = Vec::with_capacity(CONC);
                    for _ in 0..CONC {
                        let c = client.clone();
                        let u = small2.clone();
                        tasks.push(tokio::spawn(async move { req(&c, &u).await }));
                    }
                    let done = join_all(tasks).await;
                    total += start.elapsed();
                    for r in done {
                        black_box(r.expect("join"));
                    }
                }
                total
            })
        })
    });
    g.finish();

    let mut g = c.benchmark_group("clients");
    g.throughput(Throughput::Bytes(BIG as u64));
    g.sample_size(20);
    g.measurement_time(Duration::from_secs(10));
    g.bench_function("leyline_stream_4mib", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let body = ley_stream(&session, &big).await;
                    total += start.elapsed();
                    black_box(body);
                }
                total
            })
        })
    });
    g.bench_function("reqwest_stream_4mib", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    let body = req_stream(&client, &big).await;
                    total += start.elapsed();
                    black_box(body);
                }
                total
            })
        })
    });
    g.finish();
}

criterion_group!(clients, bench);
criterion_main!(clients);
