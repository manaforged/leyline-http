#![expect(
    clippy::unwrap_used,
    reason = "test harness: unwrap doubles as the assertion"
)]
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::Session;
use leyline::profile::{Browser, Platform};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn spawn_counting_proxy() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    let n = match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    if !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        continue;
                    }
                    let body = b"ok";
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(head.as_bytes()).await.unwrap();
                    stream.write_all(body).await.unwrap();
                }
            });
        }
    });
    (format!("http://{addr}"), accepted)
}

#[tokio::test]
async fn fresh_pool_on_the_same_proxy_opens_a_new_connection() {
    let (proxy, accepted) = spawn_counting_proxy().await;
    let session = Session::builder()
        .browser(Browser::Chrome148)
        .platform(Platform::Windows)
        .proxy(&proxy)
        .build()
        .unwrap();
    let url = "http://gateway-rebind.test/first";

    session.get(url).send().await.unwrap().text().await.unwrap();
    session.get(url).send().await.unwrap().text().await.unwrap();
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "same session reuses the socket"
    );

    let rebound = session.with_proxy(&proxy).fresh_pool();
    rebound.get(url).send().await.unwrap().text().await.unwrap();
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        2,
        "fresh_pool on the same gateway URL must open a new connection"
    );
}
