use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::testing::{Response, Server};
use leyline::{Body, Browser, Kind, Protocol, Proxy, Redirect, Session, Timeout, Tls};

use super::doh_and_ech::{decode_qname_and_qtype, dns_response};

#[cfg(feature = "compression-gzip")]
pub(super) fn gzip_bytes(data: &[u8]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    let mut enc = GzEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

pub(super) struct TempDir(PathBuf);

impl TempDir {
    pub(super) fn new(prefix: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("leyline-{prefix}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.0));
    }
}

#[tokio::test]
async fn realistic_api_client_pipeline_under_retries_redirects_cache_and_interop() {
    let dir = TempDir::new("v030-api-pipeline");
    let upload_path = dir.path("payload.txt");
    std::fs::write(&upload_path, b"file-upload-body-12345").unwrap();

    let origin_hits = Arc::new(AtomicUsize::new(0));
    let origin_hits_bg = Arc::clone(&origin_hits);
    let origin = Server::http(move |req| match req.target() {
        "/items" => {
            let n = origin_hits_bg.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Response::new(503).body("transient")
            } else {
                Response::new(302).header("location", "/v2/items")
            }
        }
        "/v2/items" => {
            let cookie = req.header("cookie").unwrap_or("none");
            let sig = req.header("x-sig").unwrap_or("missing");
            Response::new(200)
                .header("set-cookie", "sid=abc123; Path=/")
                .header("cache-control", "max-age=60")
                .body(format!("ok:sig={sig}:cookie={cookie}"))
        }
        "/gzipped" => {
            #[cfg(feature = "compression-gzip")]
            {
                Response::new(200)
                    .header("content-encoding", "gzip")
                    .header("content-type", "text/plain")
                    .body(gzip_bytes(b"decoded-stream-payload"))
            }
            #[cfg(not(feature = "compression-gzip"))]
            Response::new(200).body("decoded-stream-payload")
        }
        "/static" => Response::new(200)
            .header("cache-control", "max-age=60")
            .body("cached-static-body"),
        "/upload" => Response::new(200).body(format!("len={}:{}", req.body().len(), req.text())),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let origin_port = origin.addr().port();
    let proxy_server = Server::http(move |req| {
        let edge = req.header("x-proxy-edge").unwrap_or("missing");
        let sig = req.header("x-sig").unwrap_or("missing");
        Response::new(200)
            .header("set-cookie", "sid=via-proxy; Path=/")
            .header("cache-control", "max-age=60")
            .body(format!("proxied:edge={edge}:sig={sig}"))
    })
    .await
    .unwrap();
    let proxy_url = format!("http://{}", proxy_server.addr());

    let sign_calls = Arc::new(AtomicUsize::new(0));
    let sign_calls_hook = Arc::clone(&sign_calls);
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .base_url(origin.url("/"))
        .retry(2)
        .redirect(Redirect::limit(5))
        .cache(1 << 20)
        .proxy(
            Proxy::custom(move |url| {
                if url.path() == "/v2/items" {
                    Some(Proxy::from(proxy_url.as_str()))
                } else {
                    None
                }
            })
            .header("x-proxy-edge", "us-east"),
        )
        .before_send(move |req| {
            let n = sign_calls_hook.fetch_add(1, Ordering::SeqCst);
            let len = req.body().content_length().unwrap_or(0);
            let val = if req.uri().path() == "/items" {
                format!("{}-{n}-{len}", req.method())
            } else {
                format!("{}-{len}", req.method())
            };
            req.headers_mut()
                .insert("x-sig", http::HeaderValue::from_str(&val).unwrap());
            Ok(())
        })
        .build()
        .unwrap();

    let first = session.get("/items").send().await.unwrap();
    assert!(!first.from_cache());
    let first_body = first.text().await.unwrap();
    assert!(
        first_body.starts_with("proxied:edge=us-east:sig=GET-1-0"),
        "expected retry + redirect through Proxy::custom with outer header, got {first_body}"
    );
    assert_eq!(sign_calls.load(Ordering::SeqCst), 2);
    assert_eq!(origin_hits.load(Ordering::SeqCst), 2);

    let miss_static = session.get("/static").send().await.unwrap();
    assert!(!miss_static.from_cache());
    assert_eq!(miss_static.text().await.unwrap(), "cached-static-body");
    let cached = session
        .get(format!("http://127.0.0.1:{origin_port}/static"))
        .send()
        .await
        .unwrap();
    assert!(cached.from_cache());
    assert_eq!(cached.text().await.unwrap(), "cached-static-body");

    let built = session
        .get("/gzipped")
        .protocol(Protocol::Http1)
        .timeout(Timeout::new().total(Duration::from_secs(5)))
        .max_body_size(1024 * 64)
        .error_for_status()
        .stream();
    let cloned = built.try_clone().expect("empty GET request must clone");
    let raw = built.build().unwrap();
    let resp = session.request_from(raw).send().await.unwrap();
    let http_resp = resp.into_http().unwrap();
    assert_eq!(http_resp.status(), http::StatusCode::OK);
    assert_eq!(http_resp.version(), http::Version::HTTP_11);
    assert_eq!(
        http_resp.extensions().get::<url::Url>().map(url::Url::path),
        Some("/gzipped")
    );
    #[cfg(feature = "compression-gzip")]
    {
        assert!(http_resp.headers().get("content-encoding").is_none());
        assert!(http_resp.headers().get("content-length").is_none());
    }
    let mut body = http_resp.into_body();
    let mut collected = Vec::new();
    while let Some(chunk) = body.next().await {
        collected.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(collected, b"decoded-stream-payload");
    assert_eq!(cloned.text().await.unwrap(), "decoded-stream-payload");

    let file_body = Body::file(&upload_path).unwrap();
    assert_eq!(file_body.content_length(), Some(22));
    let up = session
        .post("/upload")
        .body(file_body)
        .text()
        .await
        .unwrap();
    assert_eq!(up, "len=22:file-upload-body-12345");
}

#[test]
fn realistic_per_core_doh_shared_cache_and_isolated_shutdown() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (origin_port, doh_url, doh_tls, _guards) = rt.block_on(async {
        let origin = Server::http(|req| match req.target() {
            "/shared" => Response::new(200)
                .header("set-cookie", "core=shared; Path=/")
                .body("path=/shared"),
            "/cached" => Response::new(200)
                .header("cache-control", "max-age=60")
                .body("path=/cached"),
            _ => Response::new(404),
        })
        .await
        .unwrap();
        let origin_port = origin.addr().port();
        let doh = Server::https(move |req| {
            let body = req.body();
            let Some((name, qtype)) = decode_qname_and_qtype(body) else {
                return Response::new(400);
            };
            let wire = if name == "percore.doh.test" && qtype == 1 {
                dns_response(body, 0, &[(1, 120, &[127, 0, 0, 1])])
            } else {
                dns_response(body, 0, &[])
            };
            Response::new(200)
                .header("content-type", "application/dns-message")
                .body(wire)
        })
        .await
        .unwrap();
        let doh_url = doh.url("/dns-query");
        let doh_tls = doh.tls();
        (origin_port, doh_url, doh_tls, (origin, doh))
    });

    let core_index = Arc::new(AtomicUsize::new(0));
    let core0_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seed_url = format!("http://percore.doh.test:{origin_port}/shared");
    let cached_url = format!("http://percore.doh.test:{origin_port}/cached");
    let results = Session::builder()
        .doh(doh_url)
        .tls(doh_tls)
        .cache(1 << 20)
        .timeout(Duration::from_secs(5))
        .run_per_core(|session| {
            let idx = core_index.fetch_add(1, Ordering::SeqCst);
            let core0_done = Arc::clone(&core0_done);
            let seed_url = seed_url.clone();
            let cached_url = cached_url.clone();
            async move {
                if idx == 0 {
                    struct SignalOnDrop(Arc<std::sync::atomic::AtomicBool>);
                    impl Drop for SignalOnDrop {
                        fn drop(&mut self) {
                            self.0.store(true, Ordering::Release);
                        }
                    }
                    let _signal = SignalOnDrop(Arc::clone(&core0_done));
                    assert_eq!(
                        session.get(&seed_url).text().await.unwrap(),
                        "path=/shared"
                    );
                    let warm = session.get(&cached_url).send().await.unwrap();
                    assert!(!warm.from_cache());
                    assert_eq!(warm.text().await.unwrap(), "path=/cached");
                    session.shutdown().await;
                    let after = session.get(&cached_url).send().await.unwrap_err();
                    assert!(after.is_shut_down());
                } else {
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                    while !core0_done.load(Ordering::Acquire) {
                        assert!(
                            tokio::time::Instant::now() < deadline,
                            "core 0 did not finish before timeout"
                        );
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    let second = session.get(&cached_url).send().await.unwrap();
                    assert!(
                        second.from_cache(),
                        "shared cache must serve core 0's cached entry across cores after sibling shutdown"
                    );
                    assert_eq!(second.text().await.unwrap(), "path=/cached");
                }
                session.cookies().len()
            }
        })
        .unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|&cookies| cookies >= 1));
    drop(_guards);
    drop(rt);
}

#[tokio::test]
async fn doh_ttl_expiry_http_error_size_cap_and_pkcs12_negative_matrix() {
    let origin = Server::http(|_| {
        Response::new(200)
            .header("connection", "close")
            .body("ttl-ok")
    })
    .await
    .unwrap();
    let origin_port = origin.addr().port();

    let lookups = Arc::new(AtomicUsize::new(0));
    let lookups_bg = Arc::clone(&lookups);
    let mode = Arc::new(AtomicUsize::new(0));
    let mode_bg = Arc::clone(&mode);
    let doh = Server::https(move |req| {
        let body = req.body();
        let Some((name, qtype)) = decode_qname_and_qtype(body) else {
            return Response::new(400);
        };
        if qtype == 1 {
            lookups_bg.fetch_add(1, Ordering::SeqCst);
        }
        match mode_bg.load(Ordering::SeqCst) {
            1 => Response::new(500).body("doh down"),
            2 => Response::new(200)
                .header("content-type", "application/dns-message")
                .body(vec![0u8; 70_000]),
            _ => {
                let wire = if name == "ttl.doh.test" && qtype == 1 {
                    dns_response(body, 0, &[(1, 1, &[127, 0, 0, 1])])
                } else {
                    dns_response(body, 0, &[])
                };
                Response::new(200)
                    .header("content-type", "application/dns-message")
                    .body(wire)
            }
        }
    })
    .await
    .unwrap();

    let session = Session::builder()
        .retry(0)
        .doh(doh.url("/dns-query"))
        .tls(doh.tls())
        .build()
        .unwrap();
    let url = format!("http://ttl.doh.test:{origin_port}/");
    assert_eq!(session.get(&url).text().await.unwrap(), "ttl-ok");
    let after_first = lookups.load(Ordering::SeqCst);
    assert_eq!(session.get(&url).text().await.unwrap(), "ttl-ok");
    assert_eq!(lookups.load(Ordering::SeqCst), after_first);

    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(session.get(&url).text().await.unwrap(), "ttl-ok");
    assert!(
        lookups.load(Ordering::SeqCst) > after_first,
        "expired 1s DoH TTL must trigger a fresh query"
    );

    mode.store(1, Ordering::SeqCst);
    let err_500 = session
        .get(format!("http://err500.doh.test:{origin_port}/"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(err_500.kind(), Kind::Dns);

    mode.store(2, Ordering::SeqCst);
    let err_big = session
        .get(format!("http://toobig.doh.test:{origin_port}/"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(err_big.kind(), Kind::Dns);

    for bad_der in [Vec::new(), b"not-a-pkcs12-der".to_vec()] {
        let err = Session::builder()
            .tls(Tls::new().client_cert_pkcs12(bad_der, "pw"))
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), Kind::Config);
    }

    if let Ok(v6_listener) = tokio::net::TcpListener::bind("[::1]:0").await {
        let v6_port = v6_listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            if let Ok((mut sock, _)) = v6_listener.accept().await {
                let mut req = [0u8; 512];
                drop(sock.read(&mut req).await);
                drop(
                    sock.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nv6-ok",
                    )
                    .await,
                );
            }
        });
        let before_v6 = lookups.load(Ordering::SeqCst);
        let body = session
            .get(format!("http://[::1]:{v6_port}/"))
            .text()
            .await
            .unwrap();
        assert_eq!(body, "v6-ok");
        assert_eq!(
            lookups.load(Ordering::SeqCst),
            before_v6,
            "bracketed IPv6 literal must not trigger DoH queries"
        );
    }
}

#[tokio::test]
async fn frozen_v020_session_and_crawl_json_round_trip_and_cookies_none_persistence() {
    let dir = TempDir::new("v030-frozen-compat");
    let server = Server::http(|req| match req.target() {
        "/echo-cookie" => Response::new(200)
            .header("set-cookie", "newcookie=1; Path=/")
            .body(req.header("cookie").unwrap_or("none").to_owned()),
        "/robots.txt" => Response::new(404),
        "/page1" => Response::new(200)
            .header("content-type", "text/html")
            .body("<p>hello from frozen crawl</p>"),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let v020_session = dir.path("v020-session.json");
    let v020_json = serde_json::json!({
        "version": 1,
        "platform": "linux",
        "jar": [{
            "name": "legacy",
            "value": "from020",
            "domain": "127.0.0.1",
            "path": "/",
            "secure": false,
            "http_only": false,
            "same_site": "Lax",
            "expires": null,
            "creation_time": 1_700_000_000,
            "host_only": true
        }],
        "state": {}
    });
    std::fs::write(&v020_session, serde_json::to_vec(&v020_json).unwrap()).unwrap();
    let loaded = Session::load(&v020_session).unwrap().build().unwrap();
    let echo = loaded.get(server.url("/echo-cookie")).text().await.unwrap();
    assert!(
        echo.contains("legacy=from020"),
        "frozen 0.2.0 session.json must load and send cookies: {echo}"
    );

    let no_cookie_path = dir.path("no-cookies.json");
    let no_cookies = Session::builder().cookies(None).build().unwrap();
    no_cookies.save(&no_cookie_path).unwrap();
    let reloaded_no_cookies = Session::load(&no_cookie_path).unwrap().build().unwrap();
    let _ = reloaded_no_cookies
        .get(server.url("/echo-cookie"))
        .text()
        .await
        .unwrap();
    let second = reloaded_no_cookies
        .get(server.url("/echo-cookie"))
        .text()
        .await
        .unwrap();
    assert_eq!(
        second, "none",
        "cookies(None) must persist across save/load"
    );

    #[cfg(feature = "html")]
    {
        let page1_url = server.url("/page1");
        let v020_crawl = dir.path("v020-crawl.json");
        let origin = serde_json::json!({
            "scheme": "http",
            "host": "127.0.0.1",
            "port": server.addr().port()
        });
        let v020_crawl_json = serde_json::json!({
            "version": 1,
            "frontier": {
                "queues": [[
                    origin,
                    [{
                        "url": page1_url,
                        "depth": 0,
                        "hops": 0,
                        "seed": true,
                        "widen": true,
                        "attempts": 0,
                        "chain": []
                    }]
                ]],
                "order": [origin],
                "seen": [page1_url],
                "redirected": [],
                "bounds": [{ "Host": "127.0.0.1" }],
                "scope": "Host"
            },
            "respect": null,
            "agent": "*",
            "max_body": null,
            "max_pages": null,
            "pace": {
                "rule": { "max_in_flight": 1, "interval": { "secs": 1, "nanos": 0 } },
                "overrides": [],
                "max_total": 16,
                "pause": {
                    "statuses": [429, 503],
                    "invalid": null,
                    "default": { "secs": 60, "nanos": 0 },
                    "max": { "secs": 86400, "nanos": 0 }
                }
            },
            "trusted": page1_url,
            "ready": [],
            "kept": [],
            "texts": [],
            "held": {},
            "waiting_on": {},
            "settled": [],
            "stats": { "requests": 0, "pages": 0, "errors": 0, "skipped": {} },
            "delayed": [],
            "sitemaps": [],
            "sitemap_files": [],
            "pauses": []
        });
        std::fs::write(&v020_crawl, serde_json::to_vec(&v020_crawl_json).unwrap()).unwrap();
        let resave_path = dir.path("v030-resaved-crawl.json");
        let mut crawl = leyline::Crawl::load(&loaded, &v020_crawl)
            .unwrap()
            .redirect(Redirect::none())
            .max_body_size(1024 * 64);
        crawl.save(&resave_path).await.unwrap();
        let mut reloaded_crawl = leyline::Crawl::load(&loaded, &resave_path)
            .unwrap()
            .filter(|url| url.path() == "/page1");
        let page = reloaded_crawl.next().await.unwrap().unwrap();
        assert_eq!(page.url().path(), "/page1");
        assert!(page.text().contains("hello from frozen crawl"));
        assert!(reloaded_crawl.next().await.is_none());
    }
}

#[tokio::test]
async fn pkcs12_intermediate_ca_chain_verified_over_tcp_and_h3() {
    use leyline_bssl::ssl::{Ssl, SslContextBuilder, SslMethod, SslVerifyMode};
    use leyline_bssl::stack::Stack;
    use tokio::net::TcpListener;

    let (server_cert, server_key) = super::tls_support::self_signed_dns("chain-mtls.local");
    let server_ca_pem = server_cert.to_pem().unwrap();

    let (client_root, client_inter, client_leaf, client_key) =
        super::tls_support::ca_signed_chain("client-leaf.local", false);
    let mut chain = Stack::new().unwrap();
    chain.push(client_inter).unwrap();
    let p12_with_chain = leyline_bssl::pkcs12::create(
        "chain-pass",
        "client",
        &client_key,
        &client_leaf,
        Some(&chain),
    )
    .unwrap();
    let p12_leaf_only =
        leyline_bssl::pkcs12::create("chain-pass", "client", &client_key, &client_leaf, None)
            .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp_addr = listener.local_addr().unwrap();
    let mut ctx = SslContextBuilder::new(SslMethod::tls()).unwrap();
    ctx.set_certificate(&server_cert).unwrap();
    ctx.set_private_key(&server_key).unwrap();
    ctx.cert_store_mut().add_cert(client_root.clone()).unwrap();
    ctx.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
    ctx.set_alpn_select_callback(|_, offered| {
        leyline_bssl::ssl::select_next_proto(b"\x02h2\x08http/1.1", offered)
            .ok_or(leyline_bssl::ssl::AlpnError::NOACK)
    });
    let ctx = Arc::new(ctx.build());
    let tcp_ctx = Arc::clone(&ctx);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let ctx = Arc::clone(&tcp_ctx);
            tokio::spawn(async move {
                let ssl = Ssl::new(&ctx).unwrap();
                if let Ok(stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, tcp)
                    .accept()
                    .await
                {
                    super::tls_support::serve_h1_ok(stream, "chain-mtls-ok").await;
                }
            });
        }
    });

    let tcp_url = format!("https://chain-mtls.local:{}/ok", tcp_addr.port());
    let good_tcp = Session::builder()
        .protocol(Protocol::Http1)
        .resolve("chain-mtls.local", [tcp_addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(server_ca_pem.clone())
                .client_cert_pkcs12(p12_with_chain.clone(), "chain-pass"),
        )
        .build()
        .unwrap();
    assert_eq!(
        good_tcp.get(&tcp_url).text().await.unwrap(),
        "chain-mtls-ok"
    );

    let missing_inter_tcp = Session::builder()
        .protocol(Protocol::Http1)
        .resolve("chain-mtls.local", [tcp_addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(server_ca_pem.clone())
                .client_cert_pkcs12(p12_leaf_only.clone(), "chain-pass"),
        )
        .build()
        .unwrap();
    let err = missing_inter_tcp.get(&tcp_url).send().await.unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::Tls,
        "omitted intermediate CA must fail TLS verification on HTTP/1.1: {err:?}"
    );
    assert!(!err.is_retryable());

    let missing_inter_h2 = Session::builder()
        .protocol(Protocol::Http2)
        .resolve("chain-mtls.local", [tcp_addr])
        .tls(
            Tls::new()
                .system_roots(false)
                .env_roots(false)
                .ca_pem(server_ca_pem.clone())
                .client_cert_pkcs12(p12_leaf_only.clone(), "chain-pass"),
        )
        .build()
        .unwrap();
    let err_h2 = missing_inter_h2.get(&tcp_url).send().await.unwrap_err();
    assert_eq!(
        err_h2.kind(),
        Kind::Tls,
        "omitted intermediate CA must fail TLS verification on HTTP/2: {err_h2:?}"
    );
    assert!(!err_h2.is_retryable());

    #[cfg(unix)]
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let dir = TempDir::new("sslstream-tls13-alert");
        let sock_path = dir.path("mtls.sock");
        let unix_listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
        let unix_ctx = Arc::clone(&ctx);
        tokio::spawn(async move {
            while let Ok((unix, _)) = unix_listener.accept().await {
                let unix_ctx = Arc::clone(&unix_ctx);
                tokio::spawn(async move {
                    let ssl = Ssl::new(&unix_ctx).unwrap();
                    if let Ok(mut stream) = leyline_bssl_tokio::SslStreamBuilder::new(ssl, unix)
                        .accept()
                        .await
                    {
                        let mut buf = [0u8; 512];
                        if stream.read(&mut buf).await.is_ok() {
                            drop(
                                stream
                                    .write_all(
                                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                                    )
                                    .await,
                            );
                        }
                    }
                });
            }
        });

        for proto in [Protocol::Http1, Protocol::Http2] {
            let unix_missing = Session::builder()
                .protocol(proto)
                .unix_socket(&sock_path)
                .retry(1)
                .tls(
                    Tls::new()
                        .system_roots(false)
                        .env_roots(false)
                        .ca_pem(server_ca_pem.clone())
                        .client_cert_pkcs12(p12_leaf_only.clone(), "chain-pass"),
                )
                .build()
                .unwrap();
            let err = unix_missing
                .get("https://chain-mtls.local/unix")
                .send()
                .await
                .unwrap_err();
            assert_eq!(
                err.kind(),
                Kind::Tls,
                "SslStream post-handshake TLS 1.3 alert must map to Kind::Tls ({proto:?}): {err:?}"
            );
            assert!(!err.is_retryable());
            assert_eq!(err.attempts(), 1);
        }
    }

    #[cfg(feature = "http3")]
    {
        use super::h3_support::{Limits, Reply, h3_server_with_tls};
        let h3_root = client_root.clone();
        let h3 = h3_server_with_tls(
            server_cert,
            server_key,
            move |tls| {
                tls.cert_store_mut().add_cert(h3_root).unwrap();
                tls.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
            },
            vec![Reply::Body(12)],
            Limits::default(),
        )
        .await;
        let h3_addr = std::net::SocketAddr::from(([127, 0, 0, 1], h3.port));
        let h3_url = format!("https://chain-mtls.local:{}/ok", h3.port);

        let good_h3 = Session::builder()
            .browser(Browser::Chrome154)
            .protocol(Protocol::Http3)
            .resolve("chain-mtls.local", [h3_addr])
            .tls(
                Tls::new()
                    .system_roots(false)
                    .env_roots(false)
                    .ca_pem(server_ca_pem.clone())
                    .client_cert_pkcs12(p12_with_chain, "chain-pass"),
            )
            .build()
            .unwrap();
        assert_eq!(good_h3.get(&h3_url).bytes().await.unwrap().len(), 12);

        let missing_inter_h3 = Session::builder()
            .browser(Browser::Chrome154)
            .protocol(Protocol::Http3)
            .resolve("chain-mtls.local", [h3_addr])
            .tls(
                Tls::new()
                    .system_roots(false)
                    .env_roots(false)
                    .ca_pem(server_ca_pem)
                    .client_cert_pkcs12(p12_leaf_only, "chain-pass"),
            )
            .build()
            .unwrap();
        let err = missing_inter_h3.get(&h3_url).send().await.unwrap_err();
        assert_eq!(
            err.kind(),
            Kind::Tls,
            "omitted intermediate CA must fail TLS verification on HTTP/3: {err:?}"
        );
        assert!(!err.is_retryable());
    }
}
