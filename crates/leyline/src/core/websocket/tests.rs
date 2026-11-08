use super::handshake::{forwards, overlay_headers, tungstenite_config, ws_header_pair};
use crate::core::config::WebSocketOptions;
use crate::wire::HeaderPair;
use http::Version;

#[test]
fn reserved_headers_gate_the_handshake_but_forwardable_pass() {
    for version in [Version::HTTP_11, Version::HTTP_2] {
        reserved_and_forwardable(version);
    }
}

fn reserved_and_forwardable(version: Version) {
    for h in [
        "Host",
        "connection",
        "Upgrade",
        "Sec-WebSocket-Key",
        "SEC-WEBSOCKET-VERSION",
        "sec-websocket-extensions",
        "Content-Length",
    ] {
        assert!(!forwards(version, h, "x"), "{h} must be reserved");
    }
    for h in [
        "cookie",
        "Authorization",
        "Origin",
        "User-Agent",
        "Sec-WebSocket-Protocol",
    ] {
        assert!(forwards(version, h, "x"), "{h} must be forwardable");
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
    ws_header_pair("x-request-id", "1").expect("expected Ok");
}

#[test]
fn websocket_config_limits_are_applied() {
    let cfg = WebSocketOptions {
        max_frame_size: Some(4096),
        max_message_size: Some(8192),
        read_buffer_size: Some(1024),
        ..WebSocketOptions::default()
    };
    let mapped = tungstenite_config(&cfg);
    assert_eq!(mapped.max_frame_size, Some(4096));
    assert_eq!(mapped.max_message_size, Some(8192));
    assert_eq!(mapped.read_buffer_size, 1024);
}

#[test]
fn caller_headers_replace_only_the_handshake_user_agent_and_origin() {
    let mut request: Vec<HeaderPair> = vec![
        ("Sec-WebSocket-Version".into(), "13".into()),
        ("User-Agent".into(), "default".into()),
        ("Origin".into(), "https://example.com".into()),
    ];
    let extra = [
        ("user-agent".to_owned(), "first".to_owned()),
        ("user-agent".to_owned(), "second".to_owned()),
        ("sec-websocket-version".to_owned(), "8".to_owned()),
        ("x-trace".to_owned(), "1".to_owned()),
        ("x-trace".to_owned(), "2".to_owned()),
    ];
    overlay_headers(&mut request, &extra, Version::HTTP_2);
    let sent: Vec<(&str, &str)> = request
        .iter()
        .map(|(name, value)| (name.as_ref(), value.as_ref()))
        .collect();
    assert_eq!(
        sent,
        [
            ("Sec-WebSocket-Version", "13"),
            ("User-Agent", "second"),
            ("Origin", "https://example.com"),
            ("x-trace", "1"),
            ("x-trace", "2"),
        ]
    );
}
