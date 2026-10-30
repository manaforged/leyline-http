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

#[tokio::test]
async fn an_invalid_header_fails_before_the_connection_opens() {
    let session = Session::builder()
        .proxy(crate::Proxy::new().env(false))
        .build()
        .unwrap();
    let err = session
        .get("wss://127.0.0.1:9/socket")
        .header("bad name", "value")
        .websocket()
        .await
        .unwrap_err();
    assert_eq!(err.cause(), crate::Cause::Request, "{err:?}");
}
