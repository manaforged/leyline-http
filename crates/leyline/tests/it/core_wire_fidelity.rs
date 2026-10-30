use leyline::testing::{self, queue};
use leyline::{Browser, HeaderPosition, Platform, Protocol, RequestKind, Session};

#[tokio::test]
async fn caller_user_agent_replaces_no_preset_default() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/ua"))
        .header("user-agent", "X")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    assert!(req.request_line.starts_with("GET /ua HTTP/1.1"));
    assert_eq!(req.header_values("user-agent"), vec!["X"]);
    assert_eq!(
        req.header_values("user-agent").len(),
        1,
        "{}",
        String::from_utf8_lossy(&req.raw)
    );
    server.shutdown().await;
}

#[tokio::test]
async fn bulk_headers_replace_all_no_preset_defaults() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/defaults"))
        .headers([
            ("user-agent", "ua-x"),
            ("accept", "accept-y"),
            ("accept-encoding", "encoding-z"),
            ("accept-language", "language-w"),
        ])
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("user-agent"), vec!["ua-x"]);
    assert_eq!(req.header_values("accept"), vec!["accept-y"]);
    assert_eq!(req.header_values("accept-encoding"), vec!["encoding-z"]);
    assert_eq!(req.header_values("accept-language"), vec!["language-w"]);
    assert_eq!(
        req.header_values("user-agent").len(),
        1,
        "{}",
        String::from_utf8_lossy(&req.raw)
    );
    assert_eq!(
        req.header_values("accept").len(),
        1,
        "{}",
        String::from_utf8_lossy(&req.raw)
    );
    assert_eq!(
        req.header_values("accept-encoding").len(),
        1,
        "{}",
        String::from_utf8_lossy(&req.raw)
    );
    assert_eq!(
        req.header_values("accept-language").len(),
        1,
        "{}",
        String::from_utf8_lossy(&req.raw)
    );
    server.shutdown().await;
}

#[tokio::test]
async fn dx_helpers_accept_common_pair_shapes_and_header_shortcuts() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200).body("ok").close(),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();

    let client = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();
    let _explicit = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Windows)
        .build()
        .unwrap();
    let _bare = Session::builder().build().unwrap();
    let _chrome = Session::from(Browser::default());
    let _firefox = Session::builder()
        .browser(Browser::firefox())
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let owned_headers = vec![
        ("x-owned".to_string(), "yes".to_string()),
        ("x-second".to_string(), "also".to_string()),
    ];

    let resp = client
        .request(http::Method::GET, server.url("/dx"))
        .query([("a", "1"), ("space", "hello world")])
        .headers(owned_headers)
        .header("accept", "application/json")
        .header("accept-language", "en-US,en;q=0.9")
        .header("referer", "https://example.test/from")
        .header("origin", "https://example.test")
        .header("cache-control", "no-cache")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    assert!(
        req.request_line
            .starts_with("GET /dx?a=1&space=hello+world HTTP/1.1")
    );
    assert_eq!(req.header_values("x-owned"), vec!["yes"]);
    assert_eq!(req.header_values("x-second"), vec!["also"]);
    assert_eq!(req.header_values("accept"), vec!["application/json"]);
    assert_eq!(req.header_values("accept-language"), vec!["en-US,en;q=0.9"]);
    assert_eq!(
        req.header_values("referer"),
        vec!["https://example.test/from"]
    );
    assert_eq!(req.header_values("origin"), vec!["https://example.test"]);
    assert_eq!(req.header_values("cache-control"), vec!["no-cache"]);

    let form_pairs = [
        ("email".to_string(), "a b@example.test".to_string()),
        ("password".to_string(), "s3cr3t!".to_string()),
    ];

    let resp = client
        .post(server.url("/login"))
        .form(&form_pairs)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    assert!(req.request_line.starts_with("POST /login HTTP/1.1"));
    assert_eq!(
        req.header_values("content-type"),
        vec!["application/x-www-form-urlencoded"]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn append_header_preserves_duplicate_order() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/dup"))
        .header("x-dup", "a")
        .header("x-dup", "b")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("x-dup"), vec!["a", "b"]);
    server.shutdown().await;
}

#[tokio::test]
async fn set_then_append_user_agent_preserves_caller_order() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/ua-append"))
        .header("user-agent", "X")
        .header("user-agent", "Y")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("user-agent"), vec!["X", "Y"]);
    server.shutdown().await;
}

#[tokio::test]
async fn caller_referer_wins_over_navigate_preset_referer() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/nav"))
        .kind(RequestKind::Navigate)
        .header("referer", "https://caller.example/from")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    assert_eq!(
        req.header_values("referer"),
        vec!["https://caller.example/from"]
    );
    assert_eq!(
        req.header_values("referer").len(),
        1,
        "{}",
        String::from_utf8_lossy(&req.raw)
    );
    server.shutdown().await;
}

#[tokio::test]
async fn redirect_cross_origin_strips_authorization_after_first_step() {
    let target = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let target_url = target.url("/landing");
    let redirector = testing::Server::http(queue(vec![
        testing::Response::new(302)
            .close()
            .header("location", target_url),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, redirector.url("/start"))
        .header("authorization", "Bearer secret")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let first = redirector.next_request().await.unwrap();
    let second = target.next_request().await.unwrap();
    assert_eq!(first.header_values("authorization"), vec!["Bearer secret"]);
    assert!(
        second.header_values("authorization").is_empty(),
        "{}",
        second.text()
    );
    redirector.shutdown().await;
    target.shutdown().await;
}

#[tokio::test]
async fn redirect_same_origin_preserves_authorization() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(302)
            .close()
            .header("location", "/landing"),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/start"))
        .header("authorization", "Bearer secret")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let first = server.next_request().await.unwrap();
    let second = server.next_request().await.unwrap();
    assert_eq!(first.header_values("authorization"), vec!["Bearer secret"]);
    assert_eq!(second.header_values("authorization"), vec!["Bearer secret"]);
    server.shutdown().await;
}

#[tokio::test]
async fn caller_dnt_wins_over_edge_brand_overlay() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::edge())
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/edge"))
        .header("dnt", "0")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("dnt"), vec!["0"]);
    assert_eq!(
        req.header_values("dnt").len(),
        1,
        "{}",
        String::from_utf8_lossy(&req.raw)
    );
    server.shutdown().await;
}

#[tokio::test]
async fn anchored_headers_interleave_at_preset_slots() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(leyline::Browser::default())
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .post(server.url("/submit"))
        .kind(RequestKind::Form)
        .header_at(HeaderPosition::AfterCchUa, "x-extra-1", "1")
        .header_at(HeaderPosition::AfterCchUaMobile, "x-extra-2", "2")
        .header_at(HeaderPosition::AfterCchUaMobile, "x-extra-3", "3")
        .header_at(HeaderPosition::AfterCchUaMobile, "x-extra-4", "4")
        .header_at(HeaderPosition::AfterCchUaPlatform, "x-extra-5", "5")
        .header_at(HeaderPosition::AfterUserAgent, "x-extra-6", "6")
        .header_at(HeaderPosition::AfterContentType, "x-extra-7", "7")
        .body("field=payload")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();

    let names: Vec<String> = req.headers.iter().map(|(k, _)| k.clone()).collect();
    let wire = req.text();
    let pos = |n: &str| {
        names
            .iter()
            .position(|h| h.eq_ignore_ascii_case(n))
            .unwrap_or_else(|| panic!("header {n:?} not on wire — full request:\n{wire}"))
    };

    assert_eq!(pos("x-extra-1"), pos("sec-ch-ua") + 1, "{}", wire);
    assert_eq!(pos("x-extra-2"), pos("sec-ch-ua-mobile") + 1, "{}", wire);
    assert_eq!(pos("x-extra-3"), pos("x-extra-2") + 1);
    assert_eq!(pos("x-extra-4"), pos("x-extra-3") + 1);
    assert_eq!(pos("x-extra-5"), pos("sec-ch-ua-platform") + 1);
    assert_eq!(pos("x-extra-6"), pos("user-agent") + 1);
    assert_eq!(pos("x-extra-7"), pos("content-type") + 1);
    assert!(
        req.header_values("cookie").is_empty() || pos("cookie") > pos("x-extra-7"),
        "cookie must ride at the tail if present: {wire}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn plain_authorization_rides_after_user_agent() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .protocol(Protocol::Http1)
        .build()
        .unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/auth"))
        .kind(RequestKind::Xhr)
        .header("authorization", "Bearer tok")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await.unwrap();
    let names: Vec<String> = req.headers.iter().map(|(k, _)| k.clone()).collect();
    let ua = names
        .iter()
        .position(|h| h.eq_ignore_ascii_case("user-agent"))
        .expect("preset emits user-agent");
    let auth = names
        .iter()
        .position(|h| h.eq_ignore_ascii_case("authorization"))
        .expect("caller-set authorization on wire");
    assert_eq!(auth, ua + 1, "{}", String::from_utf8_lossy(&req.raw));
    server.shutdown().await;
}
