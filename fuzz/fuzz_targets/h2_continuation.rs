//! CONTINUATION reassembly fuzz: header-block fragments split across
//! HEADERS + CONTINUATION frame boundaries, joined and HPACK-decoded.
//!
//! Mirrors the driver's `on_headers` reassembly
//! (`crates/leyline/src/h2/client/driver/recv.rs`): accumulate the HEADERS
//! fragment plus same-stream CONTINUATION fragments until END_HEADERS, then
//! decode the joined block with the connection-persistent `Decoder`. Where
//! production tears the connection down (bare CONTINUATION, interleaved
//! frames) this harness keeps scanning so one input can pack several blocks
//! through the same decoder. Decode errors are the correct outcome for
//! malformed input — panics are the bug.
#![no_main]

use bytes::Bytes;
use leyline::h2::frame::{FRAME_HEADER_LEN, Frame, FrameHeader};
use leyline::h2::hpack::Decoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut decoder = Decoder::new();
    // Open header block: (stream_id, fragments accumulated so far).
    let mut open: Option<(u32, Vec<u8>)> = None;
    let mut rest = data;
    while rest.len() >= FRAME_HEADER_LEN {
        let header = FrameHeader::parse(rest[..FRAME_HEADER_LEN].try_into().expect("9 bytes"));
        rest = &rest[FRAME_HEADER_LEN..];
        let Some(payload) = rest.get(..header.length as usize) else {
            break; // truncated final frame
        };
        rest = &rest[header.length as usize..];
        let frame = match Frame::parse(header, Bytes::copy_from_slice(payload)) {
            Ok(frame) => frame,
            Err(_) => continue,
        };
        match frame {
            Frame::Headers(h) if open.is_none() => {
                if h.end_headers {
                    let _ = decoder.decode_header_block(&h.fragment);
                } else {
                    open = Some((h.stream_id, h.fragment.to_vec()));
                }
            }
            Frame::Continuation {
                stream_id,
                end_headers,
                fragment,
            } => {
                if let Some((sid, mut block)) = open.take_if(|(sid, _)| *sid == stream_id) {
                    block.extend_from_slice(&fragment);
                    if end_headers {
                        let _ = decoder.decode_header_block(&block);
                    } else {
                        open = Some((sid, block));
                    }
                }
            }
            _ => {}
        }
    }
});
