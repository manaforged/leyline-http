//! Allocation profile for the request hot path.
//!
//! This bench binary installs a wrapper around `std::alloc::System` that
//! tallies allocation count and live-byte watermark on every alloc/dealloc.
//! Criterion is then used only as a harness to print the numbers once per
//! benchmark — throughput / ns-per-op measurements are a side effect.
//!
//! We report three numbers:
//!   * Allocations per `send_request` on an already-warm connection
//!     (serial path).
//!   * Peak resident bytes while 100 concurrent streams are in flight.
//!   * Allocations per `Session::builder().build()?`.
//!
//! Everything runs against the in-process mock HTTP/2 peer defined inline.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::BytesMut;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use futures_util::future::join_all;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::runtime::Runtime;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{
    DataFrame, FrameHeader, FrameType, HeadersFrame, SettingsFrame, FRAME_HEADER_LEN,
};
use leyline::h2::hpack;
use leyline::{Browser, Session};

// ---------------------------------------------------------------------------
// Counting allocator.
// ---------------------------------------------------------------------------

struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            let new_live = LIVE_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed)
                + layout.size() as u64;
            // Relaxed peak update — good enough for reporting.
            let mut peak = PEAK_BYTES.load(Ordering::Relaxed);
            while new_live > peak {
                match PEAK_BYTES.compare_exchange_weak(
                    peak,
                    new_live,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(p) => peak = p,
                }
            }
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = self.alloc(layout);
        if !p.is_null() {
            std::ptr::write_bytes(p, 0, layout.size());
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = System.realloc(ptr, layout, new_size);
        if !new.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            LIVE_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
            let new_live =
                LIVE_BYTES.fetch_add(new_size as u64, Ordering::Relaxed) + new_size as u64;
            let mut peak = PEAK_BYTES.load(Ordering::Relaxed);
            while new_live > peak {
                match PEAK_BYTES.compare_exchange_weak(
                    peak,
                    new_live,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(p) => peak = p,
                }
            }
        }
        new
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn reset_peak() {
    PEAK_BYTES.store(LIVE_BYTES.load(Ordering::Relaxed), Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Mock H2 server (copy of multiplex.rs to keep this bench self-contained).
// ---------------------------------------------------------------------------

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

fn req() -> (PseudoHeaders, Vec<(String, String)>) {
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

// ---------------------------------------------------------------------------
// Bench: allocations per send_request on a warm connection.
// ---------------------------------------------------------------------------

fn bench_per_request(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");

    // Build the warm connection outside the measurement window.
    let (handle, server_task, _driver) = rt.block_on(async {
        let (cio, sio) = tokio::io::duplex(1024 * 1024);
        let server = tokio::spawn(run_mock_server(sio));
        let (h, d) = ClientConnection::start(cio, test_config())
            .await
            .expect("handshake");
        // Warm the driver by doing 1 request first.
        let (p, hh) = req();
        let _ = h.send_request(p, hh, None).await.unwrap();
        (h, server, d)
    });

    const N: u64 = 100;
    let mut g = c.benchmark_group("allocs");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(5));
    g.bench_function("per_request", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                // Snapshot total alloc count over `iters * N` requests and
                // divide out. Reports time in ns-per-request; the allocation
                // count is emitted to stderr once (see end-of-suite print).
                let start_allocs = ALLOCS.load(Ordering::Relaxed);
                let t0 = std::time::Instant::now();
                for _ in 0..iters {
                    for _ in 0..N {
                        let (p, hh) = req();
                        let r = handle.send_request(p, hh, None).await.unwrap();
                        black_box(r);
                    }
                }
                let elapsed = t0.elapsed();
                let end_allocs = ALLOCS.load(Ordering::Relaxed);
                let allocs_per_req =
                    (end_allocs - start_allocs) as f64 / (iters * N) as f64;
                // Store in a once-cell-style global so the summary at the
                // end can print it; simpler to just eprintln here per call.
                eprintln!(
                    "[allocs::per_request] iters={iters} total_reqs={} allocs/req={allocs_per_req:.1}",
                    iters * N
                );
                elapsed / (N as u32)
            })
        })
    });
    g.finish();

    drop(handle);
    server_task.abort();
    rt.block_on(async move {
        let _ = server_task.await;
    });
}

// ---------------------------------------------------------------------------
// Bench: peak live bytes while 100 concurrent streams are in flight.
// ---------------------------------------------------------------------------

fn bench_concurrent_footprint(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    const N: usize = 100;

    let mut g = c.benchmark_group("allocs");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(5));
    g.bench_function("concurrent_footprint", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                let mut peaks: Vec<u64> = Vec::with_capacity(iters as usize);
                for _ in 0..iters {
                    let (cio, sio) = tokio::io::duplex(1024 * 1024);
                    let server = tokio::spawn(run_mock_server(sio));
                    let (handle, _driver) =
                        ClientConnection::start(cio, test_config()).await.expect("handshake");

                    reset_peak();
                    let before_peak = PEAK_BYTES.load(Ordering::Relaxed);
                    let start = std::time::Instant::now();
                    let mut futs = Vec::with_capacity(N);
                    for _ in 0..N {
                        let h = handle.clone();
                        let (p, hh) = req();
                        futs.push(tokio::spawn(async move {
                            h.send_request(p, hh, None).await.unwrap()
                        }));
                    }
                    let results = join_all(futs).await;
                    total += start.elapsed();
                    let peak = PEAK_BYTES.load(Ordering::Relaxed);
                    peaks.push(peak.saturating_sub(before_peak));
                    for r in results {
                        black_box(r.unwrap());
                    }
                    drop(handle);
                    server.abort();
                    let _ = server.await;
                }
                let avg = peaks.iter().sum::<u64>() / peaks.len() as u64;
                let mib = avg as f64 / (1024.0 * 1024.0);
                eprintln!(
                    "[allocs::concurrent_footprint] peak_delta_avg={avg} bytes ({mib:.2} MiB) over {} iters",
                    peaks.len()
                );
                total / (N as u32)
            })
        })
    });
    g.finish();
}

// ---------------------------------------------------------------------------
// Bench: allocations per Session::builder().build()?.
// ---------------------------------------------------------------------------

fn bench_session_build_allocs(c: &mut Criterion) {
    let mut g = c.benchmark_group("allocs");
    g.sample_size(10);
    g.bench_function("session_build", |b| {
        b.iter_custom(|iters| {
            let start_allocs = ALLOCS.load(Ordering::Relaxed);
            let t0 = std::time::Instant::now();
            for _ in 0..iters {
                let s = Session::builder()
                    .browser(Browser::Chrome147)
                    .build()
                    .expect("session");
                black_box(s);
            }
            let elapsed = t0.elapsed();
            let end_allocs = ALLOCS.load(Ordering::Relaxed);
            let per = (end_allocs - start_allocs) as f64 / iters as f64;
            eprintln!("[allocs::session_build] allocs/build={per:.1} iters={iters}");
            elapsed
        })
    });
    g.finish();
}

criterion_group!(
    alloc_benches,
    bench_per_request,
    bench_concurrent_footprint,
    bench_session_build_allocs
);
criterion_main!(alloc_benches);
