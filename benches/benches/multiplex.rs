use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use futures_util::future::join_all;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::runtime::Runtime;

use leyline::internals::h2::H2Config;
use leyline::internals::h2::H2Limits;
use leyline::internals::h2::connection::PseudoHeaders;
use leyline::internals::h2::frame::{
    DataFrame, FRAME_HEADER_LEN, FrameHeader, FrameType, HeadersFrame, SettingsFrame,
};
use leyline::internals::h2::hpack;
use leyline::internals::h2::{H2Client, Head};
use leyline::internals::pool::TransportResponse;

const MOCK_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

fn test_config() -> H2Config {
    H2Config {
        limits: H2Limits {
            rst_stream_flood_threshold: 100_000,
            ..H2Limits::default()
        },
        ..H2Config::default()
    }
}

async fn read_exact<S: AsyncRead + Unpin>(s: &mut S, buf: &mut [u8]) -> std::io::Result<()> {
    s.read_exact(buf).await.map(|_| ())
}

async fn run_mock_server(mut io: DuplexStream) {
    let mut preface = [0u8; 24];
    if read_exact(&mut io, &mut preface).await.is_err() {
        return;
    }
    assert_eq!(&preface[..], MOCK_PREFACE);

    let mut hdr_buf = [0u8; FRAME_HEADER_LEN];
    if read_exact(&mut io, &mut hdr_buf).await.is_err() {
        return;
    }
    let hdr = FrameHeader::parse(&hdr_buf);
    assert_eq!(hdr.frame_type, FrameType::Settings as u8);
    let mut body = vec![0u8; hdr.length as usize];
    if hdr.length > 0 {
        let _ = read_exact(&mut io, &mut body).await;
    }

    let mut out = BytesMut::new();
    SettingsFrame {
        ack: false,
        params: vec![],
    }
    .encode(&mut out);
    if io.write_all(&out).await.is_err() {
        return;
    }
    out.clear();
    SettingsFrame::ack().encode(&mut out);
    if io.write_all(&out).await.is_err() {
        return;
    }

    if read_exact(&mut io, &mut hdr_buf).await.is_err() {
        return;
    }

    loop {
        let mut hb = [0u8; FRAME_HEADER_LEN];
        if read_exact(&mut io, &mut hb).await.is_err() {
            return;
        }
        let h = FrameHeader::parse(&hb);
        let mut p = vec![0u8; h.length as usize];
        if h.length > 0 && read_exact(&mut io, &mut p).await.is_err() {
            return;
        }
        if h.frame_type == FrameType::Headers as u8 {
            let sid = h.stream_id;
            let mut enc = hpack::Encoder::new();
            let frag = enc.encode_header_block(&[(":status", "200")]);
            let mut bo = BytesMut::new();
            HeadersFrame {
                stream_id: sid,
                end_stream: false,
                end_headers: true,
                priority: None,
                fragment: bytes::Bytes::from(frag),
            }
            .encode(&mut bo);
            if io.write_all(&bo).await.is_err() {
                return;
            }
            bo.clear();
            DataFrame {
                stream_id: sid,
                end_stream: true,
                data: bytes::Bytes::from_static(b"ok-10byte!"),
                wire_len: 10,
            }
            .encode(&mut bo);
            if io.write_all(&bo).await.is_err() {
                return;
            }
        }
    }
}

fn req() -> (
    PseudoHeaders,
    Vec<(
        std::borrow::Cow<'static, str>,
        leyline::internals::h2::HeaderStr,
    )>,
) {
    (
        PseudoHeaders {
            method: "GET".into(),
            scheme: "https".into(),
            authority: "mock".into(),
            path: "/bench".into(),
            protocol: None,
        },
        vec![("user-agent".into(), "leyline-bench".into())],
    )
}

async fn build_client() -> (H2Client, tokio::task::JoinHandle<()>) {
    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
    let server = tokio::spawn(run_mock_server(server_io));
    let handle = leyline::internals::h2::start(client_io, test_config())
        .await
        .expect("handshake");
    (handle, server)
}

fn bench_serial_1k(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    const N: usize = 1000;

    let mut g = c.benchmark_group("multiplex");
    g.throughput(Throughput::Elements(N as u64));
    g.sample_size(20);
    g.measurement_time(Duration::from_secs(15));
    g.bench_function("serial_1k_requests", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let (handle, server) = build_client().await;
                    let start = std::time::Instant::now();
                    for _ in 0..N {
                        let (p, h) = req();
                        let resp = send(&handle, p, h).await;
                        black_box(resp);
                    }
                    total += start.elapsed();
                    drop(handle);
                    server.abort();
                    let _ = server.await;
                }
                total
            })
        })
    });
    g.finish();
}

fn bench_concurrent_100(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    const N: usize = 100;

    let mut g = c.benchmark_group("multiplex");
    g.throughput(Throughput::Elements(N as u64));
    g.sample_size(30);
    g.measurement_time(Duration::from_secs(15));
    g.bench_function("concurrent_100_inflight", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let (handle, server) = build_client().await;
                    let start = std::time::Instant::now();
                    let mut futs = Vec::with_capacity(N);
                    for _ in 0..N {
                        let h = handle.clone();
                        let (p, hh) = req();
                        futs.push(tokio::spawn(async move { send(&h, p, hh).await }));
                    }
                    let results = join_all(futs).await;
                    total += start.elapsed();
                    for r in results {
                        black_box(r.unwrap());
                    }
                    drop(handle);
                    server.abort();
                    let _ = server.await;
                }
                total
            })
        })
    });
    g.finish();
}

fn bench_handle_clone(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let (handle, server) = rt.block_on(build_client());
    c.bench_function("multiplex::handle_clone", |b| {
        b.iter(|| {
            let c = handle.clone();
            black_box(c);
        })
    });
    drop(handle);
    server.abort();
    rt.block_on(async move {
        let _ = server.await;
    });
}

criterion_group!(
    multiplex_benches,
    bench_handle_clone,
    bench_serial_1k,
    bench_concurrent_100
);
criterion_main!(multiplex_benches);

async fn send(
    handle: &H2Client,
    pseudo: PseudoHeaders,
    headers: Vec<(
        std::borrow::Cow<'static, str>,
        leyline::internals::h2::HeaderStr,
    )>,
) -> Box<TransportResponse> {
    handle
        .send_shared(
            Arc::new(Head::new(pseudo, headers)),
            leyline::Body::default(),
            false,
        )
        .await
        .expect("req ok")
}
