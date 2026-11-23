use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use http::StatusCode;
#[cfg(all(feature = "http3", feature = "socks"))]
use leyline::Protocol;
use leyline::testing::{Response, Server};
use leyline::{Browser, Kind, Proxy, Redirect, Retry, Session, Tls};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(all(feature = "http3", feature = "socks"))]
use tokio::net::TcpListener;
use tokio::sync::Notify;

use super::combination_matrix::spawn_connect_proxy;
use super::tls_support;
use super::v030_workloads::TempDir;

#[tokio::test]
async fn download_and_events_honor_retry_status_wait_and_ctl_last_id() {
    let dir = TempDir::new("contract-dl-sse");

    let dl_hits = Arc::new(AtomicUsize::new(0));
    let dl_hits_bg = Arc::clone(&dl_hits);
    let dl_server = Server::http(move |req| match req.target() {
        "/retry-ok" => {
            if dl_hits_bg.fetch_add(1, Ordering::SeqCst) == 0 {
                Response::new(429)
                    .header("retry-after", "0")
                    .header("content-encoding", "gzip")
                    .body("slow down")
            } else {
                Response::new(200).body("file-bytes")
            }
        }
        "/retry-long" => Response::new(429)
            .header("retry-after", "3600")
            .body("way too long"),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .build()
        .unwrap();
    let saw_429 = Arc::new(AtomicUsize::new(0));
    let saw_429_cb = Arc::clone(&saw_429);
    let target_ok = dir.path("ok.bin");
    let written = session
        .get(dl_server.url("/retry-ok"))
        .retry(Retry::new(1).when(move |attempt| {
            if attempt.status() == Some(StatusCode::TOO_MANY_REQUESTS) {
                saw_429_cb.fetch_add(1, Ordering::SeqCst);
                true
            } else {
                false
            }
        }))
        .download_to(&target_ok)
        .await
        .unwrap();
    assert_eq!(written, 10);
    assert_eq!(saw_429.load(Ordering::SeqCst), 1);

    let start = Instant::now();
    let err = session
        .get(dl_server.url("/retry-long"))
        .retry(Retry::new(2).max_wait(Duration::from_secs(1)))
        .download_to(dir.path("declined.bin"))
        .await
        .expect_err("Retry-After: 3600 exceeds max_wait(1s)");
    assert_eq!(err.kind(), Kind::Status);
    assert_eq!(err.status(), Some(StatusCode::TOO_MANY_REQUESTS));
    assert!(start.elapsed() < Duration::from_secs(1));

    let sse_hits = Arc::new(AtomicUsize::new(0));
    let sse_hits_bg = Arc::clone(&sse_hits);
    let sse_server = Server::http(move |req| {
        let hit = sse_hits_bg.fetch_add(1, Ordering::SeqCst);
        match hit {
            0 => Response::new(503).header("retry-after", "0").body("retry"),
            1 => Response::new(200)
                .header("content-type", "text/event-stream")
                .body("id: ctl\x07bad\ndata: first\n\n"),
            2 => {
                assert!(
                    req.header("last-event-id").is_none(),
                    "CTL Last-Event-ID must not be emitted on reconnect"
                );
                Response::new(200)
                    .header("content-type", "text/event-stream")
                    .body("data: second\n\n")
            }
            _ => Response::new(204),
        }
    })
    .await
    .unwrap();

    let saw_503 = Arc::new(AtomicUsize::new(0));
    let saw_503_cb = Arc::clone(&saw_503);
    let mut events = session.get(sse_server.url("/sse")).events().unwrap().retry(
        Retry::new(2)
            .max_wait(Duration::from_millis(80))
            .backoff(Duration::from_millis(5), Duration::from_millis(10))
            .when(move |attempt| {
                if attempt.status() == Some(StatusCode::SERVICE_UNAVAILABLE) {
                    saw_503_cb.fetch_add(1, Ordering::SeqCst);
                    true
                } else {
                    attempt.error().is_some_and(leyline::Error::is_retryable)
                }
            }),
    );

    let e1 = events.next().await.expect("e1").expect("e1 ok");
    assert_eq!(e1.data(), "first");
    let e2 = events.next().await.expect("e2").expect("e2 ok");
    assert_eq!(e2.data(), "second");
    assert!(events.next().await.is_none());
    assert_eq!(saw_503.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn redirect_taint_strips_credentials_and_pages_strips_cross_origin_referer() {
    let origin_b = Server::http(|req| match req.target() {
        "/bounce" => {
            assert!(req.header("authorization").is_none(), "B received auth");
            let back = req.header("x-return-to").unwrap().to_owned();
            Response::new(302).header("location", back)
        }
        "/p2" => {
            assert!(
                req.header("referer").is_none(),
                "cross-origin page leaked referer: {:?}",
                req.header("referer")
            );
            Response::new(200).body("page-2")
        }
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let b_bounce = origin_b.url("/bounce");
    let b_p2 = origin_b.url("/p2");
    let origin_a = Server::http(move |req| match req.target() {
        "/start" => {
            assert_eq!(req.header("authorization"), Some("Bearer secret-token"));
            Response::new(302).header("location", b_bounce.clone())
        }
        "/final" => {
            assert!(
                req.header("authorization").is_none(),
                "A->B->A tainted hop must not re-send Authorization"
            );
            Response::new(200).body("done")
        }
        "/p1" => Response::new(200)
            .header("link", format!("<{b_p2}>; rel=\"next\""))
            .body("page-1"),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .redirect(Redirect::limit(5))
        .build()
        .unwrap();

    let final_url = origin_a.url("/final");
    let text = session
        .get(origin_a.url("/start"))
        .header("authorization", "Bearer secret-token")
        .header("x-return-to", final_url)
        .text()
        .await
        .unwrap();
    assert_eq!(text, "done");

    let mut pages = session
        .get(origin_a.url("/p1"))
        .header("referer", origin_a.url("/private?secret=1"))
        .pages();
    let p1 = pages.next().await.unwrap().unwrap().text().await.unwrap();
    let p2 = pages.next().await.unwrap().unwrap().text().await.unwrap();
    assert_eq!(p1, "page-1");
    assert_eq!(p2, "page-2");
    assert!(pages.next().await.is_none());
}

#[cfg(feature = "websocket")]
#[tokio::test]
async fn proxy_identified_shares_shutdown_and_isolates_ws_cookies() {
    use base64::Engine;
    use leyline_bssl::hash::{MessageDigest, hash};

    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();
    let conns = Arc::new(AtomicUsize::new(0));
    let slow_started = Arc::new(Notify::new());
    let slow_started_srv = Arc::clone(&slow_started);
    let ws_port = tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::clone(&conns),
        |_| tls_support::HTTP11,
        move |_, mut stream| {
            let slow_started_srv = Arc::clone(&slow_started_srv);
            async move {
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                if req.starts_with("GET /slow ") {
                    slow_started_srv.notify_one();
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    return;
                }
                let key_hdr = req
                    .lines()
                    .find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        k.eq_ignore_ascii_case("sec-websocket-key")
                            .then(|| v.trim().to_owned())
                    })
                    .unwrap();
                let digest = hash(
                    MessageDigest::sha1(),
                    format!("{key_hdr}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
                )
                .unwrap();
                let accept = base64::engine::general_purpose::STANDARD.encode(digest);
                let resp = format!(
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSet-Cookie: ws_id=derived; Path=/\r\n\r\n"
                );
                drop(stream.write_all(resp.as_bytes()).await);
                let mut drain = [0u8; 64];
                drop(stream.read(&mut drain).await);
            }
        },
    )
    .await;

    let (proxy_url, _, _) = spawn_connect_proxy().await;
    let base = Session::builder()
        .browser(Browser::Chrome154)
        .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
        .proxy(Proxy::identified([(
            proxy_url,
            Browser::Chrome154,
            leyline::Platform::Linux,
        )]))
        .build()
        .unwrap();

    let ws_url = format!("wss://127.0.0.1:{ws_port}/ws");
    let https_url = format!("https://127.0.0.1:{ws_port}/ws");
    let ws = base.get(&ws_url).websocket().await.unwrap();
    drop(ws);
    assert!(
        base.cookies().cookies_for(&https_url).is_empty(),
        "Proxy::identified WS Set-Cookie must stay in the derived jar, not base"
    );

    let slow_session = base.clone();
    let slow_url = format!("https://127.0.0.1:{ws_port}/slow");
    let inflight = tokio::spawn(async move { slow_session.get(slow_url).send().await });
    slow_started.notified().await;
    let t0 = Instant::now();
    base.shutdown().await;
    let inflight_err = tokio::time::timeout(Duration::from_secs(2), inflight)
        .await
        .expect("base.shutdown() must abort in-flight derived identity request")
        .unwrap()
        .expect_err("derived request must fail with shutdown");
    assert!(inflight_err.is_shut_down(), "{inflight_err:?}");
    assert!(t0.elapsed() < Duration::from_secs(2));

    let err = base.get(&https_url).send().await.expect_err("shut down");
    assert!(err.is_shut_down(), "{err:?}");
}

#[cfg(all(feature = "http3", feature = "socks"))]
#[tokio::test]
async fn h3_socks5_proxy_preserves_cause_and_rejects_port_zero() {
    async fn spawn_mock_socks5(reply: u8, bind_port: u16) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut greet = [0u8; 3];
                    if sock.read_exact(&mut greet).await.is_err() {
                        return;
                    }
                    drop(sock.write_all(&[0x05, 0x00]).await);
                    let mut head = [0u8; 4];
                    if sock.read_exact(&mut head).await.is_err() {
                        return;
                    }
                    let addr_len = match head[3] {
                        0x01 => 4 + 2,
                        0x04 => 16 + 2,
                        0x03 => {
                            let mut l = [0u8; 1];
                            drop(sock.read_exact(&mut l).await);
                            usize::from(l[0]) + 2
                        }
                        _ => return,
                    };
                    let mut rest = vec![0u8; addr_len];
                    drop(sock.read_exact(&mut rest).await);
                    let port_bytes = bind_port.to_be_bytes();
                    let resp = [
                        0x05,
                        reply,
                        0x00,
                        0x01,
                        127,
                        0,
                        0,
                        1,
                        port_bytes[0],
                        port_bytes[1],
                    ];
                    drop(sock.write_all(&resp).await);
                });
            }
        });
        port
    }

    let unreachable_port = spawn_mock_socks5(0x04, 1080).await;
    let session_a = Session::builder()
        .browser(Browser::Chrome154)
        .protocol(Protocol::Http3)
        .proxy(format!("socks5://127.0.0.1:{unreachable_port}"))
        .build()
        .unwrap();
    let err_a = session_a
        .get("https://127.0.0.1:4433/")
        .send()
        .await
        .expect_err("SOCKS5 reply 0x04");
    assert_eq!(err_a.kind(), Kind::Proxy, "{err_a:?}");
    assert!(err_a.is_proxy(), "{err_a:?}");
    assert!(!err_a.is_retryable(), "{err_a:?}");

    let zero_port = spawn_mock_socks5(0x00, 0).await;
    let session_b = Session::builder()
        .browser(Browser::Chrome154)
        .protocol(Protocol::Http3)
        .proxy(format!("socks5://127.0.0.1:{zero_port}"))
        .build()
        .unwrap();
    let err_b = session_b
        .get("https://127.0.0.1:4433/")
        .send()
        .await
        .expect_err("SOCKS5 UDP port 0");
    assert_eq!(err_b.kind(), Kind::Proxy, "{err_b:?}");
    assert!(err_b.is_proxy(), "{err_b:?}");

    let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = closed_listener.local_addr().unwrap().port();
    drop(closed_listener);
    let session_c = Session::builder()
        .browser(Browser::Chrome154)
        .protocol(Protocol::Http3)
        .proxy(format!("socks5://127.0.0.1:{closed_port}"))
        .build()
        .unwrap();
    let err_c = session_c
        .get("https://127.0.0.1:4433/")
        .send()
        .await
        .expect_err("refused SOCKS5 proxy");
    assert_eq!(err_c.kind(), Kind::Proxy, "{err_c:?}");
    assert!(err_c.is_proxy(), "{err_c:?}");
    assert!(err_c.is_retryable(), "{err_c:?}");
}
