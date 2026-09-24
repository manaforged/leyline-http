use crate::Session;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn read_head(socket: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut bytes = [0; 2048];
    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
        let n = socket.read(&mut bytes).await.unwrap();
        assert!(n > 0, "connection closed before request head");
        request.extend_from_slice(&bytes[..n]);
    }
    String::from_utf8(request).unwrap().to_ascii_lowercase()
}

fn cookie_lines(head: &str) -> Vec<&str> {
    head.lines().filter(|l| l.starts_with("cookie:")).collect()
}

#[tokio::test]
async fn explicit_cookie_replaces_jar_cookie_and_jar_still_emits_alone() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let first = read_head(&mut socket).await;
        assert!(first.starts_with("get /seed http/1.1\r\n"), "{first}");
        assert!(cookie_lines(&first).is_empty(), "{first}");
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nSet-Cookie: jar=1; Path=/\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();

        let second = read_head(&mut socket).await;
        assert!(second.starts_with("get /jar http/1.1\r\n"), "{second}");
        assert_eq!(cookie_lines(&second), ["cookie: jar=1"], "{second}");
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();

        let third = read_head(&mut socket).await;
        assert!(third.starts_with("get /explicit http/1.1\r\n"), "{third}");
        assert_eq!(cookie_lines(&third), ["cookie: mine=2; jar=1"], "{third}");
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let session = Session::builder()
        .tls_trust(crate::TlsTrustConfig::new().without_system_roots())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    session.get(&format!("{origin}/seed")).send().await.unwrap();
    session.get(&format!("{origin}/jar")).send().await.unwrap();
    session
        .get(&format!("{origin}/explicit"))
        .header("cookie", "mine=2; jar=1")
        .send()
        .await
        .unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn cross_origin_redirect_strips_explicit_cookie_and_uses_target_jar() {
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_origin = format!("http://{}", first.local_addr().unwrap());
    let second_origin = format!("http://{}", second.local_addr().unwrap());
    let location = format!("{second_origin}/landing");
    let first_server = tokio::spawn(async move {
        let (mut socket, _) = first.accept().await.unwrap();
        let head = read_head(&mut socket).await;
        assert_eq!(cookie_lines(&head), ["cookie: mine=2"], "{head}");
        let reply =
            format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
        socket.write_all(reply.as_bytes()).await.unwrap();
    });
    let second_server = tokio::spawn(async move {
        let (mut socket, _) = second.accept().await.unwrap();
        let head = read_head(&mut socket).await;
        assert!(head.starts_with("get /landing http/1.1\r\n"), "{head}");
        assert_eq!(cookie_lines(&head), ["cookie: other=3"], "{head}");
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .unwrap();
    });
    let session = Session::builder()
        .tls_trust(crate::TlsTrustConfig::new().without_system_roots())
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    session.cookies().set_cookie(&second_origin, "other", "3");
    let body = session
        .get(&format!("{first_origin}/start"))
        .header("cookie", "mine=2")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "ok");
    first_server.await.unwrap();
    second_server.await.unwrap();
}
