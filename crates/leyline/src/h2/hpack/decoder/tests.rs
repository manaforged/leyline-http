use super::*;
use crate::h2::hpack::Encoder;

#[test]
fn decode_indexed() {
    let mut dec = Decoder::new();
    // 0x82 = indexed, index 2 = :method GET
    let headers = dec.decode_header_block(&[0x82]).unwrap();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].name, ":method");
    assert_eq!(headers[0].value, "GET");
}

#[test]
fn non_utf8_literal_preserves_raw_bytes_for_table_accounting() {
    // A raw string literal carrying non-UTF-8 obs-text must decode to the
    // ORIGINAL bytes (no U+FFFD substitution here): the dynamic table sizes
    // entries by true octet length, so any in-decoder coercion would diverge
    // our table from the peer's. The lossy &str view happens later, at the
    // HeaderStr boundary.
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
    // Eviction-boundary check for the lossy-decode fix: a non-UTF-8 value
    // inserted with incremental indexing must size the dynamic table by its
    // RAW octet length (2 bytes here), not a U+FFFD-expanded length (which
    // would be 6 bytes for two replacement chars) — otherwise our table
    // evicts on a different schedule than the encoder's and later indices
    // resolve wrong. Encode a literal-with-incremental-indexing field whose
    // value is the 2 raw octets 0xFF 0xFE under a literal name.
    let mut dec = Decoder::new();
    // 0x40 = literal w/ incremental indexing, name index 0 (literal name).
    // name: len 1 "x"; value: len 2, raw 0xFF 0xFE.
    let block = [0x40, 0x01, b'x', 0x02, 0xff, 0xfe];
    let headers = dec.decode_header_block(&block).expect("decodes");
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers[0].value.as_ref(),
        &[0xff, 0xfe],
        "raw value preserved"
    );
    // Entry size = name.len(1) + value.len(2) + 32 = 35 — computed from the
    // raw octets, matching what the peer's encoder accounted for.
    assert_eq!(dec.dynamic.size(), 1 + 2 + 32);
    // And the indexed entry round-trips back to the same raw bytes. The
    // newest dynamic entry is HPACK index `STATIC_TABLE.len()`.
    let (n, v) = table::lookup(table::STATIC_TABLE.len(), &dec.dynamic).expect("indexed");
    assert_eq!(n.as_ref(), b"x");
    assert_eq!(v.as_ref(), &[0xff, 0xfe]);
}

#[test]
fn decode_multiple_indexed() {
    let mut dec = Decoder::new();
    // :method GET, :path /, :scheme https
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

    // First request: custom header gets added to dynamic table.
    let block1 = enc.encode_header_block(&[("x-request-id", "abc123")]);
    let headers1 = dec.decode_header_block(&block1).unwrap();
    assert_eq!(headers1[0].name, "x-request-id");
    assert_eq!(headers1[0].value, "abc123");

    // Second request: same header name, different value.
    // Encoder should use indexed name from dynamic table.
    let block2 = enc.encode_header_block(&[("x-request-id", "def456")]);
    let headers2 = dec.decode_header_block(&block2).unwrap();
    assert_eq!(headers2[0].name, "x-request-id");
    assert_eq!(headers2[0].value, "def456");

    // The second block should be shorter (name is indexed).
    assert!(
        block2.len() < block1.len(),
        "second block should be shorter due to dynamic table"
    );
}

#[test]
fn table_size_update() {
    let mut enc = Encoder::new();
    let mut dec = Decoder::new();

    enc.set_max_table_size(0); // clear dynamic table
    let block = enc.encode_header_block(&[(":method", "GET")]);

    // Should start with a size update, then indexed header.
    let headers = dec.decode_header_block(&block).unwrap();
    assert_eq!(headers[0].name, ":method");
    assert_eq!(headers[0].value, "GET");
}

#[test]
fn invalid_index_rejected() {
    let mut dec = Decoder::new();
    // Index 200 doesn't exist in static or dynamic table.
    // 0x80 | prefix_max = 0xFF means index >= 127, then continuation byte.
    let result = dec.decode_header_block(&[0xFF, 0x49]); // index = 127 + 73 = 200
    assert!(result.is_err());
}
