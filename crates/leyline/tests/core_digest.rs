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
    let resp = session
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

async fn reply(sock: &mut tokio::net::TcpStream, response: &[u8]) {
    sock.write_all(response).await.unwrap();
    sock.flush().await.unwrap();
}

const CHALLENGE_401: &[u8] = b"HTTP/1.1 401 Unauthorized\r\n\
    WWW-Authenticate: Digest realm=\"r\", nonce=\"n1\", qop=\"auth\", algorithm=MD5\r\n\
    content-length: 0\r\nconnection: close\r\n\r\n";

#[tokio::test]
async fn digest_answers_the_redirected_hop_without_replaying_the_post() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut head = read_headers(&mut sock).await;
        while !head.ends_with("x=1") {
            let mut buf = [0u8; 64];
            let n = sock.read(&mut buf).await.unwrap();
            assert!(n > 0, "{head}");
            head.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        assert!(head.starts_with("POST /submit "), "{head}");
        reply(
            &mut sock,
            b"HTTP/1.1 303 See Other\r\nlocation: /protected\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        drop(sock);

        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_headers(&mut sock).await;
        assert!(head.starts_with("GET /protected "), "{head}");
        reply(&mut sock, CHALLENGE_401).await;
        drop(sock);

        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_headers(&mut sock).await;
        reply(
            &mut sock,
            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
        )
        .await;
        head
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .post(format!("http://{addr}/submit"))
        .body("x=1")
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await;
    let answered = server.await.unwrap();
    assert!(
        answered.starts_with("GET /protected "),
        "the digest answer must go to the challenged GET, not replay the POST: {answered}"
    );
    let auth = extract_authorization(&answered).expect("authorization on the challenged hop");
    assert!(auth.contains("uri=\"/protected\""), "{auth}");
    assert_eq!(resp.unwrap().status(), 200);
}

#[tokio::test]
async fn digest_answers_a_new_challenge_after_an_authenticated_redirect() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let _ = read_headers(&mut sock).await;
        reply(&mut sock, CHALLENGE_401).await;
        drop(sock);

        let (mut sock, _) = listener.accept().await.unwrap();
        let _ = read_headers(&mut sock).await;
        reply(
            &mut sock,
            b"HTTP/1.1 302 Found\r\nlocation: /next\r\n\
              content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        drop(sock);

        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_headers(&mut sock).await;
        assert!(head.starts_with("GET /next "), "{head}");
        reply(&mut sock, CHALLENGE_401).await;
        drop(sock);

        let (mut sock, _) = listener.accept().await.unwrap();
        let head = read_headers(&mut sock).await;
        reply(
            &mut sock,
            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok",
        )
        .await;
        head
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .get(format!("http://{addr}/protected"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let answered = server.await.unwrap();
    let auth = extract_authorization(&answered).expect("authorization after the redirect");
    assert!(auth.contains("uri=\"/next\""), "{auth}");
}

#[tokio::test]
async fn digest_challenge_from_a_redirected_origin_is_not_answered() {
    let first = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_addr = first.local_addr().unwrap();
    let other_addr = other.local_addr().unwrap();

    let origin = tokio::spawn(async move {
        let (mut sock, _) = first.accept().await.unwrap();
        let _ = read_headers(&mut sock).await;
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nlocation: http://{other_addr}/protected\r\n\
             content-length: 0\r\nconnection: close\r\n\r\n"
        );
        reply(&mut sock, redirect.as_bytes()).await;
    });
    let challenger = tokio::spawn(async move {
        let (mut sock, _) = other.accept().await.unwrap();
        let _ = read_headers(&mut sock).await;
        reply(&mut sock, CHALLENGE_401).await;
        drop(sock);
        tokio::time::timeout(std::time::Duration::from_millis(300), other.accept())
            .await
            .is_err()
    });

    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    let resp = session
        .get(format!("http://{first_addr}/start"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await
        .unwrap();
    origin.await.unwrap();
    assert!(
        challenger.await.unwrap(),
        "credentials must not be sent to an origin the caller did not target"
    );
    assert_eq!(resp.status(), 401);
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
