use super::super::{Session, SessionBuilder};
use crate::core::error::Kind;
use crate::core::response::{HttpVersion, Response};
use crate::{Body, ContentEncoding};
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

    let session = Session::new();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/wire?q=1"))
        .header("x-dup", "one")
        .header("x-dup", "two")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.version(), HttpVersion::Http1_1);
    assert_eq!(
        resp.headers()
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>(),
        vec!["a=1", "b=2"]
    );
    assert_eq!(resp.text().await.unwrap(), "ok");
    server.await.unwrap();
}

#[tokio::test]
async fn owned() {
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
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .await
            .unwrap();
    });
    let req = http::Request::builder()
        .method(http::Method::GET)
        .uri(format!("http://{addr}/owned"))
        .body(Body::from(Vec::new()))
        .unwrap();
    let resp = Session::new().execute(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
    server.await.unwrap();
}

async fn one_shot_get(builder: SessionBuilder, path: &str) -> Response {
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
    let resp = session.get(format!("http://{addr}{path}")).await.unwrap();
    server.await.unwrap();
    resp
}

#[tokio::test]
async fn audit_is_off_by_default() {
    let resp = one_shot_get(Session::builder(), "/x").await;
    assert!(
        resp.audit().is_none(),
        "audit() should be None unless opted in"
    );
    assert!(
        resp.request_headers().next().is_none(),
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
        resp.request_headers().any(|(k, _)| k == "user-agent"),
        "request headers should be retained when audit is on"
    );
}

#[cfg(feature = "compression-gzip")]
#[tokio::test]
async fn compress_sets_header_and_puts_compressed_bytes_on_the_wire() {
    use std::io::Read;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let (buf, body_start, content_len) = loop {
            let n = socket.read(&mut tmp).await.unwrap();
            assert!(n > 0, "client closed before full request");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(hdr_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..hdr_end]).to_lowercase();
                let content_len: usize = head
                    .split("\r\n")
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .map(|v| v.trim().parse().unwrap())
                    .expect("content-length header present");
                let body_start = hdr_end + 4;
                if buf.len() >= body_start + content_len {
                    break (buf, body_start, content_len);
                }
            }
        };

        let head = String::from_utf8_lossy(&buf[..body_start]).to_lowercase();
        assert!(
            head.contains("content-encoding: gzip"),
            "compressed request must declare content-encoding: gzip:\n{head}"
        );

        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(&buf[body_start..body_start + content_len])
            .read_to_end(&mut decoded)
            .unwrap();

        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .await
            .unwrap();
        decoded
    });

    let payload = b"the quick brown fox jumps over the lazy dog. ".repeat(64);
    let resp = Session::new()
        .post(format!("http://{addr}/upload"))
        .body(payload.clone())
        .compress(ContentEncoding::Gzip)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let decoded = server.await.unwrap();
    assert_eq!(
        decoded, payload,
        "server must decode back to the original body"
    );
}

#[tokio::test]
async fn compress_rejects_streaming_body() {
    let body = Body::stream(
        futures_util::stream::iter(vec![Ok::<_, std::io::Error>(bytes::Bytes::from_static(
            b"chunk",
        ))]),
        None,
    );
    let err = Session::new()
        .post("http://127.0.0.1:9/x")
        .body(body)
        .compress(ContentEncoding::Gzip)
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "got: {err:?}");
}

#[tokio::test]
async fn unsupported_scheme_proxy_is_refused_not_sent_in_cleartext() {
    let err = Session::new()
        .request(http::Method::GET, "https://example.test/")
        .proxy("ftp://user:secret@127.0.0.1:1")
        .send()
        .await
        .unwrap_err();
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("ftp") && msg.contains("unsupported proxy scheme"),
        "expected an unsupported-scheme cleartext-refusal error, got: {msg}"
    );
}

#[cfg(feature = "compression-gzip")]
#[tokio::test]
async fn compress_strips_stale_caller_content_length() {
    use std::io::Read;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let (buf, body_start, content_len) = loop {
            let n = socket.read(&mut tmp).await.unwrap();
            assert!(n > 0, "client closed before full request");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(hdr_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..hdr_end]).to_lowercase();
                assert_eq!(
                    head.matches("content-length:").count(),
                    1,
                    "exactly one content-length expected:\n{head}"
                );
                let content_len: usize = head
                    .split("\r\n")
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .map(|v| v.trim().parse().unwrap())
                    .expect("content-length header present");
                assert!(
                    content_len < 10_000,
                    "stale caller content-length leaked to the wire: {content_len}"
                );
                let body_start = hdr_end + 4;
                if buf.len() >= body_start + content_len {
                    break (buf, body_start, content_len);
                }
            }
        };

        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(&buf[body_start..body_start + content_len])
            .read_to_end(&mut decoded)
            .unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .await
            .unwrap();
        decoded
    });

    let payload = b"the quick brown fox jumps over the lazy dog. ".repeat(64);
    let resp = Session::new()
        .post(format!("http://{addr}/upload"))
        .header("content-length", "999999")
        .body(payload.clone())
        .compress(ContentEncoding::Gzip)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let decoded = server.await.unwrap();
    assert_eq!(decoded, payload);
}

#[tokio::test]
async fn https_scheme_proxy_is_accepted_and_dialed_over_tls() {
    let err = Session::new()
        .request(http::Method::GET, "https://example.test/")
        .proxy("https://user:secret@127.0.0.1:1")
        .send()
        .await
        .unwrap_err();
    let msg = format!("{err}").to_lowercase();
    assert!(
        !msg.contains("cleartext") && !msg.contains("unsupported proxy scheme"),
        "https proxy must be supported, got: {msg}"
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

    let session = Session::new();
    let err = session
        .post("http://127.0.0.1:9/no-network")
        .json(&BadJson)
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Json);
}
