use super::*;

#[test]
fn encode_www_example_com() {
    let mut dst = Vec::new();
    encode(b"www.example.com", &mut dst);
    assert_eq!(
        dst,
        vec![
            0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab, 0x90, 0xf4, 0xff
        ]
    );
}

#[test]
fn decode_www_example_com() {
    let encoded = vec![
        0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab, 0x90, 0xf4, 0xff,
    ];
    let decoded = decode(&encoded).unwrap();
    assert_eq!(decoded, b"www.example.com");
}

#[test]
fn roundtrip_ascii() {
    for s in &[
        b"hello" as &[u8],
        b"GET",
        b"/index.html",
        b"Mozilla/5.0",
        b"text/html",
        b"application/json",
        b"gzip, deflate, br",
    ] {
        let mut encoded = Vec::new();
        encode(s, &mut encoded);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(
            &decoded,
            s,
            "roundtrip failed for {:?}",
            std::str::from_utf8(s)
        );
    }
}

#[test]
fn roundtrip_all_bytes() {
    for byte in 0u8..=255 {
        let input = [byte];
        let mut encoded = Vec::new();
        encode(&input, &mut encoded);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, input, "roundtrip failed for byte {byte}");
    }
}

#[test]
fn roundtrip_all_byte_pairs() {
    let mut encoded = Vec::new();
    for a in 0u8..=255 {
        for b in 0u8..=255 {
            encoded.clear();
            let input = [a, b];
            encode(&input, &mut encoded);
            let decoded = decode(&encoded).unwrap();
            assert_eq!(decoded, input, "roundtrip failed for [{a}, {b}]");
        }
    }
}

#[test]
fn roundtrip_realistic_headers() {
    for s in &[
            b"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36" as &[u8],
            b"text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
            b"https://www.example.com/path/to/resource?query=value&other=thing",
            b"gzip, deflate, br, zstd",
        ] {
            let mut encoded = Vec::new();
            encode(s, &mut encoded);
            let decoded = decode(&encoded).unwrap();
            assert_eq!(&decoded, s, "roundtrip failed for {:?}", std::str::from_utf8(s));
        }
}

#[test]
fn roundtrip_control_chars() {
    let input = b"\r\n\r\n";
    let mut encoded = Vec::new();
    encode(input, &mut encoded);
    let decoded = decode(&encoded).unwrap();
    assert_eq!(decoded, input);
}

#[test]
fn encoded_len_shorter_than_raw() {
    let s = b"www.example.com";
    assert!(encoded_len(s) < s.len());
}

#[test]
fn empty_input() {
    let mut encoded = Vec::new();
    encode(b"", &mut encoded);
    assert!(encoded.is_empty());

    let decoded = decode(&encoded).unwrap();
    assert!(decoded.is_empty());
}

#[test]
fn rejects_bad_padding() {
    let result = decode(&[0x00]);
    result.expect_err("expected Err");
}

#[test]
fn decode_table_covers_all_symbols() {
    let table = decode_table();
    let total_symbols: usize = table.by_length.iter().map(|b| b.len()).sum();
    assert_eq!(
        total_symbols, 257,
        "decode table must cover all 257 symbols"
    );
}
