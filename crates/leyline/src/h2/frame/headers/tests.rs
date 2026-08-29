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
    // wire: [pad_len=4][4 padding] -> empty fragment, no error.
    // The old check rejected this off by one.
    let payload = vec![4u8, 0, 0, 0, 0];
    let f = HeadersFrame::parse(header(5, flags::PADDED), Bytes::from(payload)).unwrap();
    assert!(f.fragment.is_empty());
}

#[test]
fn padding_overwriting_the_octet_is_protocol_error() {
    // pad_len = 6 with a 6-byte payload: octet + padding (7) cannot
    // fit, so the padding length lies about the frame. Error.
    let payload = vec![6u8, 0, 0, 0, 0, 0];
    assert!(HeadersFrame::parse(header(6, flags::PADDED), Bytes::from(payload)).is_err());
}

#[test]
fn padded_with_priority_fragments_after_priority() {
    // wire: [pad_len=2][priority 5B][frag "AB"][pad 2B]
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
