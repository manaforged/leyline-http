use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use leyline::testing::{Response, Server};
use leyline::{Browser, Kind, Session, Timeout};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[cfg(feature = "html")]
use super::v030_workloads::TempDir;

#[tokio::test]
async fn enterprise_decompression_bomb_cap_slow_header_timeout_and_tab_form_flow() {
    #[cfg(feature = "compression-gzip")]
    let bomb_gz = super::v030_workloads::gzip_bytes(&vec![0u8; 4 * 1024 * 1024]);

    let stall_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stall_port = stall_listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = stall_listener.accept().await {
            let mut req = [0u8; 512];
            drop(sock.read(&mut req).await);
            tokio::time::sleep(Duration::from_secs(2)).await;
            drop(
                sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                    .await,
            );
        }
    });

    let server = Server::http(move |req| match req.target() {
        #[cfg(feature = "compression-gzip")]
        "/bomb" => Response::new(200)
            .header("content-encoding", "gzip")
            .body(bomb_gz.clone()),
        "/login" => Response::new(200)
            .header("content-type", "text/html")
            .header("set-cookie", "csrf_sid=tok42; Path=/")
            .body(
                "<form method=\"POST\" action=\"/auth\">\
                 <input type=\"hidden\" name=\"csrf\" value=\"tok42\">\
                 <input type=\"text\" name=\"user\" value=\"alice\">\
                 </form>",
            ),
        "/auth" => {
            let referer = req.header("referer").unwrap_or("none");
            let cookie = req.header("cookie").unwrap_or("none");
            let mode = req.header("sec-fetch-mode").unwrap_or("none");
            let site = req.header("sec-fetch-site").unwrap_or("none");
            let body = req.text();
            Response::new(200)
                .header("content-type", "text/html")
                .header("set-cookie", "auth=yes; Path=/")
                .body(format!(
                    "auth:ref={referer}:cookie={cookie}:mode={mode}:site={site}:body={body}"
                ))
        }
        "/dashboard" => {
            let referer = req.header("referer").unwrap_or("none");
            let cookie = req.header("cookie").unwrap_or("none");
            let dest = req.header("sec-fetch-dest").unwrap_or("none");
            Response::new(200).body(format!("dash:ref={referer}:cookie={cookie}:dest={dest}"))
        }
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let session = Session::builder()
        .browser(Browser::Chrome154)
        .max_body_size(32 * 1024)
        .timeout(
            Timeout::new()
                .response_header(Duration::from_millis(120))
                .total(Duration::from_secs(3)),
        )
        .build()
        .unwrap();

    #[cfg(feature = "compression-gzip")]
    {
        let err = session.get(server.url("/bomb")).send().await.unwrap_err();
        assert_eq!(
            err.kind(),
            Kind::BodyLimit,
            "4 MiB gzip bomb must abort at 32 KiB max_body_size: {err:?}"
        );
    }

    let stalled = session
        .get(format!("http://127.0.0.1:{stall_port}/stalled"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(stalled.kind(), Kind::Timeout);

    #[cfg(feature = "html")]
    {
        let tab = session.tab();
        let login_html = tab
            .open(server.url("/login"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let forms = leyline::html::forms(&login_html);
        assert_eq!(forms.len(), 1);
        let auth_body = tab
            .submit_form(&forms[0])
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(auth_body.contains("ref=http://127.0.0.1:"), "{auth_body}");
        assert!(auth_body.contains("csrf_sid=tok42"), "{auth_body}");
        assert!(auth_body.contains("mode=navigate"), "{auth_body}");
        assert!(auth_body.contains("site=same-origin"), "{auth_body}");
        assert!(auth_body.contains("csrf=tok42"), "{auth_body}");

        let dash = tab
            .follow(server.url("/dashboard"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            dash.contains("/auth"),
            "follow must send prior page as referer: {dash}"
        );
        assert!(dash.contains("auth=yes"), "{dash}");
        assert!(dash.contains("dest=document"), "{dash}");
    }
}

#[test]
fn enterprise_blocking_multithreaded_sessions_and_resumable_crawl_under_429() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let hits_429 = Arc::new(AtomicUsize::new(0));
    let hits_429_bg = Arc::clone(&hits_429);
    let secret_hits = Arc::new(AtomicUsize::new(0));
    let secret_hits_bg = Arc::clone(&secret_hits);
    let (origin_url, _server_guard) = rt.block_on(async {
        let server = Server::http(move |req| match req.target() {
            "/seed-cookie" => Response::new(200)
                .header("set-cookie", "shared=mt-ok; Path=/")
                .body("seeded"),
            "/echo" => Response::new(200).body(req.header("cookie").unwrap_or("none").to_owned()),
            "/robots.txt" => Response::new(200)
                .header("content-type", "text/plain")
                .body("User-agent: *\nDisallow: /secret\n"),
            "/secret" => {
                secret_hits_bg.fetch_add(1, Ordering::SeqCst);
                Response::new(200)
                    .header("content-type", "text/html")
                    .body("<p>forbidden</p>")
            }
            "/start" => Response::new(200).header("content-type", "text/html").body(
                "<html><body>\
                     <a href=\"/a\">A</a>\
                     <a href=\"/b\">B</a>\
                     <a href=\"/dup\">Dup</a>\
                     <a href=\"/secret\">Secret</a>\
                     <p>start page unique content here</p></body></html>",
            ),
            "/a" => {
                if hits_429_bg.fetch_add(1, Ordering::SeqCst) == 0 {
                    Response::new(429)
                        .header("retry-after", "0")
                        .body("slow down")
                } else {
                    Response::new(200)
                        .header("content-type", "text/html")
                        .body("<html><body><p>page A recovered after 429</p></body></html>")
                }
            }
            "/b" => Response::new(200)
                .header("content-type", "text/html")
                .body("<html><body><p>page B distinct content</p></body></html>"),
            "/dup" => Response::new(200).header("content-type", "text/html").body(
                "<html><head><link rel=\"canonical\" href=\"/b\"></head>\
                     <body><p>duplicate canonical pointing to B</p></body></html>",
            ),
            _ => Response::new(404),
        })
        .await
        .unwrap();
        let url = server.url("").trim_end_matches('/').to_owned();
        (url, server)
    });

    let mut threads = Vec::new();
    for _ in 0..4 {
        let base = origin_url.clone();
        threads.push(std::thread::spawn(move || {
            let blocking = Session::builder().build_blocking().unwrap();
            assert_eq!(
                blocking
                    .block_on(blocking.get(format!("{base}/seed-cookie")).text())
                    .unwrap(),
                "seeded"
            );
            for _ in 0..4 {
                assert_eq!(
                    blocking
                        .block_on(blocking.get(format!("{base}/echo")).text())
                        .unwrap(),
                    "shared=mt-ok"
                );
            }
        }));
    }
    for t in threads {
        t.join().unwrap();
    }

    #[cfg(feature = "html")]
    rt.block_on(async {
        let dir = TempDir::new("enterprise-crawl-429");
        let ckpt = dir.path("crawl.json");
        let session = Session::builder().build().unwrap();
        let mut crawl = session.crawl(format!("{origin_url}/start")).host_limits(
            leyline::HostLimits::new()
                .max_in_flight(1)
                .per_second(100.0)
                .pause_on([429]),
        );
        let first = crawl.next().await.unwrap().unwrap();
        assert_eq!(first.url().path(), "/start");
        crawl.save(&ckpt).await.unwrap();

        let mut resumed = leyline::crawl::Crawl::load(&session, &ckpt).unwrap();
        let mut paths = vec![first.url().path().to_owned()];
        while let Some(page) = resumed.next().await {
            paths.push(page.unwrap().url().path().to_owned());
        }
        paths.sort();
        assert_eq!(paths, vec!["/a", "/b", "/start"]);
        assert_eq!(
            secret_hits.load(Ordering::SeqCst),
            0,
            "robots.txt Disallow: /secret must never be fetched across save/load"
        );
        assert!(
            hits_429.load(Ordering::SeqCst) >= 2,
            "429 Retry-After: 0 must pause and retry /a"
        );
    });

    drop(_server_guard);
    drop(rt);
}
