#![no_main]
//! Fuzz the HTTP/2 frame parser.
//!
//! Input layout: raw bytes starting with the 9-byte frame header followed
//! by the payload. Malformed lengths, unknown frame types, and truncated
//! payloads all exercise the parser's error paths.

use bytes::Bytes;
use leyline_h2::frame::{Frame, FrameHeader, FRAME_HEADER_LEN};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < FRAME_HEADER_LEN {
        return;
    }
    let mut header_buf = [0u8; FRAME_HEADER_LEN];
    header_buf.copy_from_slice(&data[..FRAME_HEADER_LEN]);
    let header = FrameHeader::parse(&header_buf);

    // Cap payload to something the parser would see on a real wire so we
    // don't spend fuzz cycles on pathological gigabyte inputs.
    let payload_raw = &data[FRAME_HEADER_LEN..];
    let cap = payload_raw.len().min(header.length as usize).min(1 << 20);
    let payload = Bytes::copy_from_slice(&payload_raw[..cap]);

    let _ = Frame::parse(header, payload);
});
