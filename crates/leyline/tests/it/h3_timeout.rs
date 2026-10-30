#![cfg(feature = "http3")]
use std::time::Duration;

use leyline::{Browser, Protocol, Session, Timeout};

#[tokio::test]
async fn an_http3_handshake_timeout_is_a_timeout() {
    let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .protocol(Protocol::Http3)
        .timeout(
            Timeout::new()
                .connect(Duration::from_millis(500))
                .total(Duration::from_secs(10)),
        )
        .retry(0)
        .build()
        .unwrap();
    let err = session
        .get(format!("https://127.0.0.1:{port}/"))
        .send()
        .await
        .unwrap_err();
    assert!(err.kind() == leyline::Kind::Timeout, "{err:?}");
    drop(silent);
}
