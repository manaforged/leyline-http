//! Integration tests for the HTTP/1.1 keep-alive pool.
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::pool::{H1Body, H1ResponseBody, H1Target, Pool, send_request_h1_pooled};
use leyline::profile::{Browser, Platform, ProfileRegistry};
use leyline::tls::FingerprintConnector;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Spin up a bare `FingerprintConnector` so we can exercise the plaintext-HTTP path without standing up a full `Session`.
fn bare_connector() -> FingerprintConnector {
    let registry = ProfileRegistry::builtin();
    let profile = registry
        .get_browser(Browser::Chrome147)
        .expect("chrome147 profile is bundled");
    FingerprintConnector::new(profile, Platform::Windows.tcp_profile())
        .expect("build fingerprint connector")
}

/// A tiny mock server that accepts TCP connections, reads one request per connection, and sends a canned response.
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
                    let bytes = script(conn_index * 16 + req_index);
                    if socket.write_all(&bytes).await.is_err() {
                        return;
                    }
                    req_index += 1;
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
fn setup() -> (Arc<Pool>, FingerprintConnector) {
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
            false,
        )
        .await
        .expect("request succeeds");
        assert_eq!(resp.status, 200);
        let H1ResponseBody::Buffered(body) = resp.body else {
            unreachable!("buffered request returns a buffered body");
        };
        assert_eq!(body, b"ok");
    }

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
            false,
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
        false,
    )
    .await
    .expect("first request succeeds");
    assert_eq!(resp1.status, 200);

    tokio::time::sleep(Duration::from_millis(50)).await;

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
        false,
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
        stats.stale_probed + stats.evictions_dead >= 1,
        "dead pooled stream should have been detected (got {stats:?})"
    );
}

/// A mock server that delays each response and tracks the PEAK number of connections open at once — so a test can assert the per-host connection cap actually bounds concurrency.
async fn spawn_peak_tracking_server(
    delay: Duration,
) -> (
    std::net::SocketAddr,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let live = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let accepts_c = accepts.clone();
    let live_c = live.clone();
    let peak_c = peak.clone();

    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            accepts_c.fetch_add(1, Ordering::SeqCst);
            let now_live = live_c.fetch_add(1, Ordering::SeqCst) + 1;
            peak_c.fetch_max(now_live, Ordering::SeqCst);
            let live_task = live_c.clone();
            let delay = delay;
            tokio::spawn(async move {
                let mut tmp = [0u8; 1024];
                loop {
                    let mut req = Vec::new();
                    let done = loop {
                        match socket.read(&mut tmp).await {
                            Ok(0) | Err(_) => break true,
                            Ok(n) => {
                                req.extend_from_slice(&tmp[..n]);
                                if req.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break false;
                                }
                            }
                        }
                    };
                    if done {
                        break;
                    }
                    tokio::time::sleep(delay).await;
                    if socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                live_task.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });

    (addr, accepts, peak, handle)
}

/// The per-host H1 cap must bound the number of connections open at once, and the warm connections must be reused for the queued overflow rather than each request opening (and discarding) a fresh socket.
#[tokio::test]
async fn h1_cap_bounds_concurrency_and_reuses_warm_connections() {
    const CAP: usize = 3;
    const REQUESTS: usize = 12;
    let (addr, accepts, peak, _server) =
        spawn_peak_tracking_server(Duration::from_millis(40)).await;
    let pool = Arc::new(Pool::with_limits(Duration::from_secs(30), 2048, CAP));
    let connector = bare_connector();
    let host = "127.0.0.1".to_string();
    let port = addr.port();
    let url = url::Url::parse(&format!("http://{addr}/")).unwrap();

    let mut handles = Vec::new();
    for _ in 0..REQUESTS {
        let pool = Arc::clone(&pool);
        let connector = connector.clone();
        let host = host.clone();
        let url = url.clone();
        handles.push(tokio::spawn(async move {
            send_request_h1_pooled(
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
                false,
            )
            .await
            .map(|r| r.status)
        }));
    }
    for h in handles {
        let status = h.await.unwrap().expect("request succeeds");
        assert_eq!(status, 200);
    }

    assert!(
        peak.load(Ordering::SeqCst) <= CAP,
        "concurrent connections ({}) must never exceed the cap ({CAP})",
        peak.load(Ordering::SeqCst)
    );
    assert!(
        accepts.load(Ordering::SeqCst) <= CAP,
        "at most {CAP} sockets should be opened (got {})",
        accepts.load(Ordering::SeqCst)
    );
    let stats = pool.stats();
    assert!(
        stats.h1_hits >= (REQUESTS - CAP) as u64,
        "the {} queued requests should reuse warm connections (h1_hits={}, stats={stats:?})",
        REQUESTS - CAP,
        stats.h1_hits
    );
}

/// A request cancelled mid-exchange (its future dropped) must release its per-host permit, or a later request to the same host deadlocks.
#[tokio::test]
async fn h1_cancelled_request_releases_permit() {
    let (addr, _accepts, _peak, _server) =
        spawn_peak_tracking_server(Duration::from_millis(500)).await;
    let pool = Arc::new(Pool::with_limits(Duration::from_secs(30), 2048, 1));
    let connector = bare_connector();
    let host = "127.0.0.1".to_string();
    let port = addr.port();
    let url = url::Url::parse(&format!("http://{addr}/")).unwrap();

    let a = {
        let pool = Arc::clone(&pool);
        let connector = connector.clone();
        let host = host.clone();
        let url = url.clone();
        async move {
            send_request_h1_pooled(
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
                false,
            )
            .await
        }
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(50), a)
            .await
            .is_err(),
        "request A should still be in-flight (cancelled by timeout)"
    );

    let b = send_request_h1_pooled(
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
        false,
    );
    let resp = tokio::time::timeout(Duration::from_secs(3), b)
        .await
        .expect("request B must not deadlock — the cancelled request must release its permit")
        .expect("request B succeeds");
    assert_eq!(resp.status, 200);
}

/// A POST on a warm pooled socket whose server answers with a truncated body and closes must surface the error — never replay on a second connection.
#[tokio::test]
async fn post_is_not_replayed_after_mid_response_close() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let conns = Arc::new(AtomicUsize::new(0));
    let conns_srv = conns.clone();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        conns_srv.fetch_add(1, Ordering::SeqCst);

        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf).await;
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await;
        let _ = sock.flush().await;

        let _ = sock.read(&mut buf).await;
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc")
            .await;
        let _ = sock.flush().await;
        drop(sock);
    });

    let pool = Arc::new(Pool::new());
    let connector = bare_connector();

    let url = url::Url::parse(&format!("http://{addr}/warm")).unwrap();
    let warm = send_request_h1_pooled(
        &pool,
        &connector,
        "http",
        &addr.ip().to_string(),
        addr.port(),
        "GET",
        &url,
        vec![],
        H1Body::Empty,
        None,
        H1Target::OriginForm,
        false,
    )
    .await
    .expect("warm-up request succeeds");
    assert_eq!(warm.status, 200);

    let url = url::Url::parse(&format!("http://{addr}/charge")).unwrap();
    let post = send_request_h1_pooled(
        &pool,
        &connector,
        "http",
        &addr.ip().to_string(),
        addr.port(),
        "POST",
        &url,
        vec![],
        H1Body::Buffered(bytes::Bytes::from_static(b"payload")),
        None,
        H1Target::OriginForm,
        false,
    )
    .await;

    assert!(post.is_err(), "mid-response close must surface as an error");

    assert_eq!(conns.load(Ordering::SeqCst), 1, "POST must not be replayed");
    server.await.unwrap();
}
