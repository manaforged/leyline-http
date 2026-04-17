//! Frame::parse throughput over a precomputed mix representative of a
//! mid-stream connection: HEADERS, DATA, SETTINGS, WINDOW_UPDATE, PING,
//! RST_STREAM. Raw bytes built once, then parsed in the timed loop.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use leyline_h2::frame::{Frame, FrameHeader};

fn hdr(length: u32, frame_type: u8, flags: u8, stream_id: u32) -> FrameHeader {
    FrameHeader { length, frame_type, flags, stream_id }
}

/// (FrameHeader, payload) pairs covering the common mid-connection frames.
fn build_frame_mix() -> Vec<(FrameHeader, Bytes)> {
    // HEADERS stream 1, END_HEADERS | END_STREAM, small HPACK fragment.
    let headers_payload = Bytes::from_static(&[
        0x82, 0x86, 0x84, 0x41, 0x0a, b'e', b'x', b'a', b'm', b'p', b'l', b'e', b'.', b'c', b'o',
    ]);
    let data_payload = Bytes::from(vec![0xABu8; 1024]);
    // SETTINGS with 6 Chrome-ish params.
    let mut settings = Vec::with_capacity(36);
    for (id, val) in [
        (0x1u16, 65536u32), (0x2, 0), (0x3, 1000),
        (0x4, 6291456), (0x5, 16384), (0x6, 262144),
    ] {
        settings.extend_from_slice(&id.to_be_bytes());
        settings.extend_from_slice(&val.to_be_bytes());
    }

    vec![
        (hdr(headers_payload.len() as u32, 0x1, 0x4 | 0x1, 1), headers_payload),
        (hdr(data_payload.len() as u32, 0x0, 0x1, 1), data_payload),
        (hdr(settings.len() as u32, 0x4, 0, 0), Bytes::from(settings)),
        (hdr(4, 0x8, 0, 0), Bytes::from_static(&[0x00, 0x0F, 0x00, 0x00])), // WINDOW_UPDATE
        (hdr(8, 0x6, 0, 0), Bytes::from_static(&[0, 1, 2, 3, 4, 5, 6, 7])), // PING
        (hdr(4, 0x3, 0, 3), Bytes::from_static(&[0, 0, 0, 0x08])),         // RST_STREAM CANCEL
    ]
}

fn bench_parse_mix(c: &mut Criterion) {
    let mix = build_frame_mix();
    let total_bytes: u64 = mix.iter().map(|(_h, p)| 9 + p.len() as u64).sum();

    let mut g = c.benchmark_group("frames");
    g.throughput(Throughput::Bytes(total_bytes));
    g.bench_function("parse_mix", |b| {
        b.iter(|| {
            for (header, payload) in &mix {
                let f = Frame::parse(*header, payload.clone()).unwrap();
                black_box(f);
            }
        });
    });
    g.finish();
}

fn bench_parse_header_only(c: &mut Criterion) {
    // 9-byte header parse in a tight loop — the hottest code path for any
    // frame type, hit once per frame before dispatch.
    let raw: [u8; 9] = [0x00, 0x10, 0x00, 0x01, 0x04, 0x00, 0x00, 0x00, 0x01];
    c.bench_function("frames::parse_header", |b| {
        b.iter(|| {
            black_box(FrameHeader::parse(black_box(&raw)));
        });
    });
}

criterion_group!(frame_benches, bench_parse_mix, bench_parse_header_only);
criterion_main!(frame_benches);
