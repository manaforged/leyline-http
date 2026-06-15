//! Integration tests for the HTTP/1.1 keep-alive pool.
//!
//! These tests stand up a minimal `TcpListener`-based mock server,
//! drive the pool via [`send_request_h1_pooled`] directly (bypassing
//! the full `Session` plumbing to keep the contract tight), and
//! assert:
//!
//! 1. Back-to-back requests to the same host ride one TCP accept().
//! 2. `Connection: close` on the response forces a fresh TCP on the
//!    next request.
//! 3. A mid-exchange drop by the server surfaces as a clean error
//!    (no panic) and the next request opens a fresh TCP — with the
//!    `evictions_dead` counter bumped.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use leyline::pool::{send_request_h1_pooled, H1Body, H1ResponseBody, H1Target, Pool};
use leyline::profile::{Browser, Platform, ProfileRegistry};
use leyline::tls::{ConnectorVariant, FingerprintConnector};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Spin up a bare `FingerprintConnector` so we can exercise the
/// plaintext-HTTP path without standing up a full `Session`. The TLS
/// fields never fire for `http://` destinations.
fn bare_connector() -> ConnectorVariant {
    let registry = ProfileRegistry::builtin();
    let profile = registry
        .get_browser(Browser::Chrome147)
        .expect("chrome147 profile is bundled");
    ConnectorVariant::Fingerprint(
        FingerprintConnector::new(profile, Platform::Windows.tcp_profile())
            .expect("build fingerprint connector"),
    )
}

/// A tiny mock server that accepts TCP connections, reads one
/// request per connection, and sends a canned response. The script
/// closure receives the request index (0-based per accept()) and
/// returns `(response_bytes, keep_alive)` — when `keep_alive` is
/// false, the server closes the connection after writing the
/// response.
async fn spawn_mock_server<F>(
    script: F,
) -> (
    std::net::SocketAddr,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
)
where
    F: Fn(usize) -> Vec<u8> + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let accepts_clone = accepts.clone();
    let script = Arc::new(script);

    let handle = tokio::spawn(async move {
        loop {
            let accept = listener.accept().await;
            let Ok((mut socket, _)) = accept else { break };
            let conn_index = accepts_clone.fetch_add(1, Ordering::SeqCst);
            let script = script.clone();
            tokio::spawn(async move {
                // Per-connection request loop. Each iteration reads
                // one request (HEAD-only — no bodies in tests), runs
                // the script, writes the response, then decides
                // whether to keep looping based on the script's
                // response bytes.
                let mut req_index = 0usize;
                loop {
                    let mut req = Vec::new();
                    let mut tmp = [0u8; 1024];
                    loop {
                        match socket.read(&mut tmp).await {
                            Ok(0) => return,
                            Ok(n) => {
                                req.extend_from_slice(&tmp[..n]);
                                if req.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => return,
                        }
                    }
                    // Use (conn_index * 16) + req_index as the script
                    // argument — tests can tell apart "2nd request on
                    // conn 1" vs "1st request on conn 2" via index
                    // arithmetic.
                    let bytes = script(conn_index * 16 + req_index);
                    if socket.write_all(&bytes).await.is_err() {
                        return;
                    }
                    req_index += 1;
                    // Detect `Connection: close` in what we wrote;
                    // if present, drop the connection so the client
                    // sees a closed socket on the next request.
                    let resp_str = String::from_utf8_lossy(&bytes).to_lowercase();
                    if resp_str.contains("connection: close") {
                        return;
                    }
                }
            });
        }
    });

    (addr, accepts, handle)
}

/// Shared setup: pool, connector, URL.
fn setup() -> (Arc<Pool>, ConnectorVariant) {
    (Arc::new(Pool::new()), bare_connector())
}

fn ok_response() -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec()
}

#[tokio::test]
async fn h1_pool_reuses_connection_for_sequential_requests() {
    let (addr, accepts, _server) = spawn_mock_server(|_| ok_response()).await;
    let (pool, connector) = setup();

    let host = "127.0.0.1".to_string();
    let port = addr.port();
    let url = url::Url::parse(&format!("http://{addr}/")).unwrap();

    for _ in 0..3 {
        let resp = send_request_h1_pooled(
            &pool,
            &connector,
            "http",
            &host,
            port,
            "GET",
            &url,
            vec![],
            H1Body::Empty,
            None,
            H1Target::OriginForm,
        )
        .await
        .expect("request succeeds");
        assert_eq!(resp.status, 200);
        let H1ResponseBody::Buffered(body) = resp.body;
        assert_eq!(body, b"ok");
    }

    // Give any in-flight accept() a moment to resolve before we
    // probe the counter.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        1,
        "pool should have reused the single TCP connection for all 3 requests"
    );

    let stats = pool.stats();
    assert_eq!(stats.h1_hits, 2, "2 of 3 checkouts should hit the pool");
    assert_eq!(stats.h1_misses, 1, "first request is a miss (fresh open)");
}

#[tokio::test]
async fn h1_pool_honours_connection_close() {
    // Script: first response carries `Connection: close`, second does not.
    let (addr, accepts, _server) = spawn_mock_server(|i| {
        if i == 0 {
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
        } else {
            ok_response()
        }
    })
    .await;
    let (pool, connector) = setup();

    let host = "127.0.0.1".to_string();
    let port = addr.port();
    let url = url::Url::parse(&format!("http://{addr}/")).unwrap();

    for _ in 0..2 {
        let resp = send_request_h1_pooled(
            &pool,
            &connector,
            "http",
            &host,
            port,
            "GET",
            &url,
            vec![],
            H1Body::Empty,
            None,
            H1Target::OriginForm,
        )
        .await
        .expect("request succeeds");
        assert_eq!(resp.status, 200);
    }

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        2,
        "Connection: close must prevent reuse on the second request"
    );
}

#[tokio::test]
async fn h1_pool_recovers_when_server_drops_connection() {
    // Script: first request responds normally and then the server
    // drops the socket before the client's next request — simulated
    // by accepting, reading one request, writing a clean response
    // (no `Connection: close`), and then letting the connection
    // close naturally when the task ends.
    let (listener_addr, accepts, _server) = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_c = accepts.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                accepts_c.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut tmp = [0u8; 1024];
                    let mut req = Vec::new();
                    loop {
                        match socket.read(&mut tmp).await {
                            Ok(0) => return,
                            Ok(n) => {
                                req.extend_from_slice(&tmp[..n]);
                                if req.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => return,
                        }
                    }
                    // Write a clean response WITHOUT `Connection:
                    // close` — the client will happily pool the
                    // connection. Then drop the socket.
                    let _ = socket.write_all(&ok_response()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (addr, accepts, handle)
    };

    let (pool, connector) = setup();
    let host = "127.0.0.1".to_string();
    let port = listener_addr.port();
    let url = url::Url::parse(&format!("http://{listener_addr}/")).unwrap();

    // Request 1 — handshake + clean response. Pool parks the stream.
    let resp1 = send_request_h1_pooled(
        &pool,
        &connector,
        "http",
        &host,
        port,
        "GET",
        &url,
        vec![],
        H1Body::Empty,
        None,
        H1Target::OriginForm,
    )
    .await
    .expect("first request succeeds");
    assert_eq!(resp1.status, 200);

    // Give the server side of the socket a moment to finish closing
    // so the client's pooled read hits an EOF rather than a live
    // connection.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request 2 — the pooled stream is dead; send_request_h1_pooled
    // should retry on a fresh TCP and succeed.
    let resp2 = send_request_h1_pooled(
        &pool,
        &connector,
        "http",
        &host,
        port,
        "GET",
        &url,
        vec![],
        H1Body::Empty,
        None,
        H1Target::OriginForm,
    )
    .await
    .expect("second request recovers on fresh TCP");
    assert_eq!(resp2.status, 200);

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        2,
        "second request should have opened a fresh TCP after the pooled stream died"
    );

    let stats = pool.stats();
    assert!(
        stats.evictions_dead >= 1,
        "dead-stream eviction should have been counted (got {stats:?})"
    );
}
