#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use leyline::pool::{H1Body, H1PooledError, H1Target, Pool, send_request_h1_pooled};
use leyline::profile::{Browser, Platform, ProfileRegistry};
use leyline::tls::FingerprintConnector;
use std::sync::Arc;

fn connector() -> FingerprintConnector {
    let profiles = ProfileRegistry::builtin();
    let profile = profiles.get_browser(Browser::Chrome147).unwrap();
    let tcp_profile = Platform::default().tcp_profile();
    FingerprintConnector::new(profile, tcp_profile).unwrap()
}

const UNROUTABLE_URL: &str = "http://127.0.0.1:1/";

async fn short_exchange(
    method: &str,
    headers: Vec<(String, String)>,
) -> Result<leyline::pool::H1Response, H1PooledError> {
    let pool = Arc::new(Pool::new());
    let connector = connector();
    let url = url::Url::parse(UNROUTABLE_URL).unwrap();
    send_request_h1_pooled(
        &pool,
        &connector,
        "http",
        "127.0.0.1",
        1,
        method,
        &url,
        headers,
        H1Body::Empty,
        None,
        H1Target::OriginForm,
        false,
    )
    .await
}

#[tokio::test]
async fn method_with_crlf_rejected_before_wire() {
    let err = short_exchange("GET\r\nInjected: yes", vec![])
        .await
        .err()
        .expect("must reject before reaching the wire");
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid HTTP method"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn header_value_with_crlf_rejected_before_wire() {
    let err = short_exchange(
        "GET",
        vec![(
            "X-Ok".into(),
            "legit\r\nHost: attacker.example\r\nCookie: stolen".into(),
        )],
    )
    .await
    .err()
    .expect("must reject before reaching the wire");
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid value for header"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn header_name_with_colon_rejected_before_wire() {
    let err = short_exchange("GET", vec![(": evil".into(), "1".into())])
        .await
        .err()
        .expect("must reject before reaching the wire");
    let msg = format!("{err}");
    assert!(
        msg.contains("invalid header name"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn header_value_with_null_rejected() {
    let err = short_exchange("GET", vec![("X-Ok".into(), "a\0b".into())])
        .await
        .err()
        .expect("must reject before reaching the wire");
    assert!(format!("{err}").contains("invalid value for header"));
}
