//! HPACK encode / decode throughput on a realistic Chrome-147 request
//! header set (~15 headers, ~500 bytes before encoding).
//!
//! Measures Leyline's encoder + decoder in isolation. No peer comparison:
//! adding the `h2` crate as a dev-dep would pull a full tokio/hyper stack
//! into the bench suite for one micro-benchmark, so that is left as a
//! future exercise (noted in the bench module doc).

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use leyline::h2::hpack::{Decoder, Encoder};

/// ~15 headers, ~500 bytes raw — matches what `session.get(url)` produces
/// for a typical navigation request on Chrome 147.
fn chrome_147_request_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        (":method", "GET"),
        (":authority", "www.google.com"),
        (":scheme", "https"),
        (
            ":path",
            "/search?q=leyline+http+client&sourceid=chrome&ie=UTF-8",
        ),
        (
            "sec-ch-ua",
            "\"Google Chrome\";v=\"147\", \"Chromium\";v=\"147\", \"Not-A.Brand\";v=\"24\"",
        ),
        ("sec-ch-ua-mobile", "?0"),
        ("sec-ch-ua-platform", "\"Windows\""),
        ("upgrade-insecure-requests", "1"),
        (
            "user-agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36",
        ),
        (
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7",
        ),
        ("sec-fetch-site", "none"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-user", "?1"),
        ("sec-fetch-dest", "document"),
        ("accept-encoding", "gzip, deflate, br, zstd"),
        ("accept-language", "en-US,en;q=0.9"),
    ]
}

fn raw_header_bytes(h: &[(&str, &str)]) -> u64 {
    h.iter().map(|(n, v)| (n.len() + v.len()) as u64).sum()
}

fn bench_encode(c: &mut Criterion) {
    let headers = chrome_147_request_headers();
    let raw = raw_header_bytes(&headers);
    let mut g = c.benchmark_group("hpack");
    g.throughput(Throughput::Bytes(raw));
    g.bench_function("encode_chrome_headers", |b| {
        b.iter_batched(
            Encoder::new,
            |mut enc| {
                let out = enc.encode_header_block(black_box(&headers));
                black_box(out);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

fn bench_decode(c: &mut Criterion) {
    let headers = chrome_147_request_headers();
    let encoded = Encoder::new().encode_header_block(&headers);
    let mut g = c.benchmark_group("hpack");
    g.throughput(Throughput::Bytes(encoded.len() as u64));
    g.bench_function("decode_chrome_headers", |b| {
        b.iter_batched(
            Decoder::new,
            |mut dec| {
                let out = dec.decode_header_block(black_box(&encoded)).unwrap();
                black_box(out);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

fn bench_roundtrip(c: &mut Criterion) {
    let headers = chrome_147_request_headers();
    c.bench_function("hpack::roundtrip_chrome_headers", |b| {
        b.iter(|| {
            let mut enc = Encoder::new();
            let bytes = enc.encode_header_block(black_box(&headers));
            let mut dec = Decoder::new();
            let decoded = dec.decode_header_block(&bytes).unwrap();
            black_box(decoded);
        });
    });
}

criterion_group!(hpack_benches, bench_encode, bench_decode, bench_roundtrip);
criterion_main!(hpack_benches);
