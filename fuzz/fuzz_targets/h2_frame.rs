#![no_main]

use bytes::Bytes;
use leyline::internals::h2::frame::{FRAME_HEADER_LEN, Frame, FrameHeader, U31_MAX};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut rest = data;
    while rest.len() >= FRAME_HEADER_LEN {
        let header = FrameHeader::parse(rest[..FRAME_HEADER_LEN].try_into().expect("9 bytes"));
        rest = &rest[FRAME_HEADER_LEN..];
        let Some(payload) = rest.get(..header.length as usize) else {
            break;
        };
        rest = &rest[header.length as usize..];
        if let Ok(frame) = Frame::parse(header, Bytes::copy_from_slice(payload)) {
            match frame {
                Frame::WindowUpdate(f) => assert!(f.increment > 0 && f.increment <= U31_MAX),
                Frame::Ping(_) | Frame::GoAway(_) => assert_eq!(header.stream_id, 0),
                Frame::Settings(s) => {
                    assert_eq!(header.stream_id, 0);
                    if s.ack {
                        assert!(s.params.is_empty());
                    }
                }
                Frame::RstStream(r) => assert_ne!(r.stream_id, 0),
                _ => {}
            }
        }
    }
});
