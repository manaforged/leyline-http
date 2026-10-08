use super::validate_connect_response;

fn buf(s: &str) -> (Vec<u8>, usize) {
    let b = s.as_bytes().to_vec();
    let end = b.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    (b, end)
}

#[test]
fn accepts_minimal_200() {
    let (b, e) = buf("HTTP/1.1 200 OK\r\n\r\n");
    validate_connect_response(&b, e).expect("expected Ok");
}

#[test]
fn accepts_http10_200_with_reason() {
    let (b, e) = buf("HTTP/1.0 200 Connection established\r\n\r\n");
    validate_connect_response(&b, e).expect("expected Ok");
}

#[test]
fn accepts_200_with_benign_headers() {
    let (b, e) = buf("HTTP/1.1 200 OK\r\nVia: 1.1 proxy\r\nX-Foo: bar\r\n\r\n");
    validate_connect_response(&b, e).expect("expected Ok");
}

#[test]
fn rejects_200_without_space() {
    let (b, e) = buf("HTTP/1.1 200OK\r\n\r\n");
    let err = validate_connect_response(&b, e).unwrap_err();
    assert!(format!("{err}").contains("proxy CONNECT failed"));
}

#[test]
fn rejects_2000_code() {
    let (b, e) = buf("HTTP/1.1 2000 OK\r\n\r\n");
    let err = validate_connect_response(&b, e).unwrap_err();
    assert!(format!("{err}").contains("proxy CONNECT failed"));
}

#[test]
fn rejects_unsupported_http_version() {
    let (b, e) = buf("HTTP/2.0 200 OK\r\n\r\n");
    let err = validate_connect_response(&b, e).unwrap_err();
    assert!(format!("{err}").contains("proxy CONNECT failed"));
}

#[test]
fn rejects_non_200_status() {
    let (b, e) = buf("HTTP/1.1 407 Proxy Authentication Required\r\n\r\n");
    let err = validate_connect_response(&b, e).unwrap_err();
    assert!(format!("{err}").contains("proxy CONNECT failed"));
}

#[test]
fn non_200_with_trailing_body_reports_status_not_injection() {
    let full = b"HTTP/1.1 407 Proxy Authentication Required\r\n\
                     Content-Length: 9\r\n\r\nforbidden"
        .to_vec();
    let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let err = validate_connect_response(&full, end).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("proxy CONNECT failed") && msg.contains("407"),
        "expected status-line error, got: {msg}"
    );
    assert!(
        !msg.contains("trailing bytes"),
        "trailing-bytes rule must not fire on non-200: {msg}"
    );
}

#[test]
fn rejects_content_length_on_2xx() {
    let (b, e) = buf("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let err = validate_connect_response(&b, e).unwrap_err();
    assert!(format!("{err}").contains("forbidden framing header"));
}

#[test]
fn rejects_transfer_encoding_on_2xx() {
    let (b, e) = buf("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
    let err = validate_connect_response(&b, e).unwrap_err();
    assert!(format!("{err}").contains("forbidden framing header"));
}

#[test]
fn rejects_mixed_case_framing_headers() {
    let (b, e) = buf("HTTP/1.1 200 OK\r\ncONTENT-lENGTH: 0\r\n\r\n");
    let err = validate_connect_response(&b, e).unwrap_err();
    assert!(format!("{err}").contains("forbidden framing header"));
}

#[test]
fn rejects_trailing_bytes_after_terminator() {
    let full = b"HTTP/1.1 200 OK\r\n\r\nLEAKED".to_vec();
    let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let err = validate_connect_response(&full, end).unwrap_err();
    assert!(format!("{err}").contains("trailing bytes"));
}

#[test]
fn rejects_non_utf8_body() {
    let mut full = b"HTTP/1.1 200 OK\r\nX-Evil: ".to_vec();
    full.extend_from_slice(&[0xFF, 0xFE, 0x80]);
    full.extend_from_slice(b"\r\n\r\n");
    let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let err = validate_connect_response(&full, end).unwrap_err();
    assert!(format!("{err}").contains("not UTF-8"));
}

#[test]
fn rejects_end_idx_past_buf_len() {
    let b = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    let err = validate_connect_response(&b, b.len() + 1).unwrap_err();
    assert!(format!("{err}").contains("out-of-contract"));
}

#[test]
fn rejects_zero_end_idx() {
    let err = validate_connect_response(b"HTTP/1.1 200 OK\r\n\r\n", 0).unwrap_err();
    assert!(format!("{err}").contains("out-of-contract"));
}

#[test]
fn rejects_tiny_end_idx_below_terminator_len() {
    let err = validate_connect_response(b"HTT", 3).unwrap_err();
    assert!(format!("{err}").contains("out-of-contract"));
}
