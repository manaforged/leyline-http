#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use leyline::{DigestAuth, ProtocolPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

fn extract_authorization(headers: &str) -> Option<String> {
    for line in headers.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("authorization: ") {
            let _ = rest;
            let v = line.split_once(": ").unwrap().1.to_string();
            return Some(v);
        }
    }
    None
}

async fn run_digest_server(
    listener: tokio::net::TcpListener,
    algo_wire: &'static str,
    expected_user: &'static str,
    expected_realm: &'static str,
) {
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

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let mut resp = session
        .request(http::Method::GET, format!("http://{addr}/protected"))
        .digest_auth(DigestAuth::new("mufasa", "circle-of-life"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.unwrap(), "ok");
    server.await.unwrap();
}

#[tokio::test]
async fn sha256_challenge_round_trip() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        run_digest_server(listener, "SHA-256", "admin", "secure.local").await;
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/"))
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

        let (mut sock, _) = listener.accept().await.unwrap();
        let headers = read_headers(&mut sock).await;
        let auth = extract_authorization(&headers).expect("auth on stale retry");
        assert!(auth.contains("nonce=\"n2\""), "{auth}");
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/protected"))
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
async fn digest_uri_tracks_the_redirected_challenge_url() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_headers(&mut sock).await;
        assert!(head.starts_with("GET /start "), "{head}");
        sock.write_all(
            b"HTTP/1.1 302 Found\r\n\
              location: /protected\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
        drop(sock);

        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_headers(&mut sock).await;
        assert!(head.starts_with("GET /protected "), "{head}");
        sock.write_all(
            b"HTTP/1.1 401 Unauthorized\r\n\
              WWW-Authenticate: Digest realm=\"r\", nonce=\"n1\", qop=\"auth\", algorithm=MD5\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
        drop(sock);

        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_headers(&mut sock).await;
        assert!(head.starts_with("GET /start "), "{head}");
        let auth = extract_authorization(&head).expect("auth on retry first hop");
        assert!(
            auth.contains("uri=\"/protected\""),
            "digest uri must track the redirected challenge URL, got: {auth}"
        );
        assert!(
            !auth.contains("uri=\"/start\""),
            "digest uri wrongly used the caller's original URL: {auth}"
        );
        sock.write_all(
            b"HTTP/1.1 302 Found\r\n\
              location: /protected\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
        drop(sock);

        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_headers(&mut sock).await;
        assert!(head.starts_with("GET /protected "), "{head}");
        let auth = extract_authorization(&head).expect("auth on retry final hop");
        assert!(auth.contains("uri=\"/protected\""), "{auth}");
        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
            .await
            .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/start"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "digest auth across a same-origin redirect must succeed"
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
        sock.write_all(
            b"HTTP/1.1 401 Unauthorized\r\n\
              WWW-Authenticate: Basic realm=\"x\"\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        sock.flush().await.unwrap();
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    server.await.unwrap();
}
