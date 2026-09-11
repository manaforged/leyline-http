
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::BytesMut;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use futures_util::future::join_all;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::runtime::Runtime;

use leyline::h2::config::{H2Config, PseudoOrder, SettingId};
use leyline::h2::connection::{ClientConnection, PseudoHeaders};
use leyline::h2::frame::{
    DataFrame, FRAME_HEADER_LEN, FrameHeader, FrameType, HeadersFrame, SettingsFrame,
};
use leyline::h2::hpack;
use leyline::{Browser, Session};


struct Counting;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            let new_live = LIVE_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed)
                + layout.size() as u64;
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
        unsafe { System.dealloc(ptr, layout) };
        LIVE_BYTES.fetch_sub(layout.size() as u64, Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { self.alloc(layout) };
        if !p.is_null() {
            unsafe { std::ptr::write_bytes(p, 0, layout.size()) };
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
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
        max_response_body_bytes: 100 * 1024 * 1024,
        max_header_block_bytes: 256 * 1024,
        settings_flood_threshold: 20,
        settings_flood_window: Duration::from_secs(10),
        header_block_reassembly_timeout: Duration::from_secs(10),
    }
}

async fn read_exact<S: AsyncRead + Unpin>(s: &mut S, buf: &mut [u8]) -> std::io::Result<()> {
    s.read_exact(buf).await.map(|_| ())
}

#[derive(Clone, Copy)]
enum RespProfile {
    Tiny,
    Realistic,
}

const REALISTIC_HEADERS: &[(&str, &str)] = &[
    (":status", "200"),
    ("content-type", "text/html; charset=utf-8"),
    ("date", "Sat, 21 Jun 2026 12:00:00 GMT"),
    ("server", "cloudflare"),
    ("cache-control", "private, max-age=0, no-cache"),
    ("vary", "Accept-Encoding"),
    ("x-frame-options", "SAMEORIGIN"),
    ("strict-transport-security", "max-age=31536000"),
    ("x-content-type-options", "nosniff"),
    ("referrer-policy", "strict-origin-when-cross-origin"),
    (
        "set-cookie",
        "sid=abc123def456ghi789; Path=/; Secure; HttpOnly; SameSite=Lax",
    ),
    ("cf-ray", "8fa1b2c3d4e5f6a7-IAD"),
    ("alt-svc", "h3=\":443\"; ma=86400"),
];

const REALISTIC_BODY: &[u8] = &[b'x'; 2048];

fn resp_headers(profile: RespProfile) -> &'static [(&'static str, &'static str)] {
    match profile {
        RespProfile::Tiny => &[(":status", "200")],
        RespProfile::Realistic => REALISTIC_HEADERS,
    }
}

fn resp_body(profile: RespProfile) -> &'static [u8] {
    match profile {
        RespProfile::Tiny => b"ok-10byte!",
        RespProfile::Realistic => REALISTIC_BODY,
    }
}

async fn run_mock_server(mut io: DuplexStream, profile: RespProfile) {
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
    let mut enc = hpack::Encoder::new();
    let headers = resp_headers(profile);
    let body = resp_body(profile);
    let mut bo = BytesMut::new();
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
            let frag = enc.encode_header_block(headers);
            bo.clear();
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
                data: bytes::Bytes::from_static(body),
                wire_len: body.len() as u64,
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
        std::borrow::Cow<'static, str>,
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


fn bench_per_request(c: &mut Criterion) {
    let mut g = c.benchmark_group("allocs");
    g.sample_size(10);
    g.measurement_time(Duration::from_secs(5));
    per_request_profile(&mut g, "per_request_tiny", RespProfile::Tiny);
    per_request_profile(&mut g, "per_request_realistic", RespProfile::Realistic);
    g.finish();
}

fn per_request_profile(
    g: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    label: &str,
    profile: RespProfile,
) {
    let rt = Runtime::new().expect("tokio runtime");

    let (handle, server_task, _driver) = rt.block_on(async {
        let (cio, sio) = tokio::io::duplex(1024 * 1024);
        let server = tokio::spawn(run_mock_server(sio, profile));
        let (h, d) = ClientConnection::start(cio, test_config())
            .await
            .expect("handshake");
        let (p, hh) = req();
        let _ = h.send_request(p, hh, None).await.unwrap();
        (h, server, d)
    });

    const N: u64 = 100;
    g.bench_function(label, |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
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
                let allocs_per_req = (end_allocs - start_allocs) as f64 / (iters * N) as f64;
                eprintln!(
                    "[allocs::{label}] iters={iters} total_reqs={} allocs/req={allocs_per_req:.1}",
                    iters * N
                );
                elapsed / (N as u32)
            })
        })
    });

    drop(handle);
    server_task.abort();
    rt.block_on(async move {
        let _ = server_task.await;
    });
}


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
                    let server = tokio::spawn(run_mock_server(sio, RespProfile::Tiny));
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


fn bench_response_header_materialize(c: &mut Criterion) {
    let mut enc = hpack::Encoder::new();
    let block = enc.encode_header_block(&[
        (":status", "200"),
        ("content-type", "text/html; charset=utf-8"),
        ("date", "Mon, 21 Jun 2026 12:00:00 GMT"),
        ("server", "nginx"),
        ("cache-control", "max-age=3600"),
        ("content-length", "1234"),
        ("vary", "Accept-Encoding"),
        ("x-frame-options", "DENY"),
    ]);

    let mut g = c.benchmark_group("allocs");
    g.sample_size(10);

    g.bench_function("response_headers_clone", |b| {
        b.iter_custom(|iters| {
            let start = ALLOCS.load(Ordering::Relaxed);
            let t0 = std::time::Instant::now();
            for _ in 0..iters {
                let mut dec = hpack::Decoder::new();
                let decoded = dec.decode_header_block(&block).unwrap();
                let mut dest: Vec<(bytes::Bytes, bytes::Bytes)> = Vec::new();
                for h in &decoded {
                    if h.name.as_ref() != b":status" && !h.name.starts_with(b":") {
                        dest.push((h.name.clone(), h.value.clone()));
                    }
                }
                black_box((decoded, dest));
            }
            let el = t0.elapsed();
            let per = (ALLOCS.load(Ordering::Relaxed) - start) as f64 / iters as f64;
            eprintln!("[allocs::response_headers_clone] allocs/response={per:.1}");
            el
        })
    });

    g.bench_function("response_headers_move", |b| {
        b.iter_custom(|iters| {
            let start = ALLOCS.load(Ordering::Relaxed);
            let t0 = std::time::Instant::now();
            for _ in 0..iters {
                let mut dec = hpack::Decoder::new();
                let decoded = dec.decode_header_block(&block).unwrap();
                let mut dest: Vec<(bytes::Bytes, bytes::Bytes)> = Vec::new();
                for h in decoded {
                    if h.name.as_ref() != b":status" && !h.name.starts_with(b":") {
                        dest.push((h.name, h.value));
                    }
                }
                black_box(dest);
            }
            let el = t0.elapsed();
            let per = (ALLOCS.load(Ordering::Relaxed) - start) as f64 / iters as f64;
            eprintln!("[allocs::response_headers_move] allocs/response={per:.1}");
            el
        })
    });
    g.finish();
}

criterion_group!(
    alloc_benches,
    bench_per_request,
    bench_concurrent_footprint,
    bench_session_build_allocs,
    bench_response_header_materialize
);
criterion_main!(alloc_benches);
