use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::testing::{Response, Server};
use leyline::{Browser, Proxy, Redirect, Retry, Session};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::doh_and_ech::{decode_qname_and_qtype, dns_response};
use super::v030_workloads::TempDir;

async fn spawn_connect_proxy() -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connect_hits = Arc::new(AtomicUsize::new(0));
    let forward_hits = Arc::new(AtomicUsize::new(0));
    let connect_bg = Arc::clone(&connect_hits);
    let forward_bg = Arc::clone(&forward_hits);

    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            let connect_bg = Arc::clone(&connect_bg);
            let forward_bg = Arc::clone(&forward_bg);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                let head_end = loop {
                    let Ok(n) = client.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]);
                let first_line = head.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("");
                let target = parts.next().unwrap_or("");

                if method == "CONNECT" {
                    if head.to_ascii_lowercase().contains("x-edge: cf-pop") {
                        connect_bg.fetch_add(1, Ordering::SeqCst);
                    }
                    let Ok(mut upstream) = TcpStream::connect(target).await else {
                        drop(client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await);
                        return;
                    };
                    if client
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                    if buf.len() > head_end {
                        drop(upstream.write_all(&buf[head_end..]).await);
                    }
                    drop(tokio::io::copy_bidirectional(&mut client, &mut upstream).await);
                } else if let Ok(url) = url::Url::parse(target) {
                    if head.to_ascii_lowercase().contains("x-edge: cf-pop") {
                        forward_bg.fetch_add(1, Ordering::SeqCst);
                    }
                    let host = url.host_str().unwrap_or("127.0.0.1");
                    let port = url.port_or_known_default().unwrap_or(80);
                    let Ok(mut upstream) = TcpStream::connect((host, port)).await else {
                        drop(client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await);
                        return;
                    };
                    let mut path = url.path().to_owned();
                    if let Some(q) = url.query() {
                        path.push('?');
                        path.push_str(q);
                    }
                    let mut out = format!("{method} {path} HTTP/1.1\r\n");
                    for line in head.lines().skip(1) {
                        if line.is_empty() {
                            break;
                        }
                        let lower = line.to_ascii_lowercase();
                        if !lower.starts_with("proxy-connection:")
                            && !lower.starts_with("connection:")
                        {
                            out.push_str(line);
                            out.push_str("\r\n");
                        }
                    }
                    out.push_str("Connection: close\r\n\r\n");
                    if upstream.write_all(out.as_bytes()).await.is_err() {
                        return;
                    }
                    if buf.len() > head_end {
                        drop(upstream.write_all(&buf[head_end..]).await);
                    }
                    drop(upstream.shutdown().await);
                    drop(tokio::io::copy(&mut upstream, &mut client).await);
                    drop(client.shutdown().await);
                }
            });
        }
    });

    (format!("http://{addr}"), connect_hits, forward_hits)
}

#[tokio::test]
async fn combo_proxy_list_with_before_send_cache_download_tab_crawl_and_h2_connect() {
    let dir = TempDir::new("combo-proxy-list");
    let artifact = b"proxy-pool-download-payload-0123456789".to_vec();
    let artifact_sha: [u8; 32] = Sha256::digest(&artifact).into();
    let artifact_bg = Arc::new(artifact.clone());

    let tab_retry_hits = Arc::new(AtomicUsize::new(0));
    let tab_retry_bg = Arc::clone(&tab_retry_hits);

    let origin = Server::http(move |req| match req.target() {
        "/cached" => Response::new(200)
            .header("cache-control", "max-age=60")
            .body(format!(
                "cached:sig={}",
                req.header("x-sig").unwrap_or("none")
            )),
        "/dl" => Response::new(200)
            .header("etag", "\"dl-1\"")
            .body(artifact_bg.as_ref().clone()),
        "/tab/start" => Response::new(200)
            .header("content-type", "text/html")
            .header("set-cookie", "tab_sid=p99; Path=/")
            .body("<a href=\"/tab/next\">next</a>"),
        "/tab/next" => {
            if tab_retry_bg.fetch_add(1, Ordering::SeqCst) == 0 {
                Response::new(503).body("retry-me")
            } else {
                Response::new(200).body(format!(
                    "tab-ok:ref={}:cookie={}:sig={}",
                    req.header("referer").unwrap_or("none"),
                    req.header("cookie").unwrap_or("none"),
                    req.header("x-sig").unwrap_or("none"),
                ))
            }
        }
        "/robots.txt" => Response::new(404),
        "/crawl/root" => Response::new(200)
            .header("content-type", "text/html")
            .body("<a href=\"/crawl/leaf\">leaf</a><p>crawl root via proxy</p>"),
        "/crawl/leaf" => Response::new(200)
            .header("content-type", "text/html")
            .body("<p>crawl leaf via proxy</p>"),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let (proxy_a, connect_a, forward_a) = spawn_connect_proxy().await;
    let (proxy_b, connect_b, forward_b) = spawn_connect_proxy().await;

    #[cfg(feature = "bench-internals")]
    let (h2_port, h2_ca_der) = super::h2_support::spawn_h2_static_server(b"h2-via-connect").await;

    let tls = leyline::Tls::new().system_roots(false).env_roots(false);
    #[cfg(feature = "bench-internals")]
    let tls = tls.ca_der(h2_ca_der);

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .proxy(Proxy::list([proxy_a.as_str(), proxy_b.as_str()]).header("x-edge", "cf-pop"))
        .tls(tls)
        .cache(1 << 20)
        .retry(Retry::new(2).backoff(Duration::from_millis(5), Duration::from_millis(10)))
        .redirect(Redirect::limit(3))
        .before_send(|req| {
            req.headers_mut()
                .insert("x-sig", http::HeaderValue::from_static("signed-v1"));
            Ok(())
        })
        .build()
        .unwrap();

    let first = session.get(origin.url("/cached")).send().await.unwrap();
    assert!(!first.from_cache());
    assert_eq!(first.text().await.unwrap(), "cached:sig=signed-v1");
    let second = session.get(origin.url("/cached")).send().await.unwrap();
    assert!(
        !second.from_cache(),
        "second proxy in Proxy::list has its own cache partition"
    );
    assert_eq!(second.text().await.unwrap(), "cached:sig=signed-v1");
    let third = session.get(origin.url("/cached")).send().await.unwrap();
    assert!(
        third.from_cache(),
        "rotating back to first proxy must hit its cached entry"
    );
    assert_eq!(third.text().await.unwrap(), "cached:sig=signed-v1");

    let dl_path = dir.path("via-proxy.bin");
    let written = session
        .get(origin.url("/dl"))
        .sha256(artifact_sha)
        .download_to(&dl_path)
        .await
        .unwrap();
    assert_eq!(written, artifact.len() as u64);
    assert_eq!(std::fs::read(&dl_path).unwrap(), artifact);

    let tab = session.tab();
    let _ = tab.open(origin.url("/tab/start")).await.unwrap();
    let next = tab
        .follow(origin.url("/tab/next"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(next.contains("/tab/start"), "{next}");
    assert!(next.contains("tab_sid=p99"), "{next}");
    assert!(next.contains("sig=signed-v1"), "{next}");
    assert_eq!(tab_retry_hits.load(Ordering::SeqCst), 2);

    #[cfg(feature = "html")]
    {
        let ckpt = dir.path("proxy-crawl.json");
        let mut crawl = session.crawl(origin.url("/crawl/root")).host_limits(
            leyline::HostLimits::new()
                .max_in_flight(1)
                .per_second(100.0),
        );
        let root_page = crawl.next().await.unwrap().unwrap();
        assert_eq!(root_page.url().path(), "/crawl/root");
        crawl.save(&ckpt).await.unwrap();
        let mut resumed = leyline::Crawl::load(&session, &ckpt).unwrap();
        let leaf_page = resumed.next().await.unwrap().unwrap();
        assert_eq!(leaf_page.url().path(), "/crawl/leaf");
        assert!(resumed.next().await.is_none());
    }

    assert!(
        forward_a.load(Ordering::SeqCst) + forward_b.load(Ordering::SeqCst) >= 4,
        "HTTP requests, download, Tab, and Crawl must route through Proxy::list with x-edge header"
    );

    #[cfg(feature = "bench-internals")]
    {
        let h2_resp = session
            .get(format!("https://127.0.0.1:{h2_port}/tunnel"))
            .protocol(leyline::Protocol::Http2)
            .text()
            .await
            .unwrap();
        assert_eq!(h2_resp, "h2-via-connect");
        assert!(
            connect_a.load(Ordering::SeqCst) + connect_b.load(Ordering::SeqCst) >= 1,
            "HTTPS/2 via Proxy::list must tunnel over CONNECT carrying x-edge: cf-pop"
        );
    }
    #[cfg(not(feature = "bench-internals"))]
    let _ = (connect_a, connect_b);
}

#[test]
fn combo_run_per_core_with_proxy_list_before_send_cookies_save_load_doh_and_h3() {
    let dir = TempDir::new("combo-per-core");
    let state_path = dir.path("per-core-state.json");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let (origin_port, proxy_url, proxy_fwd, doh_url, doh_tls, h3_info, _guards) =
        rt.block_on(async {
            let origin = Server::http(|req| {
                Response::new(200)
                    .header("set-cookie", "pc=shared-42; Path=/")
                    .body(format!(
                        "pc:sig={}:cookie={}",
                        req.header("x-core-sig").unwrap_or("none"),
                        req.header("cookie").unwrap_or("none"),
                    ))
            })
            .await
            .unwrap();
            let origin_port = origin.addr().port();

            let (proxy_url, _c, proxy_fwd) = spawn_connect_proxy().await;

            let doh = Server::https(|req| {
                let body = req.body();
                let Some((name, qtype)) = decode_qname_and_qtype(body) else {
                    return Response::new(400);
                };
                let wire = if (name == "pc.combo.test" || name == "h3.combo.test") && qtype == 1 {
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

            #[cfg(feature = "http3")]
            let h3_info = {
                let (h3_cert, h3_key) = super::tls_support::self_signed_dns("h3.combo.test");
                let h3_ca = h3_cert.to_der().unwrap();
                let h3 = super::h3_support::h3_server_with_tls(
                    h3_cert,
                    h3_key,
                    |_| {},
                    vec![super::h3_support::Reply::Body(7)],
                    super::h3_support::Limits::default(),
                )
                .await;
                (h3.port, h3_ca, h3)
            };
            #[cfg(not(feature = "http3"))]
            let h3_info = ();

            (
                origin_port,
                proxy_url,
                proxy_fwd,
                doh_url,
                doh_tls,
                h3_info,
                (origin, doh),
            )
        });

    let tls = doh_tls;
    #[cfg(feature = "http3")]
    let tls = tls.ca_der(h3_info.1.clone());

    let doh_h1_url = format!("http://pc.combo.test:{origin_port}/work");
    let proxied_url = format!("http://127.0.0.1:{origin_port}/proxied");
    #[cfg(feature = "http3")]
    let h3_url = format!("https://h3.combo.test:{}/h3", h3_info.0);
    let saved_once = Arc::new(AtomicUsize::new(0));

    let results = Session::builder()
        .browser(Browser::Chrome154)
        .doh(doh_url)
        .tls(tls)
        .proxy(
            Proxy::list([proxy_url.as_str()])
                .header("x-edge", "cf-pop")
                .no_proxy(["pc.combo.test", "h3.combo.test"]),
        )
        .before_send(|req| {
            req.headers_mut()
                .insert("x-core-sig", http::HeaderValue::from_static("core-ok"));
            Ok(())
        })
        .timeout(Duration::from_secs(5))
        .run_per_core(|session| {
            let doh_h1_url = doh_h1_url.clone();
            let proxied_url = proxied_url.clone();
            #[cfg(feature = "http3")]
            let h3_url = h3_url.clone();
            let saved_once = Arc::clone(&saved_once);
            let state_path = state_path.clone();
            async move {
                let body = session.get(&doh_h1_url).text().await.unwrap();
                assert!(body.starts_with("pc:sig=core-ok:"), "{body}");
                let via_proxy = session.get(&proxied_url).text().await.unwrap();
                assert!(via_proxy.starts_with("pc:sig=core-ok:"), "{via_proxy}");
                #[cfg(feature = "http3")]
                {
                    let h3_bytes = session
                        .get(&h3_url)
                        .protocol(leyline::Protocol::Http3)
                        .bytes()
                        .await
                        .unwrap();
                    assert_eq!(h3_bytes.len(), 7);
                }
                if saved_once.fetch_add(1, Ordering::SeqCst) == 0 {
                    session.save(&state_path).unwrap();
                }
                session.cookies().len()
            }
        })
        .unwrap();

    assert!(!results.is_empty());
    assert!(results.iter().all(|&n| n >= 1));
    assert!(
        proxy_fwd.load(Ordering::SeqCst) >= 1,
        "run_per_core must route proxied traffic through Proxy::list with x-edge header"
    );

    let reloaded = Session::load(&state_path).unwrap().build().unwrap();
    let cookies = reloaded
        .cookies()
        .cookies_for(format!("http://pc.combo.test:{origin_port}/work"));
    assert!(
        cookies
            .iter()
            .any(|c| c.name() == "pc" && c.value() == "shared-42"),
        "saved per-core session must persist cookies across Session::load: {cookies:?}"
    );

    drop(h3_info);
    drop(_guards);
    drop(rt);
}
