//! Connection-pool micro-benchmarks.
//!
//! The pool's hot path is:
//!   * `Pool::new()` at session construction.
//!   * `Pool::checkout(key)` on every outbound request — an `Arc<Mutex<...>>`
//!     hash-map lookup followed by an `H2Client::clone` on hit.
//!   * `Pool::install(...)` on cold requests — one LRU evict check + insert.
//!
//! `checkout`/`install` are private, so we approximate them:
//!   * Construction cost → `Pool::new()` and `Pool::with_limits(...)`.
//!   * Warm checkout cost → `H2Client::clone()` (the operation `checkout`
//!     performs under the mutex; the hash lookup is O(1) over a string key,
//!     and the critical section is a few dozen ns — Arc clone dominates).
//!
//! This isolates the pool cost that a user pays on every request.

use std::time::Duration;

use bytes::BytesMut;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::runtime::Runtime;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{
    DataFrame, FrameHeader, FrameType, HeadersFrame, SettingsFrame, FRAME_HEADER_LEN,
};
use leyline::h2::hpack;
use leyline::h2::H2Client;
use leyline::pool::{
    Pool, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_H1_CONNS_PER_HOST,
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

// Warm checkout cost is measured as H2Client clone over a live driver —
// this is precisely what `Pool::checkout` does once it finds the entry.
// Mock server setup mirrors multiplex.rs.

fn test_config() -> H2Config {
    H2Config {
        settings: vec![
            (SettingId::HeaderTableSize, 4096),
            (SettingId::EnablePush, 0),
            (SettingId::InitialWindowSize, 65535),
            (SettingId::MaxFrameSize, 16384),
        ],
        settings_order: vec![
            SettingId::HeaderTableSize,
            SettingId::EnablePush,
            SettingId::InitialWindowSize,
            SettingId::MaxFrameSize,
        ],
        pseudo_order: [
            PseudoOrder::Method,
            PseudoOrder::Authority,
            PseudoOrder::Scheme,
            PseudoOrder::Path,
        ],
        initial_connection_window_size: 65535,
        default_priority: None,
        rst_stream_flood_threshold: 100_000,
        rst_stream_flood_window: Duration::from_secs(10),
        settings_ack_timeout: Duration::from_secs(10),
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 20,
        settings_flood_window: Duration::from_secs(10),
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
    let (handle, server, _driver): (H2Client, _, _) = rt.block_on(async {
        let (cio, sio) = tokio::io::duplex(1024 * 1024);
        let server = tokio::spawn(run_mock_server(sio));
        let (h, d) = ClientConnection::start(cio, test_config())
            .await
            .expect("handshake");
        // One warmup request so the driver is fully rolling.
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
        let _ = h.send_request(p, hh, None).await.unwrap();
        (h, server, d)
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

criterion_group!(
    pool_benches,
    bench_pool_new,
    bench_pool_with_limits,
    bench_handle_clone_warm
);
criterion_main!(pool_benches);
