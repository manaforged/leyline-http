use std::sync::Arc;

use bytes::BytesMut;
use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
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
use leyline::internals::pool::{
    DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_H1_CONNS_PER_HOST, Pool,
};

fn bench_pool_new(c: &mut Criterion) {
    c.bench_function("pool::new", |b| {
        b.iter(|| {
            let p = Pool::new();
            black_box(p);
        });
    });
}

fn bench_pool_with_limits(c: &mut Criterion) {
    c.bench_function("pool::with_limits", |b| {
        b.iter(|| {
            let p = Pool::with_limits(
                DEFAULT_IDLE_TIMEOUT,
                DEFAULT_MAX_CONNECTIONS,
                DEFAULT_MAX_H1_CONNS_PER_HOST,
            );
            black_box(p);
        });
    });
}

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
    let mut hdr_buf = [0u8; FRAME_HEADER_LEN];
    if read_exact(&mut io, &mut hdr_buf).await.is_err() {
        return;
    }
    let hdr = FrameHeader::parse(&hdr_buf);
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

fn bench_handle_clone_warm(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let (handle, server): (H2Client, _) = rt.block_on(async {
        let (cio, sio) = tokio::io::duplex(1024 * 1024);
        let server = tokio::spawn(run_mock_server(sio));
        let h = leyline::internals::h2::start(cio, test_config())
            .await
            .expect("handshake");
        let (p, hh) = (
            PseudoHeaders {
                method: "GET".into(),
                scheme: "https".into(),
                authority: "mock".into(),
                path: "/warmup".into(),
                protocol: None,
            },
            vec![("ua".into(), "bench".into())],
        );
        send(&h, p, hh).await;
        (h, server)
    });

    c.bench_function("pool::checkout_hit_equivalent", |b| {
        b.iter(|| {
            let c = handle.clone();
            black_box(c);
        });
    });

    drop(handle);
    server.abort();
    rt.block_on(async move {
        let _ = server.await;
    });
}

fn bench_checkout_scale(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let (handle, server): (H2Client, _) = rt.block_on(async {
        let (cio, sio) = tokio::io::duplex(1024 * 1024);
        let server = tokio::spawn(run_mock_server(sio));
        let h = leyline::internals::h2::start(cio, test_config())
            .await
            .expect("handshake");
        let p = PseudoHeaders {
            method: "GET".into(),
            scheme: "https".into(),
            authority: "mock".into(),
            path: "/warmup".into(),
            protocol: None,
        };
        send(&h, p, vec![("ua".into(), "bench".into())]).await;
        (h, server)
    });

    let mut group = c.benchmark_group("pool::checkout_at_occupancy");
    for n in [1usize, 64, 512, 2048] {
        let pool = Pool::with_limits(
            DEFAULT_IDLE_TIMEOUT,
            n.max(DEFAULT_MAX_CONNECTIONS),
            DEFAULT_MAX_H1_CONNS_PER_HOST,
        );
        pool.bench_populate_h2(n, &handle);
        assert!(pool.bench_probe(), "occupancy {n}: probe must hit");
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::from_parameter(n), &pool, |b, pool| {
            b.iter(|| black_box(pool.bench_probe()));
        });
    }
    group.finish();

    drop(handle);
    server.abort();
    rt.block_on(async move {
        let _ = server.await;
    });
}

criterion_group!(
    pool_benches,
    bench_pool_new,
    bench_pool_with_limits,
    bench_handle_clone_warm,
    bench_checkout_scale
);
criterion_main!(pool_benches);

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
