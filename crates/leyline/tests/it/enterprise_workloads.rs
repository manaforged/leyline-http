use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use leyline::testing::{Response, Server};
use leyline::{Browser, Kind, Proxy, Redirect, Retry, Session};
use sha2::{Digest, Sha256};

use super::v030_workloads::TempDir;

#[tokio::test]
async fn enterprise_auth_thundering_herd_singleflight_refresh_and_cross_origin_strip() {
    let origin_b = Server::http(|req| {
        let auth = req.header("authorization").unwrap_or("none");
        Response::new(200).body(format!("auth={auth}"))
    })
    .await
    .unwrap();
    let origin_b_landing = origin_b.url("/landing");

    let origin_a = Server::http(move |req| match req.target() {
        "/protected" => {
            if req.header("authorization") == Some("Bearer fresh-token") {
                Response::new(200).body("ok:fresh")
            } else {
                Response::new(401).body("expired")
            }
        }
        "/handoff" => Response::new(302).header("location", origin_b_landing.clone()),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let refresh_calls = Arc::new(AtomicUsize::new(0));
    let refresh_bg = Arc::clone(&refresh_calls);
    let session = Session::builder()
        .base_url(origin_a.url("/"))
        .bearer_auth("stale-token")
        .refresh_token(move || {
            let refresh_bg = Arc::clone(&refresh_bg);
            async move {
                refresh_bg.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(40)).await;
                Ok("fresh-token".to_owned())
            }
        })
        .redirect(Redirect::limit(5))
        .build()
        .unwrap();

    let mut handles = Vec::with_capacity(16);
    for _ in 0..16 {
        let s = session.clone();
        handles.push(tokio::spawn(async move {
            s.get("/protected").text().await.unwrap()
        }));
    }
    for handle in handles {
        assert_eq!(handle.await.unwrap(), "ok:fresh");
    }
    assert_eq!(
        refresh_calls.load(Ordering::SeqCst),
        1,
        "16 concurrent 401s must coalesce into a single token refresh"
    );

    let handoff = session.get("/handoff").text().await.unwrap();
    assert_eq!(
        handoff, "auth=none",
        "cross-origin redirect must strip refreshed Authorization header"
    );
}

#[tokio::test]
async fn enterprise_resumable_download_sha256_progress_and_corruption_detection() {
    let dir = TempDir::new("enterprise-download");
    let payload: Vec<u8> = (0..131_072usize)
        .map(|i| ((i.wrapping_mul(31)) ^ (i >> 8)) as u8)
        .collect();
    let expected_sha: [u8; 32] = Sha256::digest(&payload).into();
    let payload_arc = Arc::new(payload.clone());
    let mut corrupt = payload.clone();
    corrupt[1024] ^= 0xFF;
    let corrupt_arc = Arc::new(corrupt);

    let attempt = Arc::new(AtomicUsize::new(0));
    let attempt_bg = Arc::clone(&attempt);
    let ranges_seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let ranges_bg = Arc::clone(&ranges_seen);

    let server = Server::http(move |req| match req.target() {
        "/artifact" => {
            if let Some(range) = req.header("range") {
                ranges_bg.lock().unwrap().push(range.to_owned());
            }
            let n = attempt_bg.fetch_add(1, Ordering::SeqCst);
            let total = payload_arc.len();
            match n {
                0 => Response::new(200)
                    .header("etag", "\"art-v1\"")
                    .header("accept-ranges", "bytes")
                    .body(payload_arc.as_ref().clone())
                    .abort_after(32_768),
                1 => {
                    let start = 32_768usize;
                    Response::new(206)
                        .header("etag", "\"art-v1\"")
                        .header(
                            "content-range",
                            format!("bytes {start}-{}/{total}", total - 1),
                        )
                        .body(payload_arc[start..].to_vec())
                        .abort_after(32_768)
                }
                _ => {
                    let start = 65_536usize;
                    Response::new(206)
                        .header("etag", "\"art-v1\"")
                        .header(
                            "content-range",
                            format!("bytes {start}-{}/{total}", total - 1),
                        )
                        .body(payload_arc[start..].to_vec())
                }
            }
        }
        "/corrupt" => Response::new(200)
            .header("etag", "\"art-bad\"")
            .body(corrupt_arc.as_ref().clone()),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let progress_log = Arc::new(Mutex::new(Vec::<(u64, Option<u64>)>::new()));
    let progress_cb = Arc::clone(&progress_log);
    let session = Session::builder().build().unwrap();
    let dest = dir.path("artifact.bin");
    let written = session
        .get(server.url("/artifact"))
        .retry(Retry::new(4).backoff(Duration::from_millis(5), Duration::from_millis(10)))
        .sha256(expected_sha)
        .progress(move |done, total| {
            progress_cb.lock().unwrap().push((done, total));
        })
        .download_to(&dest)
        .await
        .unwrap();

    assert_eq!(written, payload.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), payload);
    assert_eq!(
        *ranges_seen.lock().unwrap(),
        vec!["bytes=32768-".to_owned(), "bytes=65536-".to_owned()]
    );
    let points = progress_log.lock().unwrap().clone();
    assert!(!points.is_empty());
    assert!(
        points.windows(2).all(|w| w[0].0 <= w[1].0),
        "download progress must be monotonic"
    );
    assert_eq!(points.last().copied(), Some((131_072, Some(131_072))));

    let bad_dest = dir.path("corrupt.bin");
    let err = session
        .get(server.url("/corrupt"))
        .retry(0)
        .sha256(expected_sha)
        .download_to(&bad_dest)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Body);
    assert!(
        !bad_dest.exists(),
        "corrupted download must not be committed to destination"
    );
}

#[tokio::test]
async fn enterprise_sse_reconnect_last_event_id_and_paginated_link_stream() {
    let last_ids = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
    let last_ids_bg = Arc::clone(&last_ids);
    let server = Server::http(move |req| match req.target() {
        "/events" => {
            let last = req.header("last-event-id").map(str::to_owned);
            last_ids_bg.lock().unwrap().push(last.clone());
            match last.as_deref() {
                None => Response::new(200)
                    .header("content-type", "text/event-stream")
                    .body("retry: 10\nid: 1\ndata: alpha\n\nid: 2\ndata: beta\n\n"),
                Some("2") => Response::new(200)
                    .header("content-type", "text/event-stream")
                    .body("id: 3\ndata: gamma\n\nid: 4\ndata: delta\n\n"),
                Some("4") => Response::new(204),
                _ => Response::new(500),
            }
        }
        "/pages?p=1" => Response::new(200)
            .header("set-cookie", "cursor=ok; Path=/")
            .header("link", "</pages?p=2>; rel=\"next\"")
            .body(format!("p1:{}", req.header("x-page-sig").unwrap_or("none"))),
        "/pages?p=2" => Response::new(200)
            .header("link", "</pages?p=3>; rel=\"next\"")
            .body(format!(
                "p2:{}:{}",
                req.header("x-page-sig").unwrap_or("none"),
                req.header("cookie").unwrap_or("none")
            )),
        "/pages?p=3" => Response::new(200).body(format!(
            "p3:{}:{}",
            req.header("x-page-sig").unwrap_or("none"),
            req.header("cookie").unwrap_or("none")
        )),
        _ => Response::new(404),
    })
    .await
    .unwrap();

    let sig_seq = Arc::new(AtomicUsize::new(0));
    let sig_hook = Arc::clone(&sig_seq);
    let session = Session::builder()
        .before_send(move |req| {
            let n = sig_hook.fetch_add(1, Ordering::SeqCst);
            req.headers_mut().insert(
                "x-page-sig",
                http::HeaderValue::from_str(&format!("s{n}")).unwrap(),
            );
            Ok(())
        })
        .build()
        .unwrap();

    let mut events = session
        .get(server.url("/events"))
        .events()
        .unwrap()
        .retry(Retry::new(3).backoff(Duration::from_millis(5), Duration::from_millis(10)));
    let mut got = Vec::new();
    while let Some(ev) = events.next().await {
        let ev = ev.unwrap();
        got.push((ev.id().to_owned(), ev.data().to_owned()));
    }
    assert_eq!(
        got,
        vec![
            ("1".into(), "alpha".into()),
            ("2".into(), "beta".into()),
            ("3".into(), "gamma".into()),
            ("4".into(), "delta".into()),
        ]
    );
    assert_eq!(
        *last_ids.lock().unwrap(),
        vec![None, Some("2".into()), Some("4".into())]
    );

    let mut pages = session.get(server.url("/pages?p=1")).pages();
    let mut bodies = Vec::new();
    while let Some(resp) = pages.next().await {
        bodies.push(resp.unwrap().text().await.unwrap());
    }
    assert_eq!(bodies.len(), 3);
    assert!(bodies[0].starts_with("p1:s"));
    assert!(bodies[1].contains("cursor=ok"));
    assert!(bodies[2].contains("cursor=ok"));
}

#[tokio::test]
async fn enterprise_proxy_pool_mixed_http_ws_failover_ban_expiry_and_bypass() {
    let origin = Server::http(|req| Response::new(200).body(format!("direct:{}", req.target())))
        .await
        .unwrap();
    let origin_port = origin.addr().port();

    let flaky_healthy = Arc::new(AtomicBool::new(false));
    let flaky_flag = Arc::clone(&flaky_healthy);
    let flaky_hits = Arc::new(AtomicUsize::new(0));
    let flaky_hits_bg = Arc::clone(&flaky_hits);
    let flaky_proxy = Server::http(move |req| {
        flaky_hits_bg.fetch_add(1, Ordering::SeqCst);
        let fleet = req.header("x-fleet").unwrap_or("missing");
        if flaky_flag.load(Ordering::SeqCst) {
            Response::new(200).body(format!("flaky-recovered:{fleet}"))
        } else {
            Response::new(403)
                .header("cf-mitigated", "challenge")
                .body("challenge")
        }
    })
    .await
    .unwrap();

    let healthy_hits = Arc::new(AtomicUsize::new(0));
    let healthy_hits_bg = Arc::clone(&healthy_hits);
    let healthy_proxy = Server::http(move |req| {
        healthy_hits_bg.fetch_add(1, Ordering::SeqCst);
        let fleet = req.header("x-fleet").unwrap_or("missing");
        Response::new(200).body(format!("healthy:{fleet}"))
    })
    .await
    .unwrap();

    let dead_url = "http://127.0.0.1:1".to_owned();
    let flaky_url = format!("http://{}", flaky_proxy.addr());
    let healthy_url = format!("http://{}", healthy_proxy.addr());

    let session = Session::builder()
        .resolve("target.fleet.test", [origin.addr()])
        .resolve("bypass.fleet.test", [origin.addr()])
        .proxy(
            Proxy::list([dead_url.as_str(), flaky_url.as_str(), healthy_url.as_str()])
                .ban_after(1)
                .ban_for(Duration::from_millis(300))
                .header("x-fleet", "prod")
                .no_proxy(["bypass.fleet.test"]),
        )
        .retry(0)
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();

    let bypassed = session
        .get(format!("http://bypass.fleet.test:{origin_port}/health"))
        .text()
        .await
        .unwrap();
    assert_eq!(bypassed, "direct:/health");
    assert_eq!(flaky_hits.load(Ordering::SeqCst), 0);
    assert_eq!(healthy_hits.load(Ordering::SeqCst), 0);

    let target = format!("http://target.fleet.test:{origin_port}/work");
    let mut saw_healthy = false;
    for _ in 0..4 {
        if let Ok(resp) = session.get(&target).send().await {
            let status = resp.status();
            let text = resp.text().await.unwrap();
            if status == 200 && text == "healthy:prod" {
                saw_healthy = true;
            }
        }
    }
    assert!(
        saw_healthy,
        "traffic must fail over to healthy proxy with custom header intact"
    );
    assert!(
        session.stats().proxy_banned_until(&dead_url).is_some(),
        "dead proxy must be banned after failure"
    );
    assert!(
        session.stats().proxy_banned_until(&flaky_url).is_some(),
        "challenged proxy must be banned after block response"
    );

    flaky_healthy.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(350)).await;
    let mut saw_recovered = false;
    for _ in 0..4 {
        if let Ok(resp) = session.get(&target).send().await
            && resp.text().await.unwrap() == "flaky-recovered:prod"
        {
            saw_recovered = true;
            break;
        }
    }
    assert!(
        saw_recovered,
        "flaky proxy must re-enter pool rotation once ban_for duration expires"
    );

    #[cfg(feature = "websocket")]
    {
        let ws_err = session
            .get(format!("wss://target.fleet.test:{origin_port}/ws"))
            .websocket()
            .await
            .unwrap_err();
        assert!(
            ws_err.is_proxy() || matches!(ws_err.kind(), Kind::Proxy | Kind::Tls | Kind::Io),
            "wss:// through proxy pool must traverse proxy pool rather than fail at URL check: {ws_err:?}"
        );
    }
}

#[tokio::test]
async fn enterprise_h1_h2_h3_concurrent_chaos_cancel_safety_and_graceful_shutdown() {
    let post_hits = Arc::new(AtomicUsize::new(0));
    let post_hits_bg = Arc::clone(&post_hits);
    let h1 = Server::http(move |req| match req.target() {
        "/fast" => Response::new(200).body("h1-ok"),
        "/slow" => Response::new(200)
            .delay(Duration::from_millis(400))
            .body("h1-slow"),
        "/close" => Response::new(200)
            .header("connection", "close")
            .body("h1-closed"),
        "/post-once" => {
            post_hits_bg.fetch_add(1, Ordering::SeqCst);
            Response::new(200).body("h1-drop").abort_after(0)
        }
        _ => Response::new(404),
    })
    .await
    .unwrap();

    #[cfg(feature = "bench-internals")]
    let (h2_port, h2_ca_der) = super::h2_support::spawn_h2_static_server(b"h2-ok").await;

    #[cfg(feature = "http3")]
    let h3 = super::h3_support::h3_server(
        vec![super::h3_support::Reply::Body(5)],
        super::h3_support::Limits::default(),
    )
    .await;

    let tls = leyline::Tls::new().system_roots(false).env_roots(false);
    #[cfg(feature = "bench-internals")]
    let tls = tls.ca_der(h2_ca_der);
    let session = Session::builder()
        .browser(Browser::Chrome154)
        .tls(tls)
        .retry(Retry::new(2).backoff(Duration::from_millis(5), Duration::from_millis(10)))
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let mut wave = Vec::new();
    for i in 0..24 {
        let s = session.clone();
        let url = if i % 4 == 0 {
            h1.url("/close")
        } else {
            h1.url("/fast")
        };
        wave.push(tokio::spawn(async move {
            let body = s.get(url).text().await.unwrap();
            assert!(body == "h1-ok" || body == "h1-closed");
        }));
    }
    #[cfg(feature = "bench-internals")]
    for _ in 0..24 {
        let s = session.clone();
        let url = format!("https://127.0.0.1:{h2_port}/mux");
        wave.push(tokio::spawn(async move {
            let body = s
                .get(url)
                .protocol(leyline::Protocol::Http2)
                .text()
                .await
                .unwrap();
            assert_eq!(body, "h2-ok");
        }));
    }
    for task in wave {
        task.await.unwrap();
    }

    #[cfg(feature = "http3")]
    {
        let h3_session = h3.session().build().unwrap();
        let mut h3_wave = Vec::new();
        for _ in 0..12 {
            let s = h3_session.clone();
            let url = h3.url();
            h3_wave.push(tokio::spawn(async move {
                let bytes = s.get(url).bytes().await.unwrap();
                assert_eq!(bytes.len(), 5);
            }));
        }
        for task in h3_wave {
            task.await.unwrap();
        }
        h3_session.shutdown().await;
        let after_h3 = h3_session.get(h3.url()).send().await.unwrap_err();
        assert!(after_h3.is_shut_down());
    }

    let mut cancels = Vec::new();
    for _ in 0..8 {
        let s = session.clone();
        let url = h1.url("/slow");
        cancels.push(tokio::spawn(async move {
            drop(tokio::time::timeout(Duration::from_millis(25), s.get(url).send()).await);
        }));
    }
    for task in cancels {
        task.await.unwrap();
    }
    assert_eq!(session.get(h1.url("/fast")).text().await.unwrap(), "h1-ok");
    assert_eq!(
        session.stats().in_flight(h1.url("/")),
        0,
        "cancelled requests must not leak in-flight permits"
    );

    let post_err = session
        .post(h1.url("/post-once"))
        .body("mutating-payload")
        .send()
        .await
        .unwrap_err();
    assert_eq!(post_err.kind(), Kind::Io);
    assert_eq!(
        post_hits.load(Ordering::SeqCst),
        1,
        "non-idempotent POST must not be retried automatically after drop"
    );

    let slow_s = session.clone();
    let slow_url = h1.url("/slow");
    let inflight = tokio::spawn(async move { slow_s.get(slow_url).send().await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    session.shutdown().await;
    match inflight.await.unwrap() {
        Ok(resp) => assert_eq!(resp.status().as_u16(), 200),
        Err(err) => assert!(
            err.is_shut_down(),
            "in-flight request during shutdown must complete or report shut_down: {err:?}"
        ),
    }

    let after = session.get(h1.url("/fast")).send().await.unwrap_err();
    assert!(after.is_shut_down());
    assert!(!after.is_retryable());
}
