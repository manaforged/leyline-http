use super::is_reserved_ws_header;

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
