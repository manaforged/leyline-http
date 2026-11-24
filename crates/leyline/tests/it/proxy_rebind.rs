#![expect(
    clippy::unwrap_used,
    reason = "test harness: unwrap doubles as the assertion"
)]
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use leyline::{Browser, Platform, Session};
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

    let rebound = session
        .to_builder()
        .proxy(&proxy)
        .share_connections(false)
        .build()
        .unwrap();
    rebound.get(url).send().await.unwrap().text().await.unwrap();
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        2,
        "fresh_pool on the same gateway URL must open a new connection"
    );
}

#[tokio::test]
async fn redirect_to_pooled_proxy_preserves_lease_on_same_url_hop_and_strikes_on_block() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local proxy");
    let proxy_addr = proxy_listener.local_addr().expect("proxy local addr");
    let proxy_hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&proxy_hits);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = proxy_listener.accept().await else {
                break;
            };
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let hit = seen.fetch_add(1, Ordering::SeqCst);
                    let reply = if req.contains("/blocked") {
                        "HTTP/1.1 403 Forbidden\r\nCF-Mitigated: challenge\r\nContent-Length: 7\r\nConnection: keep-alive\r\n\r\nblocked"
                    } else if hit == 0 {
                        "HTTP/1.1 302 Found\r\nLocation: http://pooled.test/same\r\nSet-Cookie: hop=1; Path=/\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n"
                    } else if req.to_ascii_lowercase().contains("cookie: hop=1") {
                        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok"
                    } else {
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 3\r\nConnection: keep-alive\r\n\r\nbad"
                    };
                    if stream.write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });

    let direct = leyline::testing::Server::http(|req| {
        if req.target() == "/to-same" {
            leyline::testing::Response::new(302).header("location", "http://pooled.test/same")
        } else {
            leyline::testing::Response::new(302).header("location", "http://pooled.test/blocked")
        }
    })
    .await
    .expect("direct test server");

    let proxy_url = format!("http://{proxy_addr}");
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .proxy(
            leyline::Proxy::identified([(
                proxy_url.as_str(),
                Browser::Firefox159,
                Platform::Linux,
            )])
            .no_proxy(["127.0.0.1"]),
        )
        .build()
        .expect("session builds");

    let resp = session
        .get(direct.url("/to-same"))
        .send()
        .await
        .expect("redirect response");
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.proxy().map(url::Url::as_str),
        Some(format!("{proxy_url}/").as_str())
    );
    assert_eq!(resp.text().await.expect("response body"), "ok");
    assert_eq!(proxy_hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        session.cookies().get("http://pooled.test/", "hop"),
        None,
        "cookie must stay inside the leased identity jar"
    );

    let blocked = session
        .get(direct.url("/to-block"))
        .send()
        .await
        .expect("blocked redirect response");
    assert_eq!(blocked.status().as_u16(), 403);
    assert!(
        session.stats().proxy_failures(&proxy_url) >= 1,
        "block page on redirect hop must strike the pooled proxy"
    );
}
