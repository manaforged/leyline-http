#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use leyline::{ProxyConfig, ProxyRule, RetryPolicy, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn gateway_proxy() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match socket.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            let line = String::from_utf8_lossy(&head)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned();
            log.lock().unwrap().push(line);
            drop(
                socket
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                    .await,
            );
        }
    });
    (url, seen)
}

#[cfg(feature = "websocket")]
#[tokio::test]
async fn a_websocket_uses_an_https_proxy_rule() {
    let (proxy, seen) = gateway_proxy().await;
    let session = Session::builder()
        .proxy(ProxyConfig::new().env(false).rule(ProxyRule::https(proxy)))
        .build()
        .unwrap();
    drop(
        session
            .websocket("wss://127.0.0.1:9/socket")
            .connect()
            .await,
    );
    let lines = seen.lock().unwrap().clone();
    assert!(
        lines.iter().any(|line| line.starts_with("CONNECT ")),
        "{lines:?}"
    );
}

#[tokio::test]
async fn a_proxy_answering_502_is_retried() {
    let (proxy, seen) = gateway_proxy().await;
    let session = Session::builder()
        .proxy(ProxyConfig::new().env(false).rule(ProxyRule::all(proxy)))
        .retry(
            RetryPolicy::transient()
                .max_retries(2)
                .initial_backoff(Duration::from_millis(1)),
        )
        .build()
        .unwrap();
    let err = session.get("https://127.0.0.1:9/").await.unwrap_err();
    let connects = seen.lock().unwrap().len();
    assert!(connects >= 2, "{connects} CONNECT attempts, {err:?}");
}
