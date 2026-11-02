use super::*;

#[test]
fn encode_indexed_method_get() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":method", "GET")]);
    assert_eq!(block, vec![0x82]);
}

#[test]
fn encode_indexed_path_root() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":path", "/")]);
    assert_eq!(block, vec![0x84]);
}

#[test]
fn encode_indexed_scheme_https() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[(":scheme", "https")]);
    assert_eq!(block, vec![0x87]);
}

#[test]
fn encode_literal_with_indexing() {
    let mut enc = Encoder::new();
    let block = enc.encode_header_block(&[("custom-key", "custom-value")]);
    assert_eq!(block[0], 0x40);
    assert!(!block.is_empty());
    assert_eq!(enc.dynamic.len(), 1);
}

#[test]
fn second_request_uses_dynamic_table() {
    let mut enc = Encoder::new();

    let _block1 = enc.encode_header_block(&[("x-custom", "value1")]);
    assert_eq!(enc.dynamic.len(), 1);

    let block2 = enc.encode_header_block(&[("x-custom", "value2")]);
    assert_ne!(block2[0], 0x40);
}

#[test]
fn uppercase_name_encoded_as_lowercase() {
    let mut mixed = Encoder::new();
    let mixed_block = mixed.encode_header_block(&[("X-Extra", "val")]);

    let mut lower = Encoder::new();
    let lower_block = lower.encode_header_block(&[("x-extra", "val")]);

    assert_eq!(mixed_block, lower_block);
    assert_eq!(mixed.dynamic.get(0).unwrap().0, "x-extra");
}

#[test]
fn blocks_round_trip_while_the_table_evicts() {
    let mut enc = Encoder::new();
    enc.set_max_table_size(256);
    let mut dec = super::super::decoder::Decoder::new();
    for round in 0..200usize {
        let path = format!("/item/{}", round % 7);
        let tag = format!("v{}", round % 13);
        let headers = [
            (":method", "GET"),
            (":path", path.as_str()),
            ("x-tag", tag.as_str()),
            ("x-tag", "fixed"),
            ("user-agent", "agent"),
        ];
        let block = enc.encode_header_block(&headers);
        let decoded: Vec<(String, String)> = dec
            .decode_header_block(&block)
            .unwrap()
            .into_iter()
            .map(|h| {
                (
                    String::from_utf8(h.name.to_vec()).unwrap(),
                    String::from_utf8(h.value.to_vec()).unwrap(),
                )
            })
            .collect();
        let expected: Vec<(String, String)> = headers
            .iter()
            .map(|&(n, v)| (n.to_owned(), v.to_owned()))
            .collect();
        assert_eq!(decoded, expected, "round {round}");
    }
    let repeated = [("x-a", "1"), ("x-b", "2")];
    enc.encode_header_block(&repeated);
    assert_eq!(enc.encode_header_block(&repeated).len(), repeated.len());
}

#[test]
fn header_blocks_reuse_one_buffer() {
    let mut enc = Encoder::new();
    let headers = [(":method", "GET"), ("user-agent", "leyline")];
    let first = enc.encode_block(headers.iter().copied());
    let (at, first) = (first.as_ptr(), first.to_vec());
    let second = enc.encode_block(headers.iter().copied());
    assert_eq!(second.as_ptr(), at);
    assert_eq!(first[0], 0x82);
    assert_eq!(second, [0x82, 0xBE]);
}

#[test]
fn remembered_headers_decode_after_evictions() {
    let mut enc = Encoder::new();
    let mut dec = super::super::decoder::Decoder::new();
    enc.set_max_table_size(96);
    dec.set_max_table_size(96);
    let blocks: [&[(&str, &str)]; 6] = [
        &[(":method", "GET"), ("x-a", "one"), ("x-b", "two")],
        &[(":method", "GET"), ("x-a", "one"), ("x-b", "two")],
        &[(":method", "GET"), ("x-a", "three"), ("x-b", "two")],
        &[(":method", "GET"), ("x-a", "one"), ("x-b", "two")],
        &[(":method", "POST"), ("cookie", "c=1"), ("x-b", "two")],
        &[(":method", "POST"), ("cookie", "c=1"), ("x-b", "two")],
    ];
    for headers in blocks {
        let block = enc.encode_header_block(headers);
        let mut decoded = Vec::new();
        dec.decode_block(&block, |n, v, _| {
            decoded.push((
                String::from_utf8(n.to_vec()).unwrap(),
                String::from_utf8(v.to_vec()).unwrap(),
            ));
        })
        .unwrap();
        let expected: Vec<_> = headers
            .iter()
            .map(|&(n, v)| (n.to_string(), v.to_string()))
            .collect();
        assert_eq!(decoded, expected);
        assert!(
            !block.contains(&(0x80 | 32)),
            "cookie must never be indexed"
        );
    }
}
