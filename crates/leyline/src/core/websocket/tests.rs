use super::{is_reserved_ws_header, ws_header_pair};

#[test]
fn reserved_headers_gate_the_handshake_but_forwardable_pass() {
    for h in [
        "Host",
        "connection",
        "Upgrade",
        "Sec-WebSocket-Key",
        "SEC-WEBSOCKET-VERSION",
        "sec-websocket-extensions",
        "Content-Length",
    ] {
        assert!(is_reserved_ws_header(h), "{h} must be reserved");
    }
    for h in [
        "cookie",
        "Authorization",
        "Origin",
        "User-Agent",
        "Sec-WebSocket-Protocol",
    ] {
        assert!(!is_reserved_ws_header(h), "{h} must be forwardable");
    }
}

#[test]
fn invalid_header_name_is_err() {
    let err = ws_header_pair("bad name", "x").unwrap_err();
    assert!(err.to_string().contains("header name"), "unexpected: {err}");
}

#[test]
fn invalid_header_value_is_err() {
    let err = ws_header_pair("x-foo", "a\nb").unwrap_err();
    assert!(
        err.to_string().contains("header value"),
        "unexpected: {err}"
    );
}

#[test]
fn valid_header_pair_ok() {
    assert!(ws_header_pair("x-request-id", "1").is_ok());
}
