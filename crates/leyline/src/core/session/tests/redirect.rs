use crate::{RedirectPolicy, Session};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn redirect_override_keeps_cookies_and_connection_without_changing_parent() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        for (index, path) in ["/start", "/done", "/start", "/start", "/done"]
            .iter()
            .enumerate()
        {
            let mut request = Vec::new();
            let mut bytes = [0; 1024];
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let n = socket.read(&mut bytes).await.unwrap();
                assert!(n > 0, "connection closed before request {index}");
                request.extend_from_slice(&bytes[..n]);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(
                request.starts_with(&format!("GET {path} HTTP/1.1\r\n")),
                "{request}"
            );
            if index > 0 {
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("cookie: session=kept\r\n"),
                    "{request}"
                );
            }
            let reply = if *path == "/start" {
                b"HTTP/1.1 302 Found\r\nLocation: /done\r\nSet-Cookie: session=kept; Path=/\r\nContent-Length: 0\r\n\r\n".as_slice()
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".as_slice()
            };
            socket.write_all(reply).await.unwrap();
        }
    });
    let session = Session::builder()
        .without_system_roots()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let derived = session.with_redirect_policy(RedirectPolicy::none());
    let url = format!("{origin}/start");
    assert_eq!(
        session
            .get(&url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    let response = derived.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 302);
    assert_eq!(response.header("location"), Some("/done"));
    assert_eq!(
        session
            .get(&url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    assert_eq!(derived.pool_stats().installs, 1);
    assert_eq!(session.pool_stats().h1_hits, 4);
    server.await.unwrap();
}
