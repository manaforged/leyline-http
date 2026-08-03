//! Frame codec fuzz: arbitrary bytes → `FrameHeader::parse` + `Frame::parse`
//! must never panic.
//!
//! Mirrors `codec::FrameReader`'s framing: a 9-byte header followed by exactly
//! `length` payload bytes, repeated until the input runs out. A truncated
//! final frame is treated like the reader still awaiting bytes. Parse errors
//! are the correct outcome for malformed input — panics are the bug.
#![no_main]

use bytes::Bytes;
use leyline::h2::frame::{FRAME_HEADER_LEN, Frame, FrameHeader};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut rest = data;
    while rest.len() >= FRAME_HEADER_LEN {
        let header = FrameHeader::parse(rest[..FRAME_HEADER_LEN].try_into().expect("9 bytes"));
        rest = &rest[FRAME_HEADER_LEN..];
        let Some(payload) = rest.get(..header.length as usize) else {
            break; // truncated final frame
        };
        rest = &rest[header.length as usize..];
        let _ = Frame::parse(header, Bytes::copy_from_slice(payload));
    }
});
