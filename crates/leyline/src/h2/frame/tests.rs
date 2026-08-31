use super::*;

#[test]
fn frame_header_roundtrip() {
    let header = FrameHeader {
        length: 16384,
        frame_type: 0x0,
        flags: 0x1,
        stream_id: 1,
    };
    let mut buf = BytesMut::with_capacity(9);
    header.encode(&mut buf);
    assert_eq!(buf.len(), 9);

    let parsed = FrameHeader::parse(&buf[..9].try_into().unwrap());
    assert_eq!(parsed.length, 16384);
    assert_eq!(parsed.frame_type, 0x0);
    assert_eq!(parsed.flags, 0x1);
    assert_eq!(parsed.stream_id, 1);
}

#[test]
fn frame_header_clears_reserved_bit() {
    let mut raw = [0u8; 9];
    raw[5] = 0x80;
    raw[8] = 0x01;
    let header = FrameHeader::parse(&raw);
    assert_eq!(header.stream_id, 1);
}

#[test]
fn unknown_frame_types_parse() {
    let header = FrameHeader {
        length: 4,
        frame_type: 0xFF,
        flags: 0x0,
        stream_id: 0,
    };
    let payload = Bytes::from_static(&[0, 0, 0, 0]);
    let frame = Frame::parse(header, payload).unwrap();
    assert!(matches!(
        frame,
        Frame::Unknown {
            frame_type: 0xFF,
            ..
        }
    ));
}
