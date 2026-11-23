use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use leyline::cookie::{Cookie, Jar};
use leyline::testing::{Response, Server};
use leyline::{Browser, Kind, Protocol, Proxy, Retry, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn download_to_does_not_multiply_retries_and_cleans_lock_file() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    let server = Server::http(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        Response::new(503).body("busy")
    })
    .await
    .unwrap();

    let dir = std::env::temp_dir().join(format!("leyline-dl-retry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("fail.bin");

    let session = Session::builder().build().unwrap();
    let err = session
        .get(server.url("/file"))
        .retry(Retry::new(2).backoff(Duration::from_millis(1), Duration::from_millis(2)))
        .download_to(&target)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Status);
    assert_eq!(hits.load(Ordering::SeqCst), 3);

    let ok_server = Server::http(|_| Response::new(200).body("hello"))
        .await
        .unwrap();
    let ok_target = dir.join("ok.bin");
    let written = session
        .get(ok_server.url("/ok"))
        .download_to(&ok_target)
        .await
        .unwrap();
    assert_eq!(written, 5);
    assert!(!dir.join("ok.bin.leyline.lock").exists());
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn events_does_not_multiply_retries() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    let server = Server::http(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        Response::new(503).body("busy")
    })
    .await
    .unwrap();

    let session = Session::builder()
        .retry(Retry::new(2).backoff(Duration::from_millis(1), Duration::from_millis(2)))
        .build()
        .unwrap();
    let mut events = session
        .get(server.url("/sse"))
        .events()
        .unwrap()
        .retry(Retry::new(2).backoff(Duration::from_millis(1), Duration::from_millis(2)));
    let err = events.next().await.unwrap().unwrap_err();
    assert_eq!(err.kind(), Kind::Status);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn autosave_does_not_panic_on_zero_or_max_duration() {
    let dir = std::env::temp_dir().join(format!("leyline-autosave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let session = Session::builder().build().unwrap();
    session.autosave(dir.join("zero.json"), Duration::ZERO);
    session.autosave(dir.join("max.json"), Duration::MAX);
    session.shutdown().await;
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn protocol_override_mutation_still_invalidates_http_cache() {
    let gets = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&gets);
    let server = Server::http(move |req| {
        if req.method() == "POST" {
            return Response::new(200).body("updated");
        }
        let n = count.fetch_add(1, Ordering::SeqCst) + 1;
        Response::new(200)
            .header("cache-control", "max-age=60")
            .body(format!("v{n}"))
    })
    .await
    .unwrap();

    let session = Session::builder().cache(1024 * 1024).build().unwrap();
    assert_eq!(session.get(server.url("/item")).text().await.unwrap(), "v1");
    assert_eq!(session.get(server.url("/item")).text().await.unwrap(), "v1");
    session
        .post(server.url("/item"))
        .protocol(Protocol::Http1)
        .send()
        .await
        .unwrap();
    assert_eq!(session.get(server.url("/item")).text().await.unwrap(), "v2");
    assert_eq!(gets.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn before_send_method_change_updates_retry_idempotency() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    let server = Server::http(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        Response::new(503).body("busy")
    })
    .await
    .unwrap();

    let session = Session::builder()
        .retry(Retry::new(2).backoff(Duration::from_millis(1), Duration::from_millis(2)))
        .before_send(|req| {
            *req.method_mut() = http::Method::POST;
            Ok(())
        })
        .build()
        .unwrap();
    let resp = session.get(server.url("/mutate")).send().await.unwrap();
    assert_eq!(resp.status(), 503);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn request_from_preserves_target_session_retry_when_unset() {
    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&hits);
    let server = Server::http(move |_| {
        let n = seen.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            Response::new(503).body("retry")
        } else {
            Response::new(200).body("ok")
        }
    })
    .await
    .unwrap();

    let plain = Session::builder().build().unwrap();
    let retrying = Session::builder()
        .retry(Retry::new(2).backoff(Duration::from_millis(1), Duration::from_millis(2)))
        .build()
        .unwrap();
    let built = plain.get(server.url("/interop")).build().unwrap();
    let body = retrying.request_from(built).text().await.unwrap();
    assert_eq!(body, "ok");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn custom_proxy_invalid_return_is_invoked_only_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let session = Session::builder()
        .proxy(Proxy::custom(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Some(Proxy::from("not a valid proxy url"))
        }))
        .build()
        .unwrap();
    let err = session.get("http://127.0.0.1:1/").send().await.unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "websocket")]
#[tokio::test]
async fn websocket_rejects_plaintext_ws_when_https_only() {
    let session = Session::builder().https_only(true).build().unwrap();
    let err = session
        .get("ws://127.0.0.1:1/ws")
        .websocket()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Request);
}

#[tokio::test]
async fn pages_tracks_query_url_and_strips_post_body_headers() {
    let server = Server::http(|req| {
        if req.target() == "/items?page=1" {
            Response::new(200)
                .header("link", "</items>; rel=\"next\"")
                .body("p1")
        } else {
            let ct = req.header("content-type").unwrap_or("none");
            Response::new(200).body(format!("p2:{ct}"))
        }
    })
    .await
    .unwrap();

    let session = Session::builder().build().unwrap();
    let mut pages = session
        .post(server.url("/items"))
        .query([("page", "1")])
        .header("content-type", "application/json")
        .body("{}")
        .pages();
    let first = pages.next().await.unwrap().unwrap().text().await.unwrap();
    let second = pages.next().await.unwrap().unwrap().text().await.unwrap();
    assert_eq!(first, "p1");
    assert_eq!(second, "p2:none");
    assert!(pages.next().await.is_none());
}

#[test]
fn ja4_r_layering_accepts_padding_and_clears_pre_shared_key() {
    let ja4_r = "t13d1516h2_002f,0035,009c,009d,1301,1302,1303,c02b,c02c,c02f,c030,cca8,cca9_0005,000a,000b,000d,0012,0015,0017,001b,0023,002b,002d,0033,ff01_0403,0804,0401,0503,0805,0501,0806,0601";
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .ja4_r(ja4_r)
        .build()
        .unwrap();
    let profile = session.profile().unwrap();
    assert!(
        profile.contains("[[fingerprint]]") && profile.contains(ja4_r),
        "expected ja4_r overlay in exported profile TOML: {profile}"
    );
    #[cfg(feature = "bench-internals")]
    {
        let parsed = leyline::internals::BrowserProfile::from_toml(profile).unwrap();
        assert!(parsed.tls.padding);
        assert!(!parsed.tls.pre_shared_key);
    }
}

#[test]
fn cookie_rfc6265_edge_cases() {
    let expired = Cookie::parse("https://example.com/", "a=1; Max-Age=0").unwrap();
    assert!(expired.is_expired());
    let neg = Cookie::parse("https://example.com/", "a=1; Max-Age=-5").unwrap();
    assert!(neg.is_expired());

    let jar = Jar::new();
    jar.set("https://example.com/", "a", "1");
    jar.store("https://example.com/", "a=1; Max-Age=0");
    assert_eq!(jar.get("https://example.com/", "a"), None);

    assert!(Cookie::parse("http://127.0.0.1/", "a=1; Domain=evil.com").is_none());

    let host_only = Cookie::parse("https://example.com/", "sid=1").unwrap();
    let jar2 = Jar::new();
    jar2.insert("https://sub.example.com/", host_only);
    assert!(jar2.is_empty());

    let jar3 = Jar::new();
    jar3.store("https://example.com/", "s=1; Secure");
    jar3.store("https://example.com/", "s=2");
    assert_eq!(jar3.get("https://example.com/", "s").as_deref(), Some("2"));

    let jar4 = Jar::new();
    jar4.store("https://example.com/", "p=root; Path=/");
    jar4.store("https://example.com/app", "p=app; Path=/app");
    assert_eq!(
        jar4.get("https://example.com/app/page", "p").as_deref(),
        Some("app")
    );

    let jar5 = Jar::new();
    jar5.store("https://example.com/", "slot=1");
    jar5.store("https://evil.example.com/", "slot=2; Domain=example.com");
    assert_eq!(jar5.len(), 2);
    assert_eq!(
        jar5.get("https://sub.example.com/", "slot").as_deref(),
        Some("2")
    );
}

#[tokio::test]
async fn testing_https_on_ipv6_loopback_verifies_san() {
    let Ok(listener) = TcpListener::bind("[::1]:0") else {
        return;
    };
    let server = Server::https_on(listener, |_| Response::new(200).body("v6")).unwrap();
    let session = Session::builder().tls(server.tls()).build().unwrap();
    let text = session.get(server.url("/")).text().await.unwrap();
    assert_eq!(text, "v6");
}

#[tokio::test]
async fn h1_non_ascii_framing_and_multi_connection_close_and_chunked_excess() {
    async fn serve_raw(payload: &'static [u8]) -> (String, tokio::task::JoinHandle<usize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut accepts = 0usize;
            while let Ok((mut sock, _)) = listener.accept().await {
                accepts += 1;
                let mut buf = [0u8; 1024];
                drop(sock.read(&mut buf).await);
                drop(sock.write_all(payload).await);
                drop(sock.flush().await);
                if accepts == 2 {
                    break;
                }
            }
            accepts
        });
        (format!("http://{addr}/"), handle)
    }

    let session = Session::builder().build().unwrap();

    let (url, h) = serve_raw(b"HTTP/1.1 200 OK\r\nContent-Length: 5\x80\r\n\r\nhello").await;
    let err = session.get(&url).send().await.unwrap_err();
    assert_eq!(err.kind(), Kind::Io);
    assert!(!err.is_retryable());
    h.abort();

    let (url, h) = serve_raw(b"HTTP/1.1 200 OK\r\nContent-Length: \x0b5\r\n\r\nhello").await;
    let err = session.get(&url).send().await.unwrap_err();
    assert_eq!(err.kind(), Kind::Io);
    h.abort();

    let (url, h) = serve_raw(b"HTTP/1.1 200 OK\r\nContent-Length: \xc2\xa05\r\n\r\nhello").await;
    let err = session.get(&url).send().await.unwrap_err();
    assert_eq!(err.kind(), Kind::Io);
    h.abort();

    let (url, h) = serve_raw(
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\x80\r\n\r\nhello",
    )
    .await;
    let err = session.get(&url).send().await.unwrap_err();
    assert_eq!(err.kind(), Kind::Io);
    assert!(!err.is_retryable());
    h.abort();

    let (url, h) = serve_raw(
        b"HTTP/1.1 200 OK\r\nConnection: x-custom\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok",
    )
    .await;
    assert_eq!(session.get(&url).text().await.unwrap(), "ok");
    assert_eq!(session.get(&url).text().await.unwrap(), "ok");
    assert_eq!(h.await.unwrap(), 2);

    let (url, h) = serve_raw(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nok\r\n0\r\n\r\nSMUGGLED",
    )
    .await;
    assert_eq!(session.get(&url).text().await.unwrap(), "ok");
    assert_eq!(session.get(&url).text().await.unwrap(), "ok");
    assert_eq!(h.await.unwrap(), 2);
}

#[cfg(feature = "html")]
#[test]
fn html_canonical_skips_hrefless_link() {
    let doc = r#"<link rel="canonical"><link rel="canonical" href="https://example.com/ok">"#;
    assert_eq!(
        leyline::html::canonical(doc).as_deref(),
        Some("https://example.com/ok")
    );
}

#[cfg(feature = "html")]
#[tokio::test]
async fn crawl_load_honors_post_load_redirect_setter() {
    use leyline::Redirect;

    let server = Server::http(|req| match req.target() {
        "/robots.txt" => Response::new(404),
        "/seed" => Response::new(200)
            .header("content-type", "text/html")
            .body(r#"<a href="/bounce">next</a>"#),
        "/bounce" => Response::new(302).header("location", "/dest"),
        "/dest" => Response::new(200)
            .header("content-type", "text/html")
            .body("<p>arrived</p>"),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let dir = std::env::temp_dir().join(format!("leyline-crawl-load-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let checkpoint = dir.join("crawl.json");

    let session = Session::builder().build().unwrap();
    let mut crawl = session.crawl(server.url("/seed")).respect(None);
    let first = crawl.next().await.unwrap().unwrap();
    assert_eq!(first.url().path(), "/seed");
    crawl.save(&checkpoint).await.unwrap();

    let mut resumed = leyline::Crawl::load(&session, &checkpoint)
        .unwrap()
        .redirect(Redirect::none());
    assert!(
        resumed.next().await.is_none(),
        "post-load Redirect::none() must block following /bounce -> /dest"
    );
    assert_eq!(resumed.skipped(leyline::SkipReason::Redirects), 1);
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn request_from_does_not_pin_inferred_navigate_preset() {
    let server = Server::http(|_| Response::new(200).body("ok"))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .build()
        .unwrap();
    let mut req = session.get(server.url("/api")).build().unwrap();
    *req.method_mut() = http::Method::POST;
    req.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    *req.body_mut() = leyline::Body::from(r#"{"a":1}"#);
    let resp = session.request_from(req).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let seen = server.requests().await;
    assert_eq!(
        seen[0].header("sec-fetch-mode"),
        Some("cors"),
        "mutated POST JSON request must use Chrome Xhr sec-fetch-mode"
    );
    assert_eq!(seen[0].header("sec-fetch-dest"), Some("empty"));
    assert_eq!(
        seen[0].header("accept"),
        Some("application/json, text/plain, */*")
    );
}

#[cfg(feature = "websocket")]
#[tokio::test]
async fn websocket_protocol_enforces_http2_and_rejects_http3() {
    let server = Server::https(|_| Response::new(200).body("h1-only"))
        .await
        .unwrap();
    let session = Session::builder().tls(server.tls()).build().unwrap();
    let ws_url = server.url("/ws").replacen("https://", "wss://", 1);
    let h2_err = session
        .get(&ws_url)
        .protocol(leyline::Protocol::Http2)
        .websocket()
        .await
        .unwrap_err();
    assert_eq!(
        h2_err.kind(),
        Kind::Http2,
        "protocol(Http2) must not silently fall back to HTTP/1.1 upgrade: {h2_err:?}"
    );
    #[cfg(feature = "http3")]
    {
        let h3_err = session
            .get("wss://127.0.0.1:443/ws")
            .protocol(leyline::Protocol::Http3)
            .websocket()
            .await
            .unwrap_err();
        assert_eq!(h3_err.kind(), Kind::Config);
    }
}

#[tokio::test]
async fn proxy_list_no_proxy_and_custom_proxy_across_redirects() {
    let dest = Server::http(|_| Response::new(200).body("direct-dest"))
        .await
        .unwrap();
    let dest_url = dest.url("/final");
    let hop1 = Server::http(move |_| Response::new(302).header("location", dest_url.clone()))
        .await
        .unwrap();

    let list_session = Session::builder()
        .proxy(leyline::Proxy::list(["http://127.0.0.1:1"]).no_proxy(["127.0.0.1"]))
        .build()
        .unwrap();
    assert_eq!(
        list_session.get(dest.url("/bypass")).text().await.unwrap(),
        "direct-dest"
    );

    let seen_hops: std::sync::Arc<std::sync::Mutex<Vec<String>>> = std::sync::Arc::default();
    let seen_slot = std::sync::Arc::clone(&seen_hops);
    let dest_port = dest.addr().port();
    let custom_session = Session::builder()
        .proxy(leyline::Proxy::custom(move |url| {
            seen_slot.lock().unwrap().push(url.path().to_owned());
            if url.port() == Some(dest_port) {
                Some(leyline::Proxy::from("http://127.0.0.1:1"))
            } else {
                None
            }
        }))
        .build()
        .unwrap();
    let hop2_err = custom_session
        .get(hop1.url("/start"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(hop2_err.kind(), Kind::Proxy);
    assert_eq!(
        seen_hops.lock().unwrap().as_slice(),
        &["/start".to_owned(), "/final".to_owned()],
        "Proxy::custom must be re-evaluated on redirect hops"
    );

    let bad_req_proxy = Session::builder()
        .build()
        .unwrap()
        .get(dest.url("/"))
        .proxy(leyline::Proxy::from("http://127.0.0.1:1").header("host", "evil.test"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(bad_req_proxy.kind(), Kind::Config);
}

#[tokio::test]
async fn before_send_empty_body_strips_content_length_and_shutdown_short_circuits() {
    let server = Server::http(|_| Response::new(200).body("ok"))
        .await
        .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_hook = std::sync::Arc::clone(&calls);
    let session = Session::builder()
        .before_send(move |req| {
            calls_hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *req.body_mut() = leyline::Body::default();
            Ok(())
        })
        .build()
        .unwrap();
    let resp = session
        .post(server.url("/cleared"))
        .header("content-length", "50")
        .body("hello")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.text().await.unwrap(), "ok");
    let seen = server.requests().await;
    assert_eq!(
        seen[0].header("content-length"),
        Some("0"),
        "stale content-length: 50 must be replaced with 0 when POST body is cleared"
    );

    session.shutdown().await;
    let before = calls.load(std::sync::atomic::Ordering::SeqCst);
    let err = session.get(server.url("/after")).send().await.unwrap_err();
    assert!(err.is_shut_down(), "expected shut_down error, got {err:?}");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        before,
        "before_send must not run after shutdown"
    );
}

#[test]
fn tcp_profile_window_bounds_and_empty_interface_rejected_at_build() {
    let err = "window_size = 70000"
        .parse::<leyline::TcpProfile>()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    let ok = "window_size = 65535"
        .parse::<leyline::TcpProfile>()
        .unwrap();
    assert_eq!(ok.window_size(), 65535);
    let err = Session::builder().interface("").build().unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    let err = Session::builder()
        .interface("eth\x000")
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    let err = leyline::Body::file(std::env::temp_dir()).unwrap_err();
    assert_eq!(err.kind(), Kind::Io);
}

#[tokio::test]
async fn proxy_pool_no_proxy_stats_custom_headers_and_websocket_pool() {
    let blocked = Server::http(|_| {
        Response::new(403)
            .header("cf-mitigated", "challenge")
            .body("challenged")
    })
    .await
    .unwrap();
    let pool_session = Session::builder()
        .proxy(
            leyline::Proxy::list(["http://127.0.0.1:1"])
                .no_proxy(["127.0.0.1"])
                .ban_after(1),
        )
        .build()
        .unwrap();
    let resp = pool_session.get(blocked.url("/direct")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 403);
    assert!(resp.block().is_some());
    let stats = pool_session.stats();
    assert_eq!(stats.proxy_failures("http://127.0.0.1:1"), 0);
    assert_eq!(stats.proxy_in_use("http://127.0.0.1:1"), 0);
    assert!(stats.proxy_banned_until("http://127.0.0.1:1").is_none());

    let fake_proxy = Server::http(|_| Response::new(200).body("via-custom-proxy"))
        .await
        .unwrap();
    let proxy_url = format!("http://{}", fake_proxy.addr());
    let custom_hdr_session = Session::builder()
        .proxy(
            leyline::Proxy::custom(move |_| Some(leyline::Proxy::from(proxy_url.as_str())))
                .header("x-custom-proxy-token", "secret-42"),
        )
        .build()
        .unwrap();
    let body = custom_hdr_session
        .get("http://origin.invalid/check")
        .text()
        .await
        .unwrap();
    assert_eq!(body, "via-custom-proxy");
    let seen = fake_proxy.requests().await;
    assert_eq!(seen[0].header("x-custom-proxy-token"), Some("secret-42"));

    #[cfg(feature = "websocket")]
    {
        let ws_origin = Server::https(|_| Response::new(200).body("direct-origin"))
            .await
            .unwrap();
        let ws_pool_session = Session::builder()
            .tls(ws_origin.tls())
            .proxy(leyline::Proxy::list(["http://127.0.0.1:1", "http://127.0.0.1:2"]).ban_after(1))
            .build()
            .unwrap();
        let ws_url = ws_origin.url("/ws").replacen("https://", "wss://", 1);
        let ws_err = ws_pool_session.get(&ws_url).websocket().await.unwrap_err();
        assert_eq!(
            ws_err.kind(),
            Kind::Proxy,
            "WebSocket on Proxy::list session must dial the proxy, not direct: {ws_err:?}"
        );
        let ws_stats = ws_pool_session.stats();
        assert!(
            ws_stats.proxy_banned_until("http://127.0.0.1:1").is_some(),
            "dead proxy used for WebSocket handshake must be struck and banned"
        );
        let ws_err2 = ws_pool_session.get(&ws_url).websocket().await.unwrap_err();
        assert_eq!(ws_err2.kind(), Kind::Proxy);
        assert!(
            ws_pool_session
                .stats()
                .proxy_banned_until("http://127.0.0.1:2")
                .is_some(),
            "second WebSocket handshake must rotate past banned proxy to the next entry"
        );
        assert!(
            ws_origin.requests().await.is_empty(),
            "WebSocket on Proxy::list session must never connect direct to origin"
        );
    }
}

#[cfg(feature = "html")]
#[tokio::test]
async fn crawl_load_invalid_host_limits_sends_zero_requests() {
    let server = Server::http(|req| match req.target() {
        "/robots.txt" => Response::new(404),
        "/seed" => Response::new(200)
            .header("content-type", "text/html")
            .body(r#"<a href="/next">next</a>"#),
        _ => Response::new(200)
            .header("content-type", "text/html")
            .body("<p>next</p>"),
    })
    .await
    .unwrap();

    let dir = std::env::temp_dir().join(format!("leyline-crawl-badlim-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let checkpoint = dir.join("crawl.json");

    let session = Session::builder().build().unwrap();
    let mut crawl = session.crawl(server.url("/seed")).respect(None);
    let _ = crawl.next().await.unwrap().unwrap();
    crawl.save(&checkpoint).await.unwrap();
    let before_count = server.requests().await.len();

    let mut resumed = leyline::Crawl::load(&session, &checkpoint)
        .unwrap()
        .host_limits(leyline::HostLimits::new().pause_on([9999]));
    let err = resumed.next().await.unwrap().unwrap_err();
    assert_eq!(err.kind(), Kind::Config);
    assert!(resumed.next().await.is_none());
    assert_eq!(
        server.requests().await.len(),
        before_count,
        "invalid post-load host_limits must not send any frontier requests"
    );
    drop(std::fs::remove_dir_all(&dir));
}

#[tokio::test]
async fn before_send_method_change_updates_inferred_preset() {
    let server = Server::http(|_| Response::new(200).body("ok"))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .before_send(|req| {
            *req.method_mut() = http::Method::POST;
            req.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/json"),
            );
            *req.body_mut() = leyline::Body::from(r#"{"x":1}"#);
            Ok(())
        })
        .build()
        .unwrap();
    let resp = session.get(server.url("/mutated")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let seen = server.requests().await;
    assert_eq!(
        seen[0].header("sec-fetch-mode"),
        Some("cors"),
        "before_send changing GET to POST JSON must refresh inferred Chrome Xhr preset"
    );
    assert_eq!(seen[0].header("sec-fetch-dest"), Some("empty"));
    assert_eq!(
        seen[0].header("accept"),
        Some("application/json, text/plain, */*")
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn doh_config_error_is_non_retryable_config() {
    let doh_server = Server::https(|_| Response::new(200)).await.unwrap();
    let session = Session::builder()
        .retry(2)
        .doh(doh_server.url("/dns-query"))
        .tls(doh_server.tls())
        .interface("leyline-no-such-if99")
        .build()
        .unwrap();
    let err = session
        .get("http://bad-iface.doh.test/")
        .send()
        .await
        .unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::Config,
        "DoH bad-interface error must surface as Kind::Config, got {err:?}"
    );
    assert!(!err.is_retryable());
    assert_eq!(err.attempts(), 1);
}
