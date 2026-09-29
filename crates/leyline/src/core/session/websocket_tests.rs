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
fn headers_append_every_value() {
    let builder = Session::new()
        .websocket("wss://example.com/socket")
        .headers([("a", "1")])
        .headers([("a", "2"), ("b", "3")]);
    let headers = builder.headers.unwrap();
    let sent: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.to_str().unwrap()))
        .collect();
    assert_eq!(sent, [("a", "1"), ("a", "2"), ("b", "3")]);
}

#[tokio::test]
async fn an_invalid_header_fails_before_the_connection_opens() {
    let session = Session::builder()
        .proxy(crate::ProxyConfig::new().env(false))
        .build()
        .unwrap();
    let err = session
        .websocket("wss://127.0.0.1:9/socket")
        .header("bad name", "value")
        .connect()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), crate::Kind::Request, "{err:?}");
}
