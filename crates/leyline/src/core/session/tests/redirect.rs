use crate::Session;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::sync::oneshot;
use tokio::time::timeout;

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
        .protocol(crate::ProtocolPolicy::Http1)
        .tls_trust(crate::TlsTrustConfig::new().without_system_roots())
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
    let other = session
        .get(&format!("{origin}/other?second=2"))
        .send()
        .await
        .unwrap();
    assert_eq!(other.url(), format!("{origin}/other?second=2"));
    assert_eq!(other.text().await.unwrap(), "other");
    let response = initial.await.unwrap();
    assert_eq!(response.url(), format!("{origin}/done?first=1"));
    assert_eq!(
        response.redirect_chain(),
        &[format!("{origin}/start?first=1")]
    );
    assert_eq!(response.text().await.unwrap(), "done");
    server.await.unwrap();
}
