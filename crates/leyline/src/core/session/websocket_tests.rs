use super::ws_origin;
use crate::Session;

#[test]
fn origin_keeps_explicit_port() {
    assert_eq!(
        ws_origin("wss://example.com:8443/socket").unwrap(),
        "https://example.com:8443"
    );
}

#[test]
fn origin_omits_default_port() {
    assert_eq!(
        ws_origin("wss://example.com/socket").unwrap(),
        "https://example.com"
    );
}

#[test]
fn origin_rejects_http_scheme() {
    let err = ws_origin("https://example.com/socket").unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("wss://") && message.contains("https"),
        "unexpected error: {message}"
    );
}

#[test]
fn origin_rejects_plaintext_ws() {
    let err = ws_origin("ws://example.com/socket").unwrap_err();
    let message = err.to_string();
    assert!(message.contains("wss://"), "unexpected error: {message}");
}

#[test]
fn headers_set_same_name_and_keep_distinct() {
    let builder = Session::new()
        .websocket("wss://example.com/socket")
        .headers([("a", "1")])
        .headers([("a", "2"), ("b", "3")]);
    assert_eq!(
        builder.headers,
        vec![("a".into(), "2".into()), ("b".into(), "3".into())]
    );
}
