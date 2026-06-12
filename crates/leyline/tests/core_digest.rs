//! Integration tests for HTTP Digest authentication.

use leyline::core::{DigestAuth, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Drain HTTP/1 headers off a socket, returning the header block text.
async fn read_headers(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = [0u8; 4096];
    let mut acc = Vec::new();
    loop {
        let n = sock.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        acc.extend_from_slice(&buf[..n]);
        if acc.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&acc).to_string()
}

/// Extract the `Authorization:` header value (if any) from a block of
/// HTTP/1 request headers.
fn extract_authorization(headers: &str) -> Option<String> {
    for line in headers.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("authorization: ") {
            let _ = rest;
            // Preserve the original value (with casing).
            let v = line.split_once(": ").unwrap().1.to_string();
            return Some(v);
        }
    }
    None
}

/// Mock a two-step Digest challenge/response:
/// 1. Return 401 with `WWW-Authenticate: Digest ...`.
/// 2. On the retried request, verify the `Authorization` header parses
///    as a Digest response, then return 200.
async fn run_digest_server(
    listener: tokio::net::TcpListener,
    algo_wire: &'static str,
    expected_user: &'static str,
    expected_realm: &'static str,
) {
    // Round 1.
    let (mut sock, _) = listener.accept().await.unwrap();
    let _headers = read_headers(&mut sock).await;
    let challenge = format!(
        "WWW-Authenticate: Digest realm=\"{expected_realm}\", \
         nonce=\"abc123nonce\", qop=\"auth\", algorithm={algo_wire}"
    );
    let resp = format!(
        "HTTP/1.1 401 Unauthorized\r\n\
         {challenge}\r\n\
         content-length: 0\r\nconnection: close\r\n\r\n"
    );
    sock.write_all(resp.as_bytes()).await.unwrap();
    sock.flush().await.unwrap();
    drop(sock);

    // Round 2.
    let (mut sock, _) = listener.accept().await.unwrap();
    let headers = read_headers(&mut sock).await;
    let auth = extract_authorization(&headers).expect("Authorization header on retry");
    assert!(auth.starts_with("Digest "), "expected Digest, got: {auth}");
    assert!(
        auth.contains(&format!("username=\"{expected_user}\"")),
        "{auth}"
    );
    assert!(auth.contains("nonce=\"abc123nonce\""), "{auth}");
    assert!(auth.contains("response=\""), "{auth}");
    assert!(auth.contains("qop=auth"), "{auth}");
    assert!(auth.contains(&format!("algorithm={algo_wire}")), "{auth}");

    sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
        .await
        .unwrap();
    sock.flush().await.unwrap();
}

#[tokio::test]
async fn md5_challenge_round_trip() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        run_digest_server(listener, "MD5", "mufasa", "example.org").await;
    });

    let session = Session::builder().http1().build().unwrap();
    let resp = session
        .get(&format!("http://{addr}/protected"))
        .digest_auth(DigestAuth::new("mufasa", "circle-of-life"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text(), "ok");
    server.await.unwrap();
}

#[tokio::test]
async fn sha256_challenge_round_trip() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        run_digest_server(listener, "SHA-256", "admin", "secure.local").await;
    });

    let session = Session::builder().http1().build().unwrap();
    let resp = session
        .get(&format!("http://{addr}/"))
        .digest_auth(DigestAuth::new("admin", "hunter2"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    server.await.unwrap();
}

#[tokio::test]
async fn stale_nonce_is_retried_transparently() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        // Round 1: initial challenge.
        let (mut sock, _) = listener.accept().await.unwrap();
        let _ = read_headers(&mut sock).await;
        sock.write_all(
            b"HTTP/1.1 401 Unauthorized\r\n\
              WWW-Authenticate: Digest realm=\"r\", nonce=\"n1\", qop=\"auth\", algorithm=MD5\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
        drop(sock);

        // Round 2: credentials were fine but the nonce expired in
        // flight — answer 401 stale=true with a fresh nonce.
        let (mut sock, _) = listener.accept().await.unwrap();
        let headers = read_headers(&mut sock).await;
        let auth = extract_authorization(&headers).expect("auth on first retry");
        assert!(auth.contains("nonce=\"n1\""), "{auth}");
        sock.write_all(
            b"HTTP/1.1 401 Unauthorized\r\n\
              WWW-Authenticate: Digest realm=\"r\", nonce=\"n2\", qop=\"auth\", algorithm=MD5, stale=true\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
        drop(sock);

        // Round 3: RFC 7616 §3.3 — the client must retry with the
        // fresh nonce without surfacing the 401 to the caller.
        let (mut sock, _) = listener.accept().await.unwrap();
        let headers = read_headers(&mut sock).await;
        let auth = extract_authorization(&headers).expect("auth on stale retry");
        assert!(auth.contains("nonce=\"n2\""), "{auth}");
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder().http1().build().unwrap();
    let resp = session
        .get(&format!("http://{addr}/protected"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "stale=true 401 must be retried transparently with the fresh nonce"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn non_digest_401_is_passed_through() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let _ = read_headers(&mut sock).await;
        // 401 with a Basic challenge — not Digest, so we should NOT retry.
        sock.write_all(
            b"HTTP/1.1 401 Unauthorized\r\n\
              WWW-Authenticate: Basic realm=\"x\"\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder().http1().build().unwrap();
    let resp = session
        .get(&format!("http://{addr}/"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    server.await.unwrap();
}
