use super::{chunk_size_value, trim_end, trim_start};

#[test]
fn trim_ows_only_strips_rfc9110_sp_and_htab() {
    assert_eq!(trim_start(b""), b"");
    assert_eq!(trim_end(b""), b"");
    assert_eq!(trim_start(b" \t "), b"");
    assert_eq!(trim_end(b" \t "), b"");
    assert_eq!(trim_start(b" \t value"), b"value");
    assert_eq!(trim_end(b"value \t "), b"value");
    assert_eq!(trim_end(b"ab   "), b"ab");
    assert_eq!(trim_start(b"\x0bvalue"), b"\x0bvalue");
    assert_eq!(trim_end(b"value\x0b"), b"value\x0b");
    assert_eq!(trim_start(b"\x0cvalue"), b"\x0cvalue");
    assert_eq!(trim_end(b"value\x0c"), b"value\x0c");

    let (_, headers, _) = super::parse_h1_head(
        b"HTTP/1.1 200 OK\r\nX-Trailing: bar \t \r\nContent-Length: 0\r\n\r\n",
    )
    .expect("valid h1 head");
    assert_eq!(
        headers.get("x-trailing").map(|v| v.as_bytes()),
        Some(b"bar".as_slice())
    );
}

#[test]
fn chunk_size_overflow_is_exact_at_u64_max() {
    const HEX: &[u8] = b"0123456789abcdefABCDEF";
    let mut line = [b'f'; 17];
    for &first in HEX {
        for &last in HEX {
            line[0] = first;
            line[16] = last;
            if first == b'0' {
                let low = u64::from(char::from(last).to_digit(16).expect("hex"));
                assert_eq!(chunk_size_value(&line), Some(0xffff_ffff_ffff_fff0 | low));
            } else {
                assert_eq!(chunk_size_value(&line), None);
            }
        }
    }
}
