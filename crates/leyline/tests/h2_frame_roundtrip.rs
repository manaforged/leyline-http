//! Frame roundtrip tests — encode then parse, verify fields survive.
use bytes::BytesMut;
use leyline::h2::frame::*;

fn roundtrip_buf() -> BytesMut {
    BytesMut::with_capacity(1024)
}

#[test]
fn settings_roundtrip() {
    let frame = SettingsFrame {
        ack: false,
        params: vec![
            (0x1, 65536),   // HEADER_TABLE_SIZE
            (0x2, 0),       // ENABLE_PUSH
            (0x4, 6291456), // INITIAL_WINDOW_SIZE
            (0x6, 262144),  // MAX_HEADER_LIST_SIZE
        ],
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = SettingsFrame::parse(header, payload).unwrap();

    assert!(!parsed.ack);
    assert_eq!(parsed.params.len(), 4);
    assert_eq!(parsed.params[0], (0x1, 65536));
    assert_eq!(parsed.params[1], (0x2, 0));
    assert_eq!(parsed.params[2], (0x4, 6291456));
    assert_eq!(parsed.params[3], (0x6, 262144));
}

#[test]
fn settings_ordering_preserved() {
    // Chrome 147 sends settings in a specific order that differs from ID order.
    // The fingerprint depends on this ordering being preserved.
    let chrome_params = vec![
        (0x1, 65536),   // HEADER_TABLE_SIZE
        (0x2, 0),       // ENABLE_PUSH
        (0x4, 6291456), // INITIAL_WINDOW_SIZE (ID 4, before ID 3!)
        (0x6, 262144),  // MAX_HEADER_LIST_SIZE (ID 6, no ID 5)
    ];

    let frame = SettingsFrame {
        ack: false,
        params: chrome_params.clone(),
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = SettingsFrame::parse(header, payload).unwrap();

    // Order must be preserved exactly — this is the fingerprint.
    assert_eq!(parsed.params, chrome_params);
}

#[test]
fn settings_ack_roundtrip() {
    let frame = SettingsFrame::ack();
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    assert_eq!(header.length, 0);
    let payload = bytes::Bytes::new();
    let parsed = SettingsFrame::parse(header, payload).unwrap();
    assert!(parsed.ack);
    assert!(parsed.params.is_empty());
}

#[test]
fn data_roundtrip() {
    let frame = DataFrame {
        stream_id: 1,
        end_stream: true,
        data: bytes::Bytes::from_static(b"hello world"),
        wire_len: 11,
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = DataFrame::parse(header, payload).unwrap();

    assert_eq!(parsed.stream_id, 1);
    assert!(parsed.end_stream);
    assert_eq!(&parsed.data[..], b"hello world");
}

#[test]
fn headers_roundtrip() {
    let frame = HeadersFrame {
        stream_id: 1,
        end_stream: true,
        end_headers: true,
        priority: None,
        fragment: bytes::Bytes::from_static(b"\x82\x86\x84"),
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = HeadersFrame::parse(header, payload).unwrap();

    assert_eq!(parsed.stream_id, 1);
    assert!(parsed.end_stream);
    assert!(parsed.end_headers);
    assert!(parsed.priority.is_none());
    assert_eq!(&parsed.fragment[..], b"\x82\x86\x84");
}

#[test]
fn headers_with_priority_roundtrip() {
    let frame = HeadersFrame {
        stream_id: 3,
        end_stream: false,
        end_headers: true,
        priority: Some(StreamDependency {
            exclusive: true,
            dependency_id: 0,
            weight: 255,
        }),
        fragment: bytes::Bytes::from_static(b"\x82"),
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = HeadersFrame::parse(header, payload).unwrap();

    assert_eq!(parsed.stream_id, 3);
    assert!(!parsed.end_stream);
    let dep = parsed.priority.unwrap();
    assert!(dep.exclusive);
    assert_eq!(dep.dependency_id, 0);
    assert_eq!(dep.weight, 255);
}

#[test]
fn headers_with_chrome_priority_roundtrip() {
    // Chrome's legacy RFC 7540 priority: exclusive=true, dep=0, weight=255
    // (carrying weight 256). Firefox uses exclusive=false, dep varies.
    let params = leyline::h2::PriorityParams {
        exclusive: true,
        stream_dependency: 0,
        weight: 255,
    };
    let frame = HeadersFrame {
        stream_id: 1,
        end_stream: false,
        end_headers: true,
        priority: Some(StreamDependency {
            exclusive: params.exclusive,
            dependency_id: params.stream_dependency,
            weight: params.weight,
        }),
        fragment: bytes::Bytes::from_static(b"\x82\x86\x84"),
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    // Expect PRIORITY flag in the wire flags byte (bit 0x20).
    let header_bytes: [u8; 9] = buf[..9].try_into().unwrap();
    let flags = header_bytes[4];
    assert_eq!(flags & 0x20, 0x20, "PRIORITY flag not set in HEADERS");
    // Payload should include 5-byte priority block before the fragment.
    assert_eq!(header_bytes[2] as usize, 5 + 3); // length = 5 + fragment

    let header = FrameHeader::parse(&header_bytes);
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = HeadersFrame::parse(header, payload).unwrap();
    let dep = parsed.priority.unwrap();
    assert!(dep.exclusive);
    assert_eq!(dep.dependency_id, 0);
    assert_eq!(dep.weight, 255);
    assert_eq!(&parsed.fragment[..], b"\x82\x86\x84");
}

#[test]
fn window_update_roundtrip() {
    let frame = WindowUpdateFrame {
        stream_id: 0,
        increment: 15663105, // Chrome's connection WINDOW_UPDATE
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = WindowUpdateFrame::parse(header, payload).unwrap();

    assert_eq!(parsed.stream_id, 0);
    assert_eq!(parsed.increment, 15663105);
}

#[test]
fn rst_stream_roundtrip() {
    let frame = RstStreamFrame {
        stream_id: 5,
        error_code: leyline::h2::error::ErrorCode::Cancel,
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = RstStreamFrame::parse(header, payload).unwrap();

    assert_eq!(parsed.stream_id, 5);
    assert_eq!(parsed.error_code, leyline::h2::error::ErrorCode::Cancel);
}

#[test]
fn ping_roundtrip() {
    let frame = PingFrame {
        ack: false,
        payload: [1, 2, 3, 4, 5, 6, 7, 8],
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = PingFrame::parse(header, payload).unwrap();

    assert!(!parsed.ack);
    assert_eq!(parsed.payload, [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn goaway_roundtrip() {
    let frame = GoAwayFrame {
        last_stream_id: 7,
        error_code: leyline::h2::error::ErrorCode::NoError,
        debug_data: bytes::Bytes::from_static(b"bye"),
    };
    let mut buf = roundtrip_buf();
    frame.encode(&mut buf);

    let header = FrameHeader::parse(&buf[..9].try_into().unwrap());
    let payload = bytes::Bytes::copy_from_slice(&buf[9..]);
    let parsed = GoAwayFrame::parse(header, payload).unwrap();

    assert_eq!(parsed.last_stream_id, 7);
    assert_eq!(parsed.error_code, leyline::h2::error::ErrorCode::NoError);
    assert_eq!(&parsed.debug_data[..], b"bye");
}

#[test]
fn settings_rejects_nonzero_stream() {
    let header = FrameHeader {
        length: 0,
        frame_type: 0x4,
        flags: 0,
        stream_id: 1, // invalid
    };
    let result = SettingsFrame::parse(header, bytes::Bytes::new());
    assert!(result.is_err());
}

#[test]
fn data_rejects_stream_zero() {
    let header = FrameHeader {
        length: 0,
        frame_type: 0x0,
        flags: 0,
        stream_id: 0, // invalid
    };
    let result = DataFrame::parse(header, bytes::Bytes::new());
    assert!(result.is_err());
}

#[test]
fn window_update_rejects_zero_increment() {
    let header = FrameHeader {
        length: 4,
        frame_type: 0x8,
        flags: 0,
        stream_id: 1,
    };
    let payload = bytes::Bytes::from_static(&[0, 0, 0, 0]);
    let result = WindowUpdateFrame::parse(header, payload);
    assert!(result.is_err());
}
