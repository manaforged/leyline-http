use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream;
#[cfg(feature = "compression-gzip")]
use http::StatusCode;
use leyline::testing::{Response, Server};
use leyline::{Body, Browser, Kind, Protocol, Proxy, Retry, Session, Timeout, Tls};
use leyline_bssl::ssl::{Ssl, SslContextBuilder, SslMethod, SslVerifyMode, SslVersion};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use super::combination_matrix::spawn_connect_proxy;
use super::tls_support;

#[cfg(feature = "compression-gzip")]
fn gzip_padding_bomb(padding_bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(padding_bytes + 64);
    out.extend_from_slice(&[0x1f, 0x8b, 0x08, 0x10, 0, 0, 0, 0, 0, 0xff]);
    out.extend(std::iter::repeat_n(b'A', padding_bytes));
    out.push(0);
    let inner = super::v030_workloads::gzip_bytes(b"ok");
    out.extend_from_slice(&inner[10..]);
    out
}

#[tokio::test]
async fn transport_matrix_tls_alert_parity_across_h1_tls12_h2_and_h3() {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().unwrap();

    for (label, proto) in [
        ("h1-tls13-mtls", Protocol::Http1),
        ("h2-tls13-mtls", Protocol::Http2),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
        ctx.set_certificate(&cert).unwrap();
        ctx.set_private_key(&key).unwrap();
        ctx.set_max_proto_version(Some(SslVersion::TLS1_3)).unwrap();
        ctx.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
        if matches!(proto, Protocol::Http2) {
            ctx.set_alpn_select_callback(|_, offered| {
                leyline_bssl::ssl::select_next_proto(tls_support::H2, offered)
                    .ok_or(leyline_bssl::ssl::AlpnError::NOACK)
            });
        }
        let ctx = ctx.build();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_bg = Arc::clone(&accepts);
        let server = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                accepts_bg.fetch_add(1, Ordering::SeqCst);
                let ssl = Ssl::new(&ctx).unwrap();
                drop(
                    leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                        .accept()
                        .await,
                );
            }
        });

        let session = Session::builder()
            .browser(Browser::Chrome154)
            .protocol(proto)
            .retry(2)
            .tls(
                Tls::new()
                    .env_roots(false)
                    .system_roots(false)
                    .ca_der(der.clone()),
            )
            .timeout(Timeout::new().total(Duration::from_secs(5)))
            .build()
            .unwrap();

        let err = session
            .get(format!("https://127.0.0.1:{port}/"))
            .send()
            .await
            .expect_err(label);
        server.abort();

        assert_eq!(err.kind(), Kind::Tls, "{label}: {err:?}");
        assert!(!err.is_retryable(), "{label}: {err:?}");
        assert_eq!(err.attempts(), 1, "{label}: {err:?}");
        assert_eq!(
            accepts.load(Ordering::SeqCst),
            1,
            "{label}: retried on wire"
        );
    }

    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
        ctx.set_certificate(&cert).unwrap();
        ctx.set_private_key(&key).unwrap();
        ctx.set_min_proto_version(Some(SslVersion::TLS1_2)).unwrap();
        ctx.set_max_proto_version(Some(SslVersion::TLS1_2)).unwrap();
        let ctx = ctx.build();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepts_bg = Arc::clone(&accepts);
        let server = tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                accepts_bg.fetch_add(1, Ordering::SeqCst);
                let ssl = Ssl::new(&ctx).unwrap();
                if let Ok(mut tls) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                {
                    let mut req = [0u8; 2048];
                    drop(tls.read(&mut req).await);
                    let forged = [
                        0x17, 0x03, 0x03, 0x00, 0x20, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
                        0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
                        0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa,
                        0xaa,
                    ];
                    drop(tls.get_mut().write_all(&forged).await);
                    drop(tls.get_mut().flush().await);
                }
            }
        });

        let session = Session::builder()
            .browser(Browser::Chrome154)
            .protocol(Protocol::Http1)
            .retry(2)
            .tls(
                Tls::new()
                    .env_roots(false)
                    .system_roots(false)
                    .ca_der(der.clone()),
            )
            .timeout(Timeout::new().total(Duration::from_secs(5)))
            .build()
            .unwrap();

        let err = session
            .get(format!("https://127.0.0.1:{port}/"))
            .send()
            .await
            .expect_err("h1-tls12-post-handshake-alert");
        server.abort();

        assert_eq!(err.kind(), Kind::Tls, "h1-tls12: {err:?}");
        assert!(!err.is_retryable(), "h1-tls12: {err:?}");
        assert_eq!(err.attempts(), 1, "h1-tls12: {err:?}");
        assert_eq!(accepts.load(Ordering::SeqCst), 1, "h1-tls12: retried");
    }

    #[cfg(feature = "http3")]
    {
        let h3 = super::h3_support::h3_server_with_tls(
            cert,
            key,
            |tls| {
                tls.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
            },
            vec![super::h3_support::Reply::Body(4)],
            super::h3_support::Limits::default(),
        )
        .await;
        let session = h3.session().retry(2).build().unwrap();
        let err = session.get(h3.url()).send().await.expect_err("h3-mtls");
        assert_eq!(err.kind(), Kind::Tls, "h3-mtls: {err:?}");
        assert!(!err.is_retryable(), "h3-mtls: {err:?}");
        assert_eq!(err.attempts(), 1, "h3-mtls: {err:?}");
    }
}

#[tokio::test]
async fn blocking_session_streams_live_sse_and_chunked_without_hanging() {
    let release_sse = Arc::new(Notify::new());
    let release_sse_srv = Arc::clone(&release_sse);
    let sse_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sse_port = sse_listener.local_addr().unwrap().port();
    let sse_srv = tokio::spawn(async move {
        let (mut sock, _) = sse_listener.accept().await.unwrap();
        let mut req = [0u8; 1024];
        drop(sock.read(&mut req).await);
        let head = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/event-stream\r\n",
            "Transfer-Encoding: chunked\r\n\r\n",
            "17\r\nevent: tick\ndata: one\n\n\r\n",
        );
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
        release_sse_srv.notified().await;
        sock.write_all(b"17\r\nevent: tick\ndata: two\n\n\r\n0\r\n\r\n")
            .await
            .unwrap();
        sock.flush().await.unwrap();
    });

    let release_chunk = Arc::new(Notify::new());
    let release_chunk_srv = Arc::clone(&release_chunk);
    let chunk_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let chunk_port = chunk_listener.local_addr().unwrap().port();
    let chunk_srv = tokio::spawn(async move {
        let (mut sock, _) = chunk_listener.accept().await.unwrap();
        let mut req = [0u8; 1024];
        drop(sock.read(&mut req).await);
        let head = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Transfer-Encoding: chunked\r\n\r\n",
            "5\r\nalpha\r\n",
        );
        sock.write_all(head.as_bytes()).await.unwrap();
        sock.flush().await.unwrap();
        release_chunk_srv.notified().await;
        sock.write_all(b"4\r\nbeta\r\n0\r\n\r\n").await.unwrap();
        sock.flush().await.unwrap();
    });

    let blocking_task = tokio::task::spawn_blocking(move || {
        let session = Session::builder()
            .browser(Browser::Chrome154)
            .protocol(Protocol::Http1)
            .timeout(Timeout::new().total(Duration::from_secs(5)))
            .build_blocking()
            .unwrap();

        let mut events = session
            .get(format!("http://127.0.0.1:{sse_port}/events"))
            .events()
            .unwrap()
            .retry(0);
        let first = session
            .block_on(events.next())
            .expect("first SSE event")
            .expect("first SSE ok");
        assert_eq!(first.event(), "tick");
        assert_eq!(first.data(), "one");
        release_sse.notify_one();

        let second = session
            .block_on(events.next())
            .expect("second SSE event")
            .expect("second SSE ok");
        assert_eq!(second.data(), "two");

        let resp = session
            .block_on(
                session
                    .get(format!("http://127.0.0.1:{chunk_port}/stream"))
                    .stream()
                    .send(),
            )
            .unwrap();
        let mut body = resp.bytes_stream().unwrap();
        let c1 = session
            .block_on(body.next())
            .expect("chunk 1")
            .expect("chunk 1 ok");
        assert_eq!(&c1[..], b"alpha");
        release_chunk.notify_one();

        let c2 = session
            .block_on(body.next())
            .expect("chunk 2")
            .expect("chunk 2 ok");
        assert_eq!(&c2[..], b"beta");
        assert!(session.block_on(body.next()).is_none());
    });

    tokio::time::timeout(Duration::from_secs(4), blocking_task)
        .await
        .expect("blocking stream must yield chunk 1 without deadlocking")
        .unwrap();
    sse_srv.await.unwrap();
    chunk_srv.await.unwrap();
}

#[tokio::test]
async fn body_stream_local_io_errors_stay_non_retryable_kind_body() {
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_bg = Arc::clone(&hits);
    let server = Server::http(move |_| {
        hits_bg.fetch_add(1, Ordering::SeqCst);
        Response::new(200).body("ok")
    })
    .await
    .unwrap();

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .retry(Retry::new(2).any_method(true))
        .build()
        .unwrap();

    for io_kind in [
        io::ErrorKind::TimedOut,
        io::ErrorKind::ConnectionRefused,
        io::ErrorKind::UnexpectedEof,
    ] {
        let chunks = vec![
            Ok(Bytes::from_static(b"partial")),
            Err(io::Error::new(io_kind, "local producer failed")),
        ];
        let err = session
            .post(server.url("/upload"))
            .body(Body::stream(stream::iter(chunks), None))
            .send()
            .await
            .expect_err("local body stream error must fail");
        assert_eq!(err.kind(), Kind::Body, "{io_kind:?}: {err:?}");
        assert!(!err.is_timeout(), "{io_kind:?}: {err:?}");
        assert!(!err.is_connect(), "{io_kind:?}: {err:?}");
        assert!(!err.is_retryable(), "{io_kind:?}: {err:?}");
        assert_eq!(err.attempts(), 1, "{io_kind:?}: {err:?}");
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[cfg(feature = "compression-gzip")]
#[tokio::test]
async fn streamed_gzip_padding_bomb_respects_max_body_size() {
    let bomb = Arc::new(gzip_padding_bomb(16 * 1024));
    let bomb_bg = Arc::clone(&bomb);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let srv = tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let bomb = Arc::clone(&bomb_bg);
            tokio::spawn(async move {
                let mut req = [0u8; 2048];
                let n = sock.read(&mut req).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&req[..n]);
                let status = if head.starts_with("GET /err ") {
                    "500 Internal Server Error"
                } else {
                    "200 OK"
                };
                let resp_head = format!(
                    "HTTP/1.1 {status}\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                );
                if sock.write_all(resp_head.as_bytes()).await.is_err() {
                    return;
                }
                for piece in bomb.chunks(1024) {
                    let hdr = format!("{:x}\r\n", piece.len());
                    if sock.write_all(hdr.as_bytes()).await.is_err()
                        || sock.write_all(piece).await.is_err()
                        || sock.write_all(b"\r\n").await.is_err()
                        || sock.flush().await.is_err()
                    {
                        return;
                    }
                    tokio::task::yield_now().await;
                }
                drop(sock.write_all(b"0\r\n\r\n").await);
            });
        }
    });

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .max_body_size(4 * 1024)
        .build()
        .unwrap();

    let resp = session
        .get(format!("http://127.0.0.1:{port}/ok"))
        .stream()
        .send()
        .await
        .unwrap();
    let mut bs = resp.bytes_stream().unwrap();
    let mut got_limit = false;
    while let Some(item) = bs.next().await {
        if let Err(err) = item {
            assert_eq!(err.kind(), Kind::BodyLimit, "bytes_stream: {err:?}");
            got_limit = true;
            break;
        }
    }
    assert!(got_limit, "bytes_stream must abort on compressed wire cap");

    let resp = session
        .get(format!("http://127.0.0.1:{port}/ok"))
        .stream()
        .send()
        .await
        .unwrap();
    let err = resp
        .read_until(128, |buf, _| buf.windows(8).any(|w| w == b"sentinel"))
        .await
        .expect_err("read_until must enforce wire cap on gzip padding bomb");
    assert_eq!(err.kind(), Kind::BodyLimit, "read_until: {err:?}");

    let err = session
        .get(format!("http://127.0.0.1:{port}/err"))
        .error_for_status()
        .send()
        .await
        .expect_err("error_for_status returns Kind::Status");
    assert_eq!(err.kind(), Kind::Status, "error_for_status: {err:?}");
    assert_eq!(err.status(), Some(StatusCode::INTERNAL_SERVER_ERROR));
    srv.abort();
}

#[tokio::test]
async fn hsts_is_not_stored_when_invalid_hostnames_accepted() {
    let (wrong_cert, wrong_key) = tls_support::self_signed_dns("other-host.test");
    let wrong_der = wrong_cert.to_der().unwrap();
    let conns = Arc::new(AtomicUsize::new(0));
    let https_port = tls_support::tls_server_per_connection(
        wrong_cert,
        wrong_key,
        Arc::clone(&conns),
        |_| tls_support::HTTP11,
        |_, mut stream| async move {
            let mut buf = [0u8; 2048];
            drop(stream.read(&mut buf).await);
            let resp = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Strict-Transport-Security: max-age=3600\r\n",
                "Content-Length: 2\r\n",
                "Connection: close\r\n\r\nok",
            );
            drop(stream.write_all(resp.as_bytes()).await);
            drop(stream.shutdown().await);
        },
    )
    .await;

    let http_hits = Arc::new(AtomicUsize::new(0));
    let http_hits_bg = Arc::clone(&http_hits);
    let plain = Server::http(move |_| {
        http_hits_bg.fetch_add(1, Ordering::SeqCst);
        Response::new(200).body("plain")
    })
    .await
    .unwrap();
    let http_port = plain.addr().port();

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .resolve(
            "hsts-bad-host.test",
            [std::net::SocketAddr::from(([127, 0, 0, 1], https_port))],
        )
        .tls(
            Tls::new()
                .env_roots(false)
                .system_roots(false)
                .ca_der(wrong_der)
                .danger_accept_invalid_hostnames(true),
        )
        .build()
        .unwrap();

    let ok = session
        .get(format!("https://hsts-bad-host.test:{https_port}/seed"))
        .text()
        .await
        .unwrap();
    assert_eq!(ok, "ok");

    let plain_resp = session
        .get(format!("http://hsts-bad-host.test:{http_port}/check"))
        .send()
        .await
        .unwrap();
    assert_eq!(plain_resp.url().scheme(), "http");
    assert_eq!(plain_resp.text().await.unwrap(), "plain");
    assert_eq!(http_hits.load(Ordering::SeqCst), 1);

    let (good_cert, good_key) = tls_support::self_signed_dns("hsts-good-host.test");
    let good_der = good_cert.to_der().unwrap();
    let good_port = tls_support::tls_server_per_connection(
        good_cert,
        good_key,
        Arc::new(AtomicUsize::new(0)),
        |_| tls_support::HTTP11,
        |_, mut stream| async move {
            let mut buf = [0u8; 2048];
            drop(stream.read(&mut buf).await);
            let resp = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Strict-Transport-Security: max-age=3600\r\n",
                "Content-Length: 2\r\n",
                "Connection: close\r\n\r\nok",
            );
            drop(stream.write_all(resp.as_bytes()).await);
            drop(stream.shutdown().await);
        },
    )
    .await;
    let good_session = Session::builder()
        .browser(Browser::Chrome154)
        .resolve(
            "hsts-good-host.test",
            [std::net::SocketAddr::from(([127, 0, 0, 1], good_port))],
        )
        .tls(
            Tls::new()
                .env_roots(false)
                .system_roots(false)
                .ca_der(good_der),
        )
        .build()
        .unwrap();
    assert_eq!(
        good_session
            .get(format!("https://hsts-good-host.test:{good_port}/seed"))
            .text()
            .await
            .unwrap(),
        "ok"
    );
    let upgraded = good_session
        .get(format!("http://hsts-good-host.test:{good_port}/upgraded"))
        .send()
        .await
        .unwrap();
    assert_eq!(upgraded.url().scheme(), "https");
}

#[tokio::test]
async fn proxy_precedence_and_port_scoped_no_proxy() {
    let origin_a = Server::http(|_| Response::new(200).body("origin-a"))
        .await
        .unwrap();
    let origin_b = Server::http(|_| Response::new(200).body("origin-b"))
        .await
        .unwrap();
    let tls_origin = Server::https(|_| Response::new(200).body("tls-origin"))
        .await
        .unwrap();

    let (proxy_a, a_connect, a_forward) = spawn_connect_proxy().await;
    let (proxy_b, b_connect, b_forward) = spawn_connect_proxy().await;

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .tls(
            Tls::new()
                .env_roots(false)
                .system_roots(false)
                .ca_der(tls_origin.ca_der().unwrap().to_vec()),
        )
        .proxy(
            Proxy::new()
                .all(proxy_a.clone())
                .https(proxy_b.clone())
                .header("x-edge", "cf-pop")
                .no_proxy([format!("127.0.0.1:{}", origin_a.addr().port())]),
        )
        .build()
        .unwrap();

    assert_eq!(
        session.get(origin_a.url("/")).text().await.unwrap(),
        "origin-a"
    );
    assert_eq!(a_forward.load(Ordering::SeqCst), 0);

    assert_eq!(
        session.get(origin_b.url("/")).text().await.unwrap(),
        "origin-b"
    );
    assert_eq!(a_forward.load(Ordering::SeqCst), 1);
    assert_eq!(b_forward.load(Ordering::SeqCst), 0);

    assert_eq!(
        session.get(tls_origin.url("/")).text().await.unwrap(),
        "tls-origin"
    );
    assert_eq!(a_connect.load(Ordering::SeqCst), 0);
    assert_eq!(b_connect.load(Ordering::SeqCst), 1);

    let reverse = Session::builder()
        .browser(Browser::Chrome154)
        .proxy(
            Proxy::new()
                .https(proxy_b.clone())
                .all(proxy_a.clone())
                .header("x-edge", "cf-pop"),
        )
        .build()
        .unwrap();
    assert_eq!(
        reverse.get(origin_b.url("/")).text().await.unwrap(),
        "origin-b"
    );
    assert_eq!(a_forward.load(Ordering::SeqCst), 2);
    assert_eq!(b_forward.load(Ordering::SeqCst), 0);

    let replaced = Session::builder()
        .browser(Browser::Chrome154)
        .proxy(
            Proxy::new()
                .http(proxy_a)
                .http(proxy_b)
                .header("x-edge", "cf-pop"),
        )
        .build()
        .unwrap();
    assert_eq!(
        replaced.get(origin_b.url("/")).text().await.unwrap(),
        "origin-b"
    );
    assert_eq!(a_forward.load(Ordering::SeqCst), 2);
    assert_eq!(b_forward.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pool_idle_timeout_evicts_on_checkout_before_reaper_tick() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let accepted_bg = Arc::clone(&accepted);
    let srv = tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            accepted_bg.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
                            if sock.write_all(resp.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .protocol(Protocol::Http1)
        .pool_idle_timeout(Duration::from_millis(80))
        .build()
        .unwrap();

    assert_eq!(
        session
            .get(format!("http://127.0.0.1:{port}/1"))
            .text()
            .await
            .unwrap(),
        "ok"
    );
    tokio::time::sleep(Duration::from_millis(110)).await;
    assert_eq!(
        session
            .get(format!("http://127.0.0.1:{port}/2"))
            .text()
            .await
            .unwrap(),
        "ok"
    );
    srv.abort();

    assert_eq!(accepted.load(Ordering::SeqCst), 2);
    assert!(session.stats().evictions_idle() >= 1);

    #[cfg(feature = "bench-internals")]
    {
        let (h2_port, der) = super::h2_support::spawn_h2_static_server(b"h2-ok").await;
        let h2_session = Session::builder()
            .browser(Browser::Chrome154)
            .protocol(Protocol::Http2)
            .pool_idle_timeout(Duration::from_millis(80))
            .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
            .build()
            .unwrap();
        let url = format!("https://127.0.0.1:{h2_port}/");
        assert_eq!(h2_session.get(&url).text().await.unwrap(), "h2-ok");
        assert_eq!(h2_session.get(&url).text().await.unwrap(), "h2-ok");
        let mid = h2_session.stats();
        assert_eq!(mid.hits(http::Version::HTTP_2), 1, "mid stats: {mid:?}");
        assert_eq!(mid.installs(), 1, "mid stats: {mid:?}");
        tokio::time::sleep(Duration::from_millis(110)).await;
        assert_eq!(h2_session.get(&url).text().await.unwrap(), "h2-ok");
        let after = h2_session.stats();
        assert_eq!(after.installs(), 2, "after stats: {after:?}");
        assert!(after.evictions_idle() >= 1, "after stats: {after:?}");
    }
}
