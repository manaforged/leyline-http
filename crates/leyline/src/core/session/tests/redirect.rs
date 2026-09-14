use crate::{RedirectPolicy, Session};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::sync::oneshot;
use tokio::time::timeout;

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

async fn head(socket: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut bytes = [0; 1024];
    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
        let n = socket.read(&mut bytes).await.unwrap();
        assert!(n > 0);
        request.extend_from_slice(&bytes[..n]);
    }
    String::from_utf8(request).unwrap()
}

#[tokio::test]
async fn redirect_retains_url_when_another_request_replaces_cache() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (ready, started) = oneshot::channel();
    let server = spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        assert!(
            head(&mut first)
                .await
                .starts_with("GET /start?first=1 HTTP/1.1\r\n")
        );
        ready.send(()).unwrap();
        let (mut other, _) = listener.accept().await.unwrap();
        assert!(
            head(&mut other)
                .await
                .starts_with("GET /other?second=2 HTTP/1.1\r\n")
        );
        other
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nother")
            .await
            .unwrap();
        drop(other);
        first
            .write_all(b"HTTP/1.1 302 Found\r\nLocation: /done?first=1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        drop(first);
        let (mut done, _) = listener.accept().await.unwrap();
        assert!(
            head(&mut done)
                .await
                .starts_with("GET /done?first=1 HTTP/1.1\r\n")
        );
        done.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone")
            .await
            .unwrap();
    });
    let session = Session::builder()
        .http1()
        .without_system_roots()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let request = session.clone();
    let start = format!("{origin}/start?first=1");
    let initial = spawn(async move { request.get(&start).send().await.unwrap() });
    timeout(Duration::from_secs(3), started)
        .await
        .unwrap()
        .unwrap();
    let mut other = session
        .get(&format!("{origin}/other?second=2"))
        .send()
        .await
        .unwrap();
    assert_eq!(other.url(), format!("{origin}/other?second=2"));
    assert_eq!(other.text().await.unwrap(), "other");
    let mut response = initial.await.unwrap();
    assert_eq!(response.url(), format!("{origin}/done?first=1"));
    assert_eq!(
        response.redirect_chain(),
        &[format!("{origin}/start?first=1")]
    );
    assert_eq!(response.text().await.unwrap(), "done");
    server.await.unwrap();
}
