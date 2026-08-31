//! Head-to-head HPACK peer comparison (Leyline vs the `h2` crate).
//!
//! # Status: h2 crate does not expose HPACK primitives publicly
//!
//! The `h2` crate (hyperium/h2, v0.4) keeps its HPACK encoder/decoder inside
//! a private `h2::hpack` module. The only public entry points to its header
//! compression machinery are `h2::client::handshake` and the `SendRequest`
//! it returns — a full handshake that pulls in tokio IO, the frame codec,
//! and the SETTINGS/ACK round-trip. Measuring HPACK cost through that is
//! apples-to-oranges and would muddy the numbers.
//!
//! So this bench reports Leyline-only figures here. A proper peer would
//! need `h2` to expose its HPACK module.

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use leyline::h2::hpack::{Decoder, Encoder};

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

fn raw_bytes(h: &[(&str, &str)]) -> u64 {
    h.iter().map(|(n, v)| (n.len() + v.len()) as u64).sum()
}

fn bench_leyline_encode(c: &mut Criterion) {
    let headers = chrome_147_request_headers();
    let mut g = c.benchmark_group("hpack_vs_h2");
    g.throughput(Throughput::Bytes(raw_bytes(&headers)));
    g.bench_function("leyline_encode", |b| {
        b.iter_batched(
            Encoder::new,
            |mut enc| {
                black_box(enc.encode_header_block(black_box(&headers)));
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

fn bench_leyline_decode(c: &mut Criterion) {
    let headers = chrome_147_request_headers();
    let encoded = Encoder::new().encode_header_block(&headers);
    let mut g = c.benchmark_group("hpack_vs_h2");
    g.throughput(Throughput::Bytes(encoded.len() as u64));
    g.bench_function("leyline_decode", |b| {
        b.iter_batched(
            Decoder::new,
            |mut dec| {
                black_box(dec.decode_header_block(black_box(&encoded)).unwrap());
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

/// Peer-comparison placeholder that records the deferred status. Not timed
/// against anything — exists so the benchmark set keeps the named slot.
fn bench_h2_peer_note(c: &mut Criterion) {
    c.bench_function("hpack_vs_h2::h2_peer_status", |b| {
        // A no-op of the note — this is intentionally trivial; see
        // the module doc for the real explanation.
        b.iter(|| {
            black_box("h2 crate does not expose hpack publicly");
        });
    });
}

criterion_group!(
    hpack_vs_h2,
    bench_leyline_encode,
    bench_leyline_decode,
    bench_h2_peer_note
);
criterion_main!(hpack_vs_h2);
