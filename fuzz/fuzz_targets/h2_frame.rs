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
            break;         };
        rest = &rest[header.length as usize..];
        let _ = Frame::parse(header, Bytes::copy_from_slice(payload));
    }
});
