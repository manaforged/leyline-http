use hex::encode;
use leyline::internals::h2::hpack::Encoder;

#[test]
fn rfc7541_huffman_requests() {
    let mut encoder = Encoder::new();
    let first = [
        (":method", "GET"),
        (":scheme", "http"),
        (":path", "/"),
        (":authority", "www.example.com"),
    ];
    assert_eq!(
        encode(encoder.encode_header_block(&first)),
        "828684418cf1e3c2e5f23a6ba0ab90f4ff"
    );
    let second = [
        (":method", "GET"),
        (":scheme", "http"),
        (":path", "/"),
        (":authority", "www.example.com"),
        ("cache-control", "no-cache"),
    ];
    assert_eq!(
        encode(encoder.encode_header_block(&second)),
        "828684be5886a8eb10649cbf"
    );
    let third = [
        (":method", "GET"),
        (":scheme", "https"),
        (":path", "/index.html"),
        (":authority", "www.example.com"),
        ("custom-key", "custom-value"),
    ];
    assert_eq!(
        encode(encoder.encode_header_block(&third)),
        "828785bf408825a849e95ba97d7f8925a849e95bb8e8b4bf"
    );
}

#[test]
fn repeated_sensitive_headers_are_never_indexed() {
    let mut encoder = Encoder::new();
    let headers = [("x-public", "value")];
    assert_eq!(encoder.encode_header_block(&headers)[0], 0x40);
    for name in [
        "authorization",
        "cookie",
        "set-cookie",
        "proxy-authorization",
    ] {
        for _ in 0..2 {
            let block = encoder.encode_header_block(&[(name, "private")]);
            assert_eq!(block[0] & 0xf0, 0x10);
        }
    }
    assert_eq!(encoder.encode_header_block(&headers), [0xbe]);
}

#[test]
fn table_resize_removes_cached_static_names() {
    let mut encoder = Encoder::new();
    let headers = [("user-agent", "test")];
    assert_eq!(encoder.encode_header_block(&headers)[0], 0x7a);
    assert_eq!(encoder.encode_header_block(&headers), [0xbe]);
    encoder.set_max_table_size(0);
    let block = encoder.encode_header_block(&headers);
    assert_eq!(&block[..2], &[0x20, 0x7a]);
    assert_eq!(encoder.encode_header_block(&headers)[0], 0x7a);
}

#[test]
fn multiple_table_resizes_emit_minimum_before_final() {
    let mut encoder = Encoder::new();
    let headers = [("x-custom", "value")];
    let _ = encoder.encode_header_block(&headers);
    encoder.set_max_table_size(0);
    encoder.set_max_table_size(4096);
    let block = encoder.encode_header_block(&headers);
    assert_eq!(block[0], 0x20);
    assert_eq!(block[1], 0x3f);
    assert_eq!(block[2], 0xe1);
    assert_eq!(block[3], 0x1f);
}
