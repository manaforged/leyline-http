use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use leyline::testing::{Response, Server};
use leyline::{Body, Browser, HostLimits, Kind, Platform, Protocol, Proxy, Session, Timeout, Tls};
use leyline_bssl::ssl::{Ssl, SslContextBuilder, SslMethod, SslVersion};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use super::combination_matrix::spawn_connect_proxy;
use super::tls_support;
use super::v030_workloads::TempDir;

#[test]
fn blocking_h1_fixed_length_stream_yields_first_chunk_before_eof() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build server runtime");
    let release = Arc::new(Notify::new());
    let release_srv = Arc::clone(&release);
    let (port, handle) = rt.block_on(async move {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fixed-length listener");
        let port = listener.local_addr().expect("listener addr").port();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept client");
            let mut req = [0u8; 1024];
            drop(sock.read(&mut req).await);
            let head_and_part1 =
                b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\npart1";
            sock.write_all(head_and_part1).await.expect("write part1");
            sock.flush().await.expect("flush part1");
            release_srv.notified().await;
            sock.write_all(b"part2").await.expect("write part2");
            sock.flush().await.expect("flush part2");
        });
        (port, handle)
    });

    let server_thread = std::thread::spawn(move || {
        rt.block_on(handle).expect("server task finished");
    });

    let session = Session::builder()
        .protocol(Protocol::Http1)
        .timeout(Timeout::new().total(Duration::from_secs(5)))
        .build_blocking()
        .expect("build blocking session");
    let resp = session
        .block_on(
            session
                .get(format!("http://127.0.0.1:{port}/fixed"))
                .stream()
                .send(),
        )
        .expect("head arrived");
    let mut stream = resp.bytes_stream().expect("bytes_stream");
    let first = session
        .block_on(stream.next())
        .expect("first chunk present")
        .expect("first chunk ok");
    assert_eq!(&first[..], b"part1");

    release.notify_one();
    let second = session
        .block_on(stream.next())
        .expect("second chunk present")
        .expect("second chunk ok");
    assert_eq!(&second[..], b"part2");
    assert!(session.block_on(stream.next()).is_none());
    server_thread.join().expect("join server thread");
}

#[tokio::test]
async fn proxy_pool_no_proxy_redirect_to_non_bypassed_host_uses_proxy_and_identity() {
    let direct_target_hits = Arc::new(AtomicUsize::new(0));
    let direct_target_bg = Arc::clone(&direct_target_hits);
    let target = Server::http(move |_| {
        direct_target_bg.fetch_add(1, Ordering::SeqCst);
        Response::new(200).body("leaked-direct")
    })
    .await
    .expect("start target server");
    let target_url = target.url("/secret");

    let hop1 = Server::http(move |_| Response::new(302).header("location", target_url.clone()))
        .await
        .expect("start bypassed redirect server");
    let hop1_bypass = format!("127.0.0.1:{}", hop1.addr().port());

    let proxy_hits = Arc::new(AtomicUsize::new(0));
    let proxy_hits_bg = Arc::clone(&proxy_hits);
    let proxy = Server::http(move |req| {
        proxy_hits_bg.fetch_add(1, Ordering::SeqCst);
        let ua = req.header("user-agent").unwrap_or("").to_owned();
        Response::new(200).body(format!("proxied:{ua}"))
    })
    .await
    .expect("start proxy server");
    let proxy_url = format!("http://{}", proxy.addr());

    let list_session = Session::builder()
        .proxy(Proxy::list([proxy_url.as_str()]).no_proxy([hop1_bypass.as_str()]))
        .build()
        .expect("build proxy list session");
    let resp = list_session
        .get(hop1.url("/start"))
        .send()
        .await
        .expect("redirected request succeeds via proxy");
    assert!(
        resp.proxy().is_some(),
        "final response must report the proxy used on the redirected hop"
    );
    let body = resp.text().await.expect("read body");
    assert!(body.starts_with("proxied:"), "got {body}");
    assert_eq!(proxy_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        direct_target_hits.load(Ordering::SeqCst),
        0,
        "target must never be hit directly bypassing the proxy pool"
    );

    let identified_session = Session::builder()
        .browser(Browser::Chrome154)
        .proxy(
            Proxy::identified([(proxy_url.as_str(), Browser::Firefox159, Platform::Linux)])
                .no_proxy([hop1_bypass.as_str()]),
        )
        .build()
        .expect("build identified proxy pool session");
    let id_body = identified_session
        .get(hop1.url("/start"))
        .text()
        .await
        .expect("identified redirect succeeds");
    assert!(
        id_body.contains("Firefox/159"),
        "redirect hop must use the proxy's identified browser UA, got {id_body}"
    );
    assert_eq!(proxy_hits.load(Ordering::SeqCst), 2);
    assert_eq!(direct_target_hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unclean_tls_eof_fails_close_delimited_body_and_download() {
    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().expect("cert der");
    let mut ctx = SslContextBuilder::new(SslMethod::tls()).expect("ssl ctx");
    ctx.set_certificate(&cert).expect("set cert");
    ctx.set_private_key(&key).expect("set key");
    ctx.set_max_proto_version(Some(SslVersion::TLS1_3))
        .expect("tls 1.3");
    let ctx = Arc::new(ctx.build());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tls listener");
    let port = listener.local_addr().expect("addr").port();
    let srv = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let ctx = Arc::clone(&ctx);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).expect("ssl");
                let Ok(mut tls) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                else {
                    return;
                };
                let mut req = [0u8; 2048];
                let n = tls.read(&mut req).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&req[..n]);
                let clean = head.contains("GET /clean ");
                let payload = if clean {
                    b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ncomplete-data".as_slice()
                } else {
                    b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\ntruncated".as_slice()
                };
                drop(tls.write_all(payload).await);
                drop(tls.flush().await);
                if clean {
                    drop(tls.shutdown().await);
                }
            });
        }
    });

    let session = Session::builder()
        .protocol(Protocol::Http1)
        .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
        .build()
        .expect("build tls session");

    let clean_text = session
        .get(format!("https://127.0.0.1:{port}/clean"))
        .text()
        .await
        .expect("clean close_notify succeeds");
    assert_eq!(clean_text, "complete-data");

    let trunc_err = session
        .get(format!("https://127.0.0.1:{port}/abrupt"))
        .text()
        .await
        .expect_err("abrupt TCP close without close_notify must fail");
    assert!(
        matches!(trunc_err.kind(), Kind::Io | Kind::Body),
        "expected Io or Body, got {trunc_err:?}"
    );

    let dir = TempDir::new("unclean-tls-eof");
    let dest = dir.path("out.bin");
    let dl_err = session
        .get(format!("https://127.0.0.1:{port}/abrupt"))
        .download_to(&dest)
        .await
        .expect_err("abrupt TLS close must not save truncated file");
    assert!(
        matches!(dl_err.kind(), Kind::Io | Kind::Body),
        "expected Io or Body, got {dl_err:?}"
    );
    assert!(!dest.exists(), "truncated download must not commit target");
    srv.abort();
}

#[tokio::test]
async fn singleflight_refresh_token_survives_first_caller_timeout() {
    let server = Server::http(|req| {
        if req.header("authorization") == Some("Bearer fresh-token") {
            Response::new(200).body("authorized")
        } else {
            Response::new(401).body("expired")
        }
    })
    .await
    .expect("start auth server");

    let refresh_calls = Arc::new(AtomicUsize::new(0));
    let calls_bg = Arc::clone(&refresh_calls);
    let started = Arc::new(Notify::new());
    let started_bg = Arc::clone(&started);
    let session = Session::builder()
        .base_url(server.url("/"))
        .bearer_auth("stale-token")
        .refresh_token(move || {
            let calls_bg = Arc::clone(&calls_bg);
            let started_bg = Arc::clone(&started_bg);
            async move {
                calls_bg.fetch_add(1, Ordering::SeqCst);
                started_bg.notify_waiters();
                tokio::time::sleep(Duration::from_millis(120)).await;
                Ok("fresh-token".to_owned())
            }
        })
        .build()
        .expect("build refresh session");

    let s1 = session.clone();
    let caller1 = tokio::spawn(async move {
        s1.get("/data")
            .timeout(Timeout::new().total(Duration::from_millis(40)))
            .send()
            .await
    });
    started.notified().await;
    let body2 = session
        .get("/data")
        .timeout(Timeout::new().total(Duration::from_secs(3)))
        .text()
        .await
        .expect("caller 2 must share the in-flight refresh and succeed");
    assert_eq!(body2, "authorized");

    let err1 = caller1
        .await
        .expect("join caller 1")
        .expect_err("caller 1 must hit its 40ms timeout");
    assert_eq!(err1.kind(), Kind::Timeout);
    assert_eq!(
        refresh_calls.load(Ordering::SeqCst),
        1,
        "refresh callback must run exactly once across both callers"
    );
}

#[cfg(feature = "bench-internals")]
#[tokio::test]
async fn h2_idle_reaper_preserves_connection_with_active_open_stream() {
    use super::h2_support;
    use leyline::internals::h2::frame::FrameType;
    use leyline::internals::h2::hpack::Decoder;

    let (cert, key) = tls_support::self_signed();
    let der = cert.to_der().expect("cert der");
    let mut ctx = SslContextBuilder::new(SslMethod::tls()).expect("ssl ctx");
    ctx.set_certificate(&cert).expect("set cert");
    ctx.set_private_key(&key).expect("set key");
    ctx.set_alpn_select_callback(|_, offered| {
        leyline_bssl::ssl::select_next_proto(tls_support::H2, offered)
            .ok_or(leyline_bssl::ssl::AlpnError::NOACK)
    });
    let ctx = Arc::new(ctx.build());

    let release_slow = Arc::new(Notify::new());
    let release_bg = Arc::clone(&release_slow);
    let slow_started = Arc::new(Notify::new());
    let slow_started_bg = Arc::clone(&slow_started);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind h2");
    let port = listener.local_addr().expect("addr").port();
    let srv = tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let ctx = Arc::clone(&ctx);
            let release_bg = Arc::clone(&release_bg);
            let slow_started_bg = Arc::clone(&slow_started_bg);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).expect("ssl");
                let Ok(mut tls) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                else {
                    return;
                };
                h2_support::read_preface(&mut tls).await;
                h2_support::write_server_settings(&mut tls).await;
                let mut decoder = Decoder::new();
                loop {
                    let (hdr, payload) = h2_support::read_frame(&mut tls).await;
                    if hdr.frame_type == FrameType::Settings as u8 && hdr.flags & 0x1 == 0 {
                        h2_support::write_settings_ack(&mut tls).await;
                    } else if hdr.frame_type == FrameType::Headers as u8 {
                        let fields = decoder.decode_header_block(&payload).unwrap_or_default();
                        let path = fields
                            .iter()
                            .find(|h| h.name.as_ref() == b":path")
                            .map(|h| String::from_utf8_lossy(&h.value).into_owned())
                            .unwrap_or_default();
                        if path == "/slow" {
                            slow_started_bg.notify_one();
                            release_bg.notified().await;
                        }
                        h2_support::write_response(&mut tls, hdr.stream_id, b"ok").await;
                    }
                }
            });
        }
    });

    let session = Session::builder()
        .protocol(Protocol::Http2)
        .pool_idle_timeout(Duration::from_millis(80))
        .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
        .build()
        .expect("build h2 session");

    let s1 = session.clone();
    let slow_url = format!("https://127.0.0.1:{port}/slow");
    let slow_task = tokio::spawn(async move { s1.get(&slow_url).text().await });
    slow_started.notified().await;

    tokio::time::sleep(Duration::from_millis(360)).await;
    release_slow.notify_one();
    assert_eq!(slow_task.await.expect("join").expect("slow ok"), "ok");

    let resp2 = session
        .get(format!("https://127.0.0.1:{port}/fast"))
        .send()
        .await
        .expect("second stream ok");
    assert!(
        resp2.reused(),
        "reaper must not evict an H2 connection while a stream is open"
    );
    assert_eq!(session.stats().installs(), 1);
    srv.abort();
}

#[tokio::test]
async fn tls_resumption_key_isolates_proxy_headers() {
    let origin = Server::https(|_| Response::new(200).body("ok"))
        .await
        .expect("start https origin");
    let (proxy_url, _connects, _forwards) = spawn_connect_proxy().await;

    let session_a = Session::builder()
        .browser(Browser::Chrome154)
        .tls(origin.tls())
        .proxy(Proxy::from(proxy_url.as_str()).header("x-tenant", "alpha"))
        .build()
        .expect("build session A");
    assert_eq!(
        session_a.get(origin.url("/a")).text().await.expect("ok"),
        "ok"
    );

    let session_b = Session::builder()
        .browser(Browser::Chrome154)
        .tls(origin.tls())
        .proxy(Proxy::from(proxy_url.as_str()).header("x-tenant", "beta"))
        .build()
        .expect("build session B");
    assert_eq!(
        session_b.get(origin.url("/b")).text().await.expect("ok"),
        "ok"
    );

    let key_a =
        serde_json::to_value(&session_a).expect("json A")["state"]["tls_sessions"][0]["key"]
            .as_str()
            .expect("saved key A")
            .to_owned();
    let key_b =
        serde_json::to_value(&session_b).expect("json B")["state"]["tls_sessions"][0]["key"]
            .as_str()
            .expect("saved key B")
            .to_owned();
    assert!(key_a.contains("#proxy-identity="), "{key_a}");
    assert_ne!(
        key_a, key_b,
        "TLS resumption keys must differ when proxy headers differ"
    );
}

#[tokio::test]
async fn download_to_status_600_maps_to_kind_status_and_fifo_body_file_fails_without_blocking() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind status-600 server");
    let port = listener.local_addr().expect("addr").port();
    let srv = tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let mut req = [0u8; 1024];
            drop(sock.read(&mut req).await);
            drop(
                sock.write_all(
                    b"HTTP/1.1 600 Custom\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad",
                )
                .await,
            );
        }
    });

    let dir = TempDir::new("status600-fifo");
    let dest = dir.path("out.bin");
    let err = Session::new()
        .get(format!("http://127.0.0.1:{port}/"))
        .download_to(&dest)
        .await
        .expect_err("status 600 must fail download_to");
    assert_eq!(
        err.kind(),
        Kind::Status,
        "expected Kind::Status, got {err:?}"
    );
    assert_eq!(err.status().map(|s| s.as_u16()), Some(600));
    srv.abort();

    #[cfg(unix)]
    {
        let fifo = dir.path("named.pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("run mkfifo");
        assert!(status.success(), "mkfifo failed");
        let start = std::time::Instant::now();
        let err = Body::file(&fifo).expect_err("FIFO must be rejected without blocking");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "Body::file blocked on FIFO"
        );
        assert!(
            matches!(err.kind(), Kind::Io | Kind::Request),
            "unexpected kind: {err:?}"
        );
    }
}

#[tokio::test]
async fn h2_empty_post_preserves_content_length_zero_and_pages_upgrades_hsts_next_link() {
    #[cfg(feature = "bench-internals")]
    {
        let (cert, key) = tls_support::self_signed();
        let der = cert.to_der().expect("cert der");
        let seen_cl = Arc::new(std::sync::Mutex::new(None::<String>));
        let seen_cl_bg = Arc::clone(&seen_cl);

        let mut ctx = SslContextBuilder::new(SslMethod::tls()).expect("ssl ctx");
        ctx.set_certificate(&cert).expect("set cert");
        ctx.set_private_key(&key).expect("set key");
        ctx.set_alpn_select_callback(|_, offered| {
            leyline_bssl::ssl::select_next_proto(tls_support::H2, offered)
                .ok_or(leyline_bssl::ssl::AlpnError::NOACK)
        });
        let ctx = Arc::new(ctx.build());
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind h2");
        let port = listener.local_addr().expect("addr").port();
        let srv = tokio::spawn(async move {
            use super::h2_support;
            use leyline::internals::h2::frame::FrameType;
            use leyline::internals::h2::hpack::Decoder;

            if let Ok((tcp, _)) = listener.accept().await {
                let ssl = Ssl::new(&ctx).expect("ssl");
                let Ok(mut tls) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                else {
                    return;
                };
                h2_support::read_preface(&mut tls).await;
                h2_support::write_server_settings(&mut tls).await;
                let mut decoder = Decoder::new();
                loop {
                    let (hdr, payload) = h2_support::read_frame(&mut tls).await;
                    if hdr.frame_type == FrameType::Settings as u8 && hdr.flags & 0x1 == 0 {
                        h2_support::write_settings_ack(&mut tls).await;
                    } else if hdr.frame_type == FrameType::Headers as u8 {
                        let fields = decoder.decode_header_block(&payload).unwrap_or_default();
                        if let Some(h) =
                            fields.iter().find(|h| h.name.as_ref() == b"content-length")
                        {
                            *seen_cl_bg.lock().expect("lock") =
                                Some(String::from_utf8_lossy(&h.value).into_owned());
                        }
                        h2_support::write_response(&mut tls, hdr.stream_id, b"ok").await;
                        break;
                    }
                }
            }
        });

        let session = Session::builder()
            .protocol(Protocol::Http2)
            .tls(Tls::new().env_roots(false).system_roots(false).ca_der(der))
            .build()
            .expect("build h2 session");
        let resp = session
            .post(format!("https://127.0.0.1:{port}/empty"))
            .text()
            .await
            .expect("empty POST succeeds");
        assert_eq!(resp, "ok");
        assert_eq!(
            seen_cl.lock().expect("lock").as_deref(),
            Some("0"),
            "empty H2 POST must send content-length: 0"
        );
        srv.abort();
    }

    let (pages_cert, pages_key) = tls_support::self_signed_dns("hsts-pages.test");
    let pages_der = pages_cert.to_der().expect("pages cert der");
    let pages_conns = Arc::new(AtomicUsize::new(0));
    let https_port = tls_support::tls_server_per_connection(
        pages_cert,
        pages_key,
        Arc::clone(&pages_conns),
        |_| tls_support::HTTP11,
        |_, mut stream| async move {
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]);
            let resp = if head.contains("GET /p1 ") {
                let host = head
                    .lines()
                    .find_map(|l| l.strip_prefix("host: ").or_else(|| l.strip_prefix("Host: ")))
                    .unwrap_or("hsts-pages.test")
                    .trim();
                format!(
                    "HTTP/1.1 200 OK\r\nStrict-Transport-Security: max-age=3600\r\nLink: <http://{host}/p2>; rel=\"next\"\r\nContent-Length: 5\r\nConnection: close\r\n\r\npage1"
                )
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\npage2".to_owned()
            };
            drop(stream.write_all(resp.as_bytes()).await);
            drop(stream.shutdown().await);
        },
    )
    .await;
    let session = Session::builder()
        .resolve(
            "hsts-pages.test",
            [std::net::SocketAddr::from(([127, 0, 0, 1], https_port))],
        )
        .tls(
            Tls::new()
                .env_roots(false)
                .system_roots(false)
                .ca_der(pages_der),
        )
        .build()
        .expect("build pages session");
    let mut pages = session
        .get(format!("https://hsts-pages.test:{https_port}/p1"))
        .pages();
    let first = pages.next().await.expect("page 1").expect("page 1 ok");
    assert_eq!(first.text().await.expect("body 1"), "page1");
    let second = pages
        .next()
        .await
        .expect("http:// next link must HSTS-upgrade instead of stopping")
        .expect("page 2 ok");
    assert_eq!(second.url().scheme(), "https");
    assert_eq!(second.text().await.expect("body 2"), "page2");
    assert!(pages.next().await.is_none());
}

#[tokio::test]
async fn download_options_fail_on_non_download_sends_and_host_limits_owns_h1_pending_queue() {
    let server = Server::http(|_| Response::new(200).body("ok"))
        .await
        .expect("start server");
    let session = Session::new();

    for err in [
        session
            .get(server.url("/a"))
            .sha256([0u8; 32])
            .send()
            .await
            .expect_err("send with sha256"),
        session
            .get(server.url("/b"))
            .progress(|_, _| {})
            .text()
            .await
            .expect_err("text with progress"),
        session
            .get(server.url("/c"))
            .resume(false)
            .bytes()
            .await
            .expect_err("bytes with resume"),
        session
            .get(server.url("/d"))
            .sha256([1u8; 32])
            .events()
            .expect_err("events with sha256"),
    ] {
        assert_eq!(err.kind(), Kind::Config, "expected Config, got {err:?}");
    }

    let mut pages = session.get(server.url("/e")).sha256([2u8; 32]).pages();
    let page_err = pages
        .next()
        .await
        .expect("pages yields error")
        .expect_err("pages with sha256");
    assert_eq!(page_err.kind(), Kind::Config);

    let tab = session.tab();
    let target = leyline::Url::parse(&server.url("/tab")).expect("valid url");
    tab.page(target.clone());
    assert_eq!(tab.current().as_ref(), Some(&target));
    tab.page(None);
    assert_eq!(tab.current(), None);

    #[cfg(feature = "http3")]
    {
        let bad_h3 = Session::builder()
            .protocol(Protocol::Http3)
            .proxy(
                Proxy::new()
                    .all("socks5h://127.0.0.1:1080")
                    .https("http://127.0.0.1:8080"),
            )
            .build()
            .expect_err("HTTP/3 with https http:// proxy override must fail build");
        assert_eq!(bad_h3.kind(), Kind::Config);
    }

    let gate = Arc::new(Notify::new());
    let gate_bg = Arc::clone(&gate);
    let arrived = Arc::new(Notify::new());
    let arrived_bg = Arc::clone(&arrived);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind h1");
    let h1_port = listener.local_addr().expect("addr").port();
    let srv = tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            drop(sock.read(&mut buf).await);
            arrived_bg.notify_one();
            gate_bg.notified().await;
            drop(
                sock.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                )
                .await,
            );
        }
    });

    let limited = Session::builder()
        .protocol(Protocol::Http1)
        .max_connections_per_host(1)
        .host_limits(HostLimits::new().max_pending(0))
        .build()
        .expect("build limited session");
    let url = format!("http://127.0.0.1:{h1_port}/slow");
    let s1 = limited.clone();
    let u1 = url.clone();
    let first = tokio::spawn(async move { s1.get(&u1).text().await });
    arrived.notified().await;

    let overloaded = limited
        .get(&url)
        .send()
        .await
        .expect_err("second H1 request must fail with Kind::Overloaded under max_pending(0)");
    assert_eq!(overloaded.kind(), Kind::Overloaded);
    gate.notify_one();
    assert_eq!(first.await.expect("join").expect("first ok"), "ok");
    srv.abort();
}
