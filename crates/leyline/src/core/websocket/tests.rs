use super::{WsConnection, is_reserved_ws_header, tungstenite_config, ws_header_pair};
use crate::core::WebSocketConfig;
use crate::core::error::{Error, Kind};

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

#[test]
fn only_missing_connect_protocol_falls_back() {
    assert!(WsConnection::is_h2_fallback_trigger(
        &Error::new(Kind::Request).with_message("h2-no-connect-protocol")
    ));
    assert!(!WsConnection::is_h2_fallback_trigger(
        &Error::new(Kind::Request).with_message("peer reset")
    ));
}

#[test]
fn websocket_config_limits_are_applied() {
    let cfg = WebSocketConfig {
        max_frame_size: Some(4096),
        max_message_size: Some(8192),
        read_buffer_size: Some(1024),
        ..WebSocketConfig::default()
    };
    let mapped = tungstenite_config(&cfg);
    assert_eq!(mapped.max_frame_size, Some(4096));
    assert_eq!(mapped.max_message_size, Some(8192));
    assert_eq!(mapped.read_buffer_size, 1024);
}
