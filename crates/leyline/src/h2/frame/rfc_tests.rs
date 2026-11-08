use bytes::Bytes;

use super::{Frame, FrameHeader};
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;

const MAX_STREAM_ID: u32 = 0x7FFF_FFFF;
const STREAM_IDS: [u32; 3] = [0, 1, MAX_STREAM_ID];
const FLAGS: [u8; 7] = [0x00, 0x01, 0x08, 0x20, 0x28, 0xFE, 0xFF];
const WORDS: [u32; 11] = [
    0,
    1,
    0xd,
    0xe,
    0x0100_0001,
    0x0400_0000,
    0x0700_0000,
    0x0C00_0000,
    MAX_STREAM_ID,
    0x8000_0000,
    u32::MAX,
];
const MAX_PAYLOAD: usize = 13;

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Accepted,
    Connection(ErrorCode),
    Stream(ErrorCode),
}

struct Input {
    header: FrameHeader,
    payload: Vec<u8>,
}

impl Input {
    fn ack(&self) -> bool {
        self.header.flags & 0x1 != 0
    }
    fn word(&self, at: usize) -> u32 {
        let b = &self.payload[at..at + 4];
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
}

fn every_frame(frame_type: u8, mut check: impl FnMut(&Input, Result<Frame, H2Error>)) {
    for len in 0..=MAX_PAYLOAD {
        for stream_id in STREAM_IDS {
            for flags in FLAGS {
                for word in WORDS {
                    let mut payload: Vec<u8> =
                        (0..len).map(|i| (i as u8).wrapping_mul(37)).collect();
                    let head = word.to_be_bytes();
                    let n = len.min(4);
                    payload[..n].copy_from_slice(&head[..n]);
                    let header = FrameHeader {
                        length: len as u32,
                        frame_type,
                        flags,
                        stream_id,
                    };
                    let input = Input { header, payload };
                    let result = Frame::parse(input.header, Bytes::copy_from_slice(&input.payload));
                    check(&input, result);
                }
            }
        }
    }
}

fn outcome(input: &Input, result: &Result<Frame, H2Error>) -> Outcome {
    match result {
        Ok(_) => Outcome::Accepted,
        Err(H2Error::Connection { code, .. }) => Outcome::Connection(*code),
        Err(H2Error::Stream { code, stream_id }) => {
            assert_eq!(
                *stream_id, input.header.stream_id,
                "stream error names the frame's stream"
            );
            Outcome::Stream(*code)
        }
        Err(other) => panic!("unexpected error kind {other}"),
    }
}

fn expect(frame_type: u8, rule: impl Fn(&Input) -> Outcome) {
    every_frame(frame_type, |input, result| {
        assert_eq!(
            outcome(input, &result),
            rule(input),
            "type {frame_type:#x} len {} stream {} flags {:#x} payload {:02x?}",
            input.payload.len(),
            input.header.stream_id,
            input.header.flags,
            input.payload
        );
    });
}

#[test]
fn priority_follows_rfc_9113_section_6_3() {
    expect(0x2, |f| match (f.header.stream_id, f.payload.len()) {
        (0, _) => Outcome::Connection(ErrorCode::ProtocolError),
        (_, 5) => Outcome::Accepted,
        _ => Outcome::Stream(ErrorCode::FrameSizeError),
    });
}

#[test]
fn rst_stream_follows_rfc_9113_section_6_4() {
    expect(0x3, |f| match (f.header.stream_id, f.payload.len()) {
        (0, _) => Outcome::Connection(ErrorCode::ProtocolError),
        (_, 4) => Outcome::Accepted,
        _ => Outcome::Connection(ErrorCode::FrameSizeError),
    });
    every_frame(0x3, |f, result| {
        if let Ok(Frame::RstStream(frame)) = result {
            assert_eq!(frame.stream_id, f.header.stream_id);
            let known = f.word(0) <= 0xd;
            assert_eq!(frame.error_code as u32 == f.word(0), known);
        }
    });
}

#[test]
fn settings_follows_rfc_9113_section_6_5() {
    expect(0x4, |f| {
        let len = f.payload.len();
        if f.header.stream_id != 0 {
            Outcome::Connection(ErrorCode::ProtocolError)
        } else if (f.ack() && len != 0) || len % 6 != 0 {
            Outcome::Connection(ErrorCode::FrameSizeError)
        } else {
            Outcome::Accepted
        }
    });
    every_frame(0x4, |f, result| {
        if let Ok(Frame::Settings(frame)) = result {
            assert_eq!(frame.ack, f.ack());
            let want: Vec<_> = f
                .payload
                .as_chunks::<6>()
                .0
                .iter()
                .map(|p| {
                    (
                        u16::from_be_bytes([p[0], p[1]]),
                        u32::from_be_bytes([p[2], p[3], p[4], p[5]]),
                    )
                })
                .collect();
            assert_eq!(frame.params, want);
        }
    });
}

#[test]
fn ping_follows_rfc_9113_section_6_7() {
    expect(0x6, |f| match (f.header.stream_id, f.payload.len()) {
        (0, 8) => Outcome::Accepted,
        (0, _) => Outcome::Connection(ErrorCode::FrameSizeError),
        _ => Outcome::Connection(ErrorCode::ProtocolError),
    });
    every_frame(0x6, |f, result| {
        if let Ok(Frame::Ping(frame)) = result {
            assert_eq!(frame.payload[..], f.payload[..]);
            assert_eq!(frame.ack, f.ack());
        }
    });
}

#[test]
fn goaway_follows_rfc_9113_section_6_8() {
    expect(0x7, |f| match (f.header.stream_id, f.payload.len()) {
        (0, 8..) => Outcome::Accepted,
        (0, _) => Outcome::Connection(ErrorCode::FrameSizeError),
        _ => Outcome::Connection(ErrorCode::ProtocolError),
    });
    every_frame(0x7, |f, result| {
        if let Ok(Frame::GoAway(frame)) = result {
            assert_eq!(frame.last_stream_id, f.word(0) & MAX_STREAM_ID);
            assert_eq!(frame.debug_data[..], f.payload[8..]);
        }
    });
}

#[test]
fn window_update_follows_rfc_9113_section_6_9() {
    expect(0x8, |f| {
        if f.payload.len() != 4 {
            return Outcome::Connection(ErrorCode::FrameSizeError);
        }
        match (f.word(0) & MAX_STREAM_ID, f.header.stream_id) {
            (0, 0) => Outcome::Connection(ErrorCode::ProtocolError),
            (0, _) => Outcome::Stream(ErrorCode::ProtocolError),
            _ => Outcome::Accepted,
        }
    });
    every_frame(0x8, |f, result| {
        if let Ok(Frame::WindowUpdate(frame)) = result {
            assert_eq!(frame.increment, f.word(0) & MAX_STREAM_ID);
            assert_eq!(frame.stream_id, f.header.stream_id);
        }
    });
}

#[test]
fn every_unknown_frame_type_is_ignored_per_rfc_9113_section_4_1() {
    for frame_type in 0x0a..=u8::MAX {
        every_frame(frame_type, |_, result| {
            assert!(matches!(result, Ok(Frame::Unknown)))
        });
    }
}

fn padded_fields(f: &Input, padded: bool, fixed: usize) -> Result<std::ops::Range<usize>, Outcome> {
    let mandatory = usize::from(padded) + fixed;
    let len = f.payload.len();
    if len < mandatory {
        return Err(Outcome::Connection(ErrorCode::FrameSizeError));
    }
    let pad = if padded { usize::from(f.payload[0]) } else { 0 };
    if pad > len - mandatory {
        return Err(Outcome::Connection(ErrorCode::ProtocolError));
    }
    Ok(mandatory..len - pad)
}

#[test]
fn data_padding_follows_rfc_9113_section_6_1() {
    let rule = |f: &Input| {
        if f.header.stream_id == 0 {
            return Err(Outcome::Connection(ErrorCode::ProtocolError));
        }
        padded_fields(f, f.header.flags & 0x8 != 0, 0)
    };
    expect(0x0, |f| rule(f).map_or_else(|o| o, |_| Outcome::Accepted));
    every_frame(0x0, |f, result| {
        if let Ok(Frame::Data(frame)) = result {
            let range = rule(f).expect("accepted");
            assert_eq!(frame.data[..], f.payload[range]);
            assert_eq!(frame.end_stream, f.header.flags & 0x1 != 0);
        }
    });
}

#[test]
fn headers_padding_and_priority_follow_rfc_9113_section_6_2() {
    let rule = |f: &Input| {
        if f.header.stream_id == 0 {
            return Err(Outcome::Connection(ErrorCode::ProtocolError));
        }
        let fixed = if f.header.flags & 0x20 != 0 { 5 } else { 0 };
        padded_fields(f, f.header.flags & 0x8 != 0, fixed)
    };
    expect(0x1, |f| rule(f).map_or_else(|o| o, |_| Outcome::Accepted));
    every_frame(0x1, |f, result| {
        if let Ok(Frame::Headers(frame)) = result {
            let range = rule(f).expect("accepted");
            assert_eq!(frame.fragment[..], f.payload[range]);
            assert_eq!(frame.priority.is_some(), f.header.flags & 0x20 != 0);
        }
    });
}

#[test]
fn push_promise_padding_follows_rfc_9113_section_6_6() {
    let rule = |f: &Input| {
        if f.header.stream_id == 0 {
            return Err(Outcome::Connection(ErrorCode::ProtocolError));
        }
        padded_fields(f, f.header.flags & 0x8 != 0, 4)
    };
    expect(0x5, |f| rule(f).map_or_else(|o| o, |_| Outcome::Accepted));
    every_frame(0x5, |f, result| {
        if let Ok(Frame::PushPromise(frame)) = result {
            let range = rule(f).expect("accepted");
            assert_eq!(frame.fragment[..], f.payload[range]);
        }
    });
}
