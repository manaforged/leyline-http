use super::*;
use crate::h2::frame::{FrameHeader, FrameType};

fn header(len: u32, flags: u8) -> FrameHeader {
    FrameHeader {
        length: len,
        frame_type: FrameType::Data as u8,
        flags,
        stream_id: 1,
    }
}

#[test]
fn padded_frame_wire_len_covers_pad_octet_and_padding() {
    let mut payload = vec![4u8];
    payload.extend_from_slice(b"hello");
    payload.extend_from_slice(&[0u8; 4]);
    let f = DataFrame::parse(header(10, flags::PADDED), Bytes::from(payload)).unwrap();
    assert_eq!(f.data.as_ref(), b"hello");
    assert_eq!(f.wire_len, 10);
    assert_ne!(f.wire_len as usize, f.data.len());
}

#[test]
fn unpadded_frame_wire_len_equals_data_len() {
    let f = DataFrame::parse(header(5, 0), Bytes::from_static(b"hello")).unwrap();
    assert_eq!(f.data.as_ref(), b"hello");
    assert_eq!(f.wire_len, 5);
}

#[test]
fn padding_equal_to_payload_is_protocol_error() {
    let payload = vec![10u8, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    DataFrame::parse(header(10, flags::PADDED), Bytes::from(payload)).expect_err("expected Err");
}
