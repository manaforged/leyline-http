use super::super::Session;
use crate::core::error::Error;
use crate::core::response::HttpVersion;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn plaintext_http_uses_h1_and_preserves_duplicate_headers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = socket.read(&mut tmp).await.unwrap();
            assert!(n > 0, "client closed before request headers");
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        let text = String::from_utf8_lossy(&req);
        assert!(text.starts_with("GET /wire?q=1 HTTP/1.1\r\n"), "{text}");
        assert!(text.contains(&format!("\r\nHost: {addr}\r\n")), "{text}");
        assert!(text.contains("\r\nUser-Agent: "), "{text}");
        assert!(text.contains("\r\nAccept-Encoding: "), "{text}");
        assert!(text.contains("\r\nConnection: keep-alive\r\n"), "{text}");
        let first = text.find("x-dup: one").unwrap();
        let second = text.find("x-dup: two").unwrap();
        assert!(first < second, "{text}");

        socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nset-cookie: a=1\r\nset-cookie: b=2\r\n\r\nok",
                )
                .await
                .unwrap();
    });

    let session = Session::chrome_latest().unwrap();
    let resp = session
        .get(&format!("http://{addr}/wire?q=1"))
        .append_header("x-dup", "one")
        .append_header("x-dup", "two")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.version(), HttpVersion::Http1_1);
    assert_eq!(resp.text(), "ok");
    assert_eq!(resp.header_all("set-cookie"), vec!["a=1", "b=2"]);
    server.await.unwrap();
}

/// Spin up a one-shot H1 mock and return the `Response` for inspection.
async fn one_shot_get(builder: super::super::SessionBuilder, path: &str) -> crate::core::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut tmp = [0u8; 1024];
        let mut req = Vec::new();
        loop {
            let n = socket.read(&mut tmp).await.unwrap();
            if n == 0 {
                break;
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .await
            .unwrap();
    });
    let session = builder.build().unwrap();
    let resp = session
        .get(&format!("http://{addr}{path}"))
        .send()
        .await
        .unwrap();
    server.await.unwrap();
    resp
}

#[tokio::test]
async fn audit_is_off_by_default() {
    let resp = one_shot_get(Session::builder(), "/x").await;
    // Default: no fingerprint introspection, no retained request headers.
    assert!(
        resp.audit().is_none(),
        "audit() should be None unless opted in"
    );
    assert!(
        resp.request_headers().is_empty(),
        "request headers should not be retained unless opted in"
    );
}

#[tokio::test]
async fn audit_opt_in_populates_fingerprints_and_headers() {
    let resp = one_shot_get(Session::builder().audit(true), "/x").await;
    let audit = resp
        .audit()
        .expect("audit() should be Some when .audit(true) is set");
    assert!(!audit.ja4.is_empty(), "JA4 should be populated");
    assert_eq!(
        audit.ja4h.split('_').count(),
        4,
        "JA4H shape: {}",
        audit.ja4h
    );
    assert!(
        resp.request_headers()
            .iter()
            .any(|(k, _)| k == "user-agent"),
        "request headers should be retained when audit is on"
    );
}

#[tokio::test]
async fn json_builder_returns_error_instead_of_panicking() {
    struct BadJson;

    impl serde::Serialize for BadJson {
        fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(serde::ser::Error::custom("intentional test error"))
        }
    }

    let session = Session::chrome_latest().unwrap();
    let err = session
        .post("http://127.0.0.1:9/no-network")
        .json(&BadJson)
        .send()
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Json(_)));
}
