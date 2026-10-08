use super::*;
use crate::h2::frame::{FrameHeader, FrameType};

fn header(len: u32, flags: u8) -> FrameHeader {
    FrameHeader {
        length: len,
        frame_type: FrameType::Headers as u8,
        flags,
        stream_id: 1,
    }
}

#[test]
fn padding_consuming_all_but_the_octet_leaves_empty_fragment() {
    let payload = vec![4u8, 0, 0, 0, 0];
    let f = HeadersFrame::parse(header(5, flags::PADDED), Bytes::from(payload)).unwrap();
    assert!(f.fragment.is_empty());
}

#[test]
fn padding_overwriting_the_octet_is_protocol_error() {
    let payload = vec![6u8, 0, 0, 0, 0, 0];
    HeadersFrame::parse(header(6, flags::PADDED), Bytes::from(payload)).expect_err("expected Err");
}

#[test]
fn padded_with_priority_fragments_after_priority() {
    let mut payload = vec![2u8, 0, 0, 0, 1, 16];
    payload.extend_from_slice(b"AB");
    payload.extend_from_slice(&[0u8; 2]);
    let f = HeadersFrame::parse(
        header(10, flags::PADDED | flags::PRIORITY),
        Bytes::from(payload),
    )
    .unwrap();
    assert!(f.priority.is_some());
    assert_eq!(f.fragment.as_ref(), b"AB");
}
