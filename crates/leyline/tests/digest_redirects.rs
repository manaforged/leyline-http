#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use leyline::{DigestAuth, ProtocolPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn read_head(sock: &mut TcpStream) -> String {
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

fn authorization(head: &str) -> Option<String> {
    head.split("\r\n")
        .find(|line| line.to_ascii_lowercase().starts_with("authorization: "))
        .map(|line| line.split_once(": ").unwrap().1.to_string())
}

async fn reply(sock: &mut TcpStream, response: &str) {
    sock.write_all(response.as_bytes()).await.unwrap();
    sock.flush().await.unwrap();
}

async fn accept(listener: &TcpListener) -> (TcpStream, String) {
    let (mut sock, _) = listener.accept().await.unwrap();
    let head = read_head(&mut sock).await;
    (sock, head)
}

const OK: &str = "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok";

fn session() -> Session {
    Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap()
}

#[tokio::test]
async fn digest_skips_a_challenge_it_cannot_answer() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = accept(&listener).await;
        reply(
            &mut sock,
            "HTTP/1.1 401 Unauthorized\r\n\
             WWW-Authenticate: Digest realm=\"r\", nonce=\"only-int\", qop=\"auth-int\"\r\n\
             WWW-Authenticate: Digest realm=\"r\", nonce=\"plain\", qop=\"auth\"\r\n\
             content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        drop(sock);
        let retry =
            tokio::time::timeout(std::time::Duration::from_secs(5), accept(&listener)).await;
        let (mut sock, head) = retry.expect("the client retried with the answerable challenge");
        reply(&mut sock, OK).await;
        head
    });

    let resp = session()
        .get(format!("http://{addr}/protected"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await;
    let answered = server.await.unwrap();
    let auth = authorization(&answered).expect("authorization on the retry");
    assert!(auth.contains("nonce=\"plain\""), "{auth}");
    assert_eq!(resp.unwrap().status(), 200);
}

#[tokio::test]
async fn digest_authorizes_a_same_origin_redirect_without_a_new_challenge() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = accept(&listener).await;
        reply(
            &mut sock,
            "HTTP/1.1 401 Unauthorized\r\n\
             WWW-Authenticate: Digest realm=\"r\", nonce=\"n1\", qop=\"auth\"\r\n\
             content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        drop(sock);
        let (mut sock, _) = accept(&listener).await;
        reply(
            &mut sock,
            "HTTP/1.1 302 Found\r\nlocation: /next\r\n\
             content-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        drop(sock);
        let (mut sock, head) = accept(&listener).await;
        reply(&mut sock, OK).await;
        head
    });

    let resp = session()
        .get(format!("http://{addr}/protected"))
        .digest_auth(DigestAuth::new("u", "p"))
        .send()
        .await;
    let redirected = server.await.unwrap();
    assert!(redirected.starts_with("GET /next "), "{redirected}");
    let auth = authorization(&redirected).expect("authorization on the redirect step");
    assert!(auth.contains("uri=\"/next\""), "{auth}");
    assert!(auth.contains("nc=00000002"), "{auth}");
    assert_eq!(resp.unwrap().status(), 200);
}
