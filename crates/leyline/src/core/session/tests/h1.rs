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

    let session = Session::chrome();
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
        // Read until the full header block plus the declared body are in.
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

    // Compressible payload large enough that the gzip output is strictly
    // smaller than the input — proves the wire body really is compressed.
    let payload = b"the quick brown fox jumps over the lazy dog. ".repeat(64);
    let resp = Session::chrome()
        .post(&format!("http://{addr}/upload"))
        .body(payload.clone())
        .compress(crate::ContentEncoding::Gzip)
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
    // Compression is applied before dispatch, so this errors without a live
    // server: a streaming body cannot be compressed in place.
    let body = crate::Body::stream(futures_util::stream::iter(vec![Ok::<_, std::io::Error>(
        bytes::Bytes::from_static(b"chunk"),
    )]));
    let err = Session::chrome()
        .post("http://127.0.0.1:9/x")
        .body(body)
        .compress(crate::ContentEncoding::Gzip)
        .send()
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Body(_)), "got: {err:?}");
}

#[tokio::test]
async fn unsupported_scheme_proxy_is_refused_not_sent_in_cleartext() {
    // Schemes leyline cannot tunnel safely (here `ftp`) must error before any
    // socket opens — `RequestBuilder::proxy(&str)` is not validated through
    // `ProxyUrl`, so an unhandled scheme must not fall through to the cleartext
    // CONNECT path and leak Proxy-Authorization.
    let err = Session::chrome()
        .get("https://example.test/")
        .proxy("ftp://user:secret@127.0.0.1:1")
        .send()
        .await
        .unwrap_err();
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("ftp") && msg.contains("cleartext"),
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
                // Exactly one content-length, and it must be the compressed
                // length — never the caller's stale 999999.
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
    let resp = Session::chrome()
        .post(&format!("http://{addr}/upload"))
        // A bogus caller-supplied content-length must not survive compression.
        .header("content-length", "999999")
        .body(payload.clone())
        .compress(crate::ContentEncoding::Gzip)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let decoded = server.await.unwrap();
    assert_eq!(decoded, payload);
}

#[tokio::test]
async fn https_scheme_proxy_is_refused_not_sent_in_cleartext() {
    // `ProxyUrl` advertises `https` as a valid scheme, but leyline cannot yet
    // TLS-handshake to the proxy — so an https:// proxy must error rather than
    // send the CONNECT request (and Proxy-Authorization) in cleartext. The
    // refusal fires on scheme inspection, before any socket is opened, so the
    // unroutable :1 port is never dialed.
    let err = Session::chrome()
        .get("https://example.test/")
        .proxy("https://user:secret@127.0.0.1:1")
        .send()
        .await
        .unwrap_err();
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("https://") && msg.contains("cleartext"),
        "expected an https-proxy cleartext-refusal error, got: {msg}"
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

    let session = Session::chrome();
    let err = session
        .post("http://127.0.0.1:9/no-network")
        .json(&BadJson)
        .send()
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Json(_)));
}

// ---- H1 streaming download ----

/// Read a request head (up to the `\r\n\r\n` terminator). Returns `false`
/// on a clean EOF before any bytes — the keep-alive connection was closed.
async fn read_request_head(sock: &mut tokio::net::TcpStream) -> bool {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = sock.read(&mut tmp).await.unwrap();
        if n == 0 {
            return !buf.is_empty();
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return true;
        }
    }
}

#[tokio::test]
async fn streamed_chunked_response_reassembles() {
    use futures_util::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        read_request_head(&mut sock).await;
        // "Hello, " (7) + "streaming " (A=10) + "world!" (6), then terminator.
        sock.write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
              7\r\nHello, \r\nA\r\nstreaming \r\n6\r\nworld!\r\n0\r\n\r\n",
        )
        .await
        .unwrap();
    });

    let resp = Session::chrome()
        .get(&format!("http://{addr}/x"))
        .stream()
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let mut body = Vec::new();
    let mut stream = resp.into_stream().unwrap();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(body, b"Hello, streaming world!");
    server.await.unwrap();
}

#[tokio::test]
async fn streamed_fixed_length_response_reassembles() {
    use futures_util::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // 10 KB — larger than the 8 KB pump read buffer, so it spans multiple chunks.
    let payload = b"the quick brown fox ".repeat(500);
    let payload_srv = payload.clone();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        read_request_head(&mut sock).await;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            payload_srv.len()
        );
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.write_all(&payload_srv).await.unwrap();
    });

    let resp = Session::chrome()
        .get(&format!("http://{addr}/x"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut body = Vec::new();
    let mut stream = resp.into_stream().unwrap();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(body, payload);
    server.await.unwrap();
}

#[tokio::test]
async fn streamed_connection_is_reused_after_full_drain() {
    use futures_util::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // The server accepts exactly ONE connection and serves two sequential
    // requests on it. If the streamed connection is reinstated after a clean
    // drain, request 2 reuses it; otherwise request 2 would need a second
    // accept that never comes and the test would hang.
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        for body in [b"first".as_slice(), b"second".as_slice()] {
            assert!(read_request_head(&mut sock).await, "expected a request");
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                body.len()
            );
            sock.write_all(head.as_bytes()).await.unwrap();
            sock.write_all(body).await.unwrap();
        }
    });

    let session = Session::chrome();
    let r1 = session
        .get(&format!("http://{addr}/a"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut b1 = Vec::new();
    let mut s1 = r1.into_stream().unwrap();
    while let Some(chunk) = s1.next().await {
        b1.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(b1, b"first");

    // Reuses the reinstated connection (the server only accepted once).
    let r2 = session
        .get(&format!("http://{addr}/b"))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.text(), "second");
    server.await.unwrap();
}

#[tokio::test]
async fn streamed_read_timeout_fires_on_stall() {
    use futures_util::StreamExt;
    use std::time::Duration;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        read_request_head(&mut sock).await;
        // One chunk, then stall: no further bytes, connection held open, so the
        // per-chunk read-idle timeout must fire on the consumer side.
        sock.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });

    let session = Session::builder()
        .read_timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let resp = session
        .get(&format!("http://{addr}/x"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut stream = resp.into_stream().unwrap();
    assert_eq!(&stream.next().await.unwrap().unwrap()[..], b"hello");
    // The next chunk never arrives → read_timeout fires (well before the
    // 300 s request-wide timeout would).
    let err = stream.next().await.unwrap().unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut, "{err}");
    server.abort();
}

#[tokio::test]
async fn streamed_connection_dropped_when_consumer_drops_early() {
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_srv = accepts.clone();
    // Real keep-alive server: each accepted connection serves requests in a
    // loop until the peer closes it. A correctly-dropped early stream closes
    // its connection, forcing request 2 onto a second accept.
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            accepts_srv.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                while read_request_head(&mut sock).await {
                    // 50 chunks of 10 bytes — far more than the pump's 16-deep
                    // channel can buffer, so a consumer that reads one chunk and
                    // drops leaves unread body on the wire.
                    let mut resp =
                        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n"
                            .to_vec();
                    for _ in 0..50 {
                        resp.extend_from_slice(b"A\r\n0123456789\r\n");
                    }
                    resp.extend_from_slice(b"0\r\n\r\n");
                    if sock.write_all(&resp).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    let session = Session::chrome();
    let r1 = session
        .get(&format!("http://{addr}/a"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut s1 = r1.into_stream().unwrap();
    let _first = s1.next().await.unwrap().unwrap();
    drop(s1); // early drop — the pump must NOT reinstate this connection

    // A correctly-dropped connection forces a second accept for request 2.
    let r2 = session
        .get(&format!("http://{addr}/b"))
        .send()
        .await
        .unwrap();
    assert_eq!(r2.status(), 200);
    // Poll briefly: the second accept is registered as r2 completes.
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        2,
        "an early-dropped stream's connection must not be reused"
    );
}
