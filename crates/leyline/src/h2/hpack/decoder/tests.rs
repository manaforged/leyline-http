use super::*;
use crate::h2::hpack::Encoder;

#[test]
fn decode_indexed() {
    let mut dec = Decoder::new();
    let headers = dec.decode_header_block(&[0x82]).unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].name, ":method");
    assert_eq!(headers[0].value, "GET");
}

#[test]
fn non_utf8_literal_preserves_raw_bytes_for_table_accounting() {
    let (value, consumed) = decode_string(&[0x02, 0xff, 0xfe]).expect("decodes, not error");
    assert_eq!(consumed, 3);
    assert_eq!(
        value.as_ref(),
        &[0xff, 0xfe],
        "raw wire bytes preserved verbatim"
    );
}

#[test]
fn non_utf8_indexed_value_keeps_table_in_lockstep_with_peer() {
    let mut dec = Decoder::new();
    let block = [0x40, 0x01, b'x', 0x02, 0xff, 0xfe];
    let headers = dec.decode_header_block(&block).expect("decodes");
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers[0].value.as_ref(),
        &[0xff, 0xfe],
        "raw value preserved"
    );
    assert_eq!(dec.dynamic.len(), 1);
    let (n, v) = table::lookup(table::STATIC_TABLE.len(), &dec.dynamic).expect("indexed");
    assert_eq!(n.as_ref(), b"x");
    assert_eq!(v.as_ref(), &[0xff, 0xfe]);
}

#[test]
fn decode_multiple_indexed() {
    let mut dec = Decoder::new();
    let headers = dec.decode_header_block(&[0x82, 0x84, 0x87]).unwrap();
    assert_eq!(headers.len(), 3);
    assert_eq!(
        headers[0],
        Header {
            name: ":method".into(),
            value: "GET".into()
        }
    );
    assert_eq!(
        headers[1],
        Header {
            name: ":path".into(),
            value: "/".into()
        }
    );
    assert_eq!(
        headers[2],
        Header {
            name: ":scheme".into(),
            value: "https".into()
        }
    );
}

#[test]
fn encoder_decoder_roundtrip() {
    let mut enc = Encoder::new();
    let mut dec = Decoder::new();

    let original = vec![
        (":method", "GET"),
        (":path", "/api/all"),
        (":scheme", "https"),
        (":authority", "example.com"),
        ("user-agent", "Mozilla/5.0"),
        ("accept", "text/html"),
    ];

    let encoded = enc.encode_header_block(&original);
    let decoded = dec.decode_header_block(&encoded).unwrap();

    assert_eq!(decoded.len(), original.len());
    for (i, &(name, value)) in original.iter().enumerate() {
        assert_eq!(decoded[i].name, name, "header {i} name mismatch");
        assert_eq!(decoded[i].value, value, "header {i} value mismatch");
    }
}

#[test]
fn dynamic_table_roundtrip() {
    let mut enc = Encoder::new();
    let mut dec = Decoder::new();

    let block1 = enc.encode_header_block(&[("x-request-id", "abc123")]);
    let headers1 = dec.decode_header_block(&block1).unwrap();
    assert_eq!(headers1[0].name, "x-request-id");
    assert_eq!(headers1[0].value, "abc123");

    let block2 = enc.encode_header_block(&[("x-request-id", "def456")]);
    let headers2 = dec.decode_header_block(&block2).unwrap();
    assert_eq!(headers2[0].name, "x-request-id");
    assert_eq!(headers2[0].value, "def456");

    assert!(
        block2.len() < block1.len(),
        "second block should be shorter due to dynamic table"
    );
}

#[test]
fn table_size_update() {
    let mut enc = Encoder::new();
    let mut dec = Decoder::new();

    enc.set_max_table_size(0);
    let block = enc.encode_header_block(&[(":method", "GET")]);

    let headers = dec.decode_header_block(&block).unwrap();
    assert_eq!(headers[0].name, ":method");
    assert_eq!(headers[0].value, "GET");
}

#[test]
fn invalid_index_rejected() {
    let mut dec = Decoder::new();
    let result = dec.decode_header_block(&[0xFF, 0x49]);
    result.expect_err("expected Err");
}

#[test]
fn a_table_size_update_after_a_header_is_rejected() {
    let mut dec = Decoder::new();
    dec.decode_header_block(&[0x88, 0x20])
        .expect_err("expected Err");
}

#[test]
fn a_third_table_size_update_in_one_block_is_rejected() {
    let mut dec = Decoder::new();
    dec.decode_header_block(&[0x20, 0x20, 0x20, 0x88])
        .expect_err("expected Err");
}

#[test]
fn two_table_size_updates_at_the_block_start_are_accepted() {
    let mut dec = Decoder::new();
    let headers = dec
        .decode_header_block(&[0x20, 0x3f, 0xe1, 0x1f, 0x88])
        .unwrap();
    assert_eq!(headers.len(), 1);
}
