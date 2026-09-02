//! Wire-fidelity integration tests for session header merging.
#[path = "core_support/raw_server.rs"]
mod raw_server;

use leyline::Session;
use leyline::profile::{HeaderAnchor, Preset};
use leyline::{Browser, Platform};
use raw_server::{RawResponse, RawServer};

#[tokio::test]
async fn caller_user_agent_replaces_no_preset_default() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
    let session = Session::builder().http1().build().unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/ua"))
        .header("user-agent", "X")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;
    assert!(req.request_line.starts_with("GET /ua HTTP/1.1"));
    assert_eq!(req.header_values("user-agent"), vec!["X"]);
    assert_eq!(req.header_count("user-agent"), 1, "{}", req.text());
    server.finish().await;
}

#[tokio::test]
async fn bulk_headers_replace_all_no_preset_defaults() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
    let session = Session::builder().http1().build().unwrap();

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
    let req = server.next_request().await;
    assert_eq!(req.header_values("user-agent"), vec!["ua-x"]);
    assert_eq!(req.header_values("accept"), vec!["accept-y"]);
    assert_eq!(req.header_values("accept-encoding"), vec!["encoding-z"]);
    assert_eq!(req.header_values("accept-language"), vec!["language-w"]);
    assert_eq!(req.header_count("user-agent"), 1, "{}", req.text());
    assert_eq!(req.header_count("accept"), 1, "{}", req.text());
    assert_eq!(req.header_count("accept-encoding"), 1, "{}", req.text());
    assert_eq!(req.header_count("accept-language"), 1, "{}", req.text());
    server.finish().await;
}

#[tokio::test]
async fn dx_helpers_accept_common_pair_shapes_and_header_shortcuts() {
    let mut server = RawServer::start(vec![RawResponse::ok(), RawResponse::ok()]).await;

    let client = Session::builder().http1().build().unwrap();
    let _explicit = Session::builder()
        .profile(Browser::Chrome147, Platform::Windows)
        .build()
        .unwrap();
    let _default = Session::new();
    let _chrome = Session::chrome();
    let _firefox = Session::builder().firefox().http1().build().unwrap();

    let owned_headers = vec![
        ("x-owned".to_string(), "yes".to_string()),
        ("x-second".to_string(), "also".to_string()),
    ];

    let resp = client
        .request(http::Method::GET, server.url("/dx"))
        .query([("a", "1"), ("space", "hello world")])
        .headers(&owned_headers)
        .accept("application/json")
        .accept_language("en-US,en;q=0.9")
        .referer("https://example.test/from")
        .origin("https://example.test")
        .header("cache-control", "no-cache")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;
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
        .post(&server.url("/login"))
        .form(&form_pairs)
        .content_type("application/x-www-form-urlencoded; charset=UTF-8")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;
    assert!(req.request_line.starts_with("POST /login HTTP/1.1"));
    assert_eq!(
        req.header_values("content-type"),
        vec!["application/x-www-form-urlencoded; charset=UTF-8"]
    );
    server.finish().await;
}

#[tokio::test]
async fn append_header_preserves_duplicate_order() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
    let session = Session::builder().http1().build().unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/dup"))
        .append_header("x-dup", "a")
        .append_header("x-dup", "b")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;
    assert_eq!(req.header_values("x-dup"), vec!["a", "b"]);
    server.finish().await;
}

#[tokio::test]
async fn set_then_append_user_agent_preserves_caller_order() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
    let session = Session::builder().http1().build().unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/ua-append"))
        .header("user-agent", "X")
        .append_header("user-agent", "Y")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;
    assert_eq!(req.header_values("user-agent"), vec!["X", "Y"]);
    server.finish().await;
}

#[tokio::test]
async fn caller_referer_wins_over_navigate_preset_referer() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
    let session = Session::builder().http1().build().unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/nav"))
        .preset(Preset::Navigate)
        .header("referer", "https://caller.example/from")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;
    assert_eq!(
        req.header_values("referer"),
        vec!["https://caller.example/from"]
    );
    assert_eq!(req.header_count("referer"), 1, "{}", req.text());
    server.finish().await;
}

#[tokio::test]
async fn redirect_cross_origin_strips_authorization_after_first_hop() {
    let mut target = RawServer::start(vec![RawResponse::ok()]).await;
    let target_url = target.url("/landing");
    let mut redirector = RawServer::start(vec![RawResponse::redirect(target_url)]).await;
    let session = Session::builder().http1().build().unwrap();

    let resp = session
        .request(http::Method::GET, redirector.url("/start"))
        .header("authorization", "Bearer secret")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let first = redirector.next_request().await;
    let second = target.next_request().await;
    assert_eq!(first.header_values("authorization"), vec!["Bearer secret"]);
    assert!(
        second.header_values("authorization").is_empty(),
        "{}",
        second.text()
    );
    redirector.finish().await;
    target.finish().await;
}

#[tokio::test]
async fn redirect_same_origin_preserves_authorization() {
    let mut server =
        RawServer::start(vec![RawResponse::redirect("/landing"), RawResponse::ok()]).await;
    let session = Session::builder().http1().build().unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/start"))
        .header("authorization", "Bearer secret")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let first = server.next_request().await;
    let second = server.next_request().await;
    assert_eq!(first.header_values("authorization"), vec!["Bearer secret"]);
    assert_eq!(second.header_values("authorization"), vec!["Bearer secret"]);
    server.finish().await;
}

#[tokio::test]
async fn caller_dnt_wins_over_edge_brand_overlay() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
    let session = Session::builder().http1().edge().build().unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/edge"))
        .header("dnt", "0")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;
    assert_eq!(req.header_values("dnt"), vec!["0"]);
    assert_eq!(req.header_count("dnt"), 1, "{}", req.text());
    server.finish().await;
}

/// A site can emit 7 headers at 5 different anchor points.
#[tokio::test]
async fn anchored_headers_interleave_at_preset_slots() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
    let session = Session::builder().http1().build().unwrap();

    let resp = session
        .post(&server.url("/collect"))
        .preset(Preset::Form)
        .anchored(HeaderAnchor::AfterCchUa, "x-extra-1", "f")
        .anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-2", "a0")
        .anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-3", "b")
        .anchored(HeaderAnchor::AfterCchUaMobile, "x-extra-4", "a")
        .anchored(HeaderAnchor::AfterCchUaPlatform, "x-extra-5", "z")
        .anchored(HeaderAnchor::AfterUserAgent, "x-extra-6", "c")
        .anchored(HeaderAnchor::AfterContentType, "x-extra-7", "d")
        .body("field=payload")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;

    let names: Vec<String> = req.headers.iter().map(|(k, _)| k.clone()).collect();
    let wire = req.text();
    let pos = |n: &str| {
        names
            .iter()
            .position(|h| h.eq_ignore_ascii_case(n))
            .unwrap_or_else(|| panic!("header {n:?} not on wire — full request:\n{wire}"))
    };

    assert_eq!(pos("x-extra-1"), pos("sec-ch-ua") + 1, "{}", wire);
    assert_eq!(
        pos("x-extra-2"),
        pos("sec-ch-ua-mobile") + 1,
        "{}",
        wire
    );
    assert_eq!(pos("x-extra-3"), pos("x-extra-2") + 1);
    assert_eq!(pos("x-extra-4"), pos("x-extra-3") + 1);
    assert_eq!(pos("x-extra-5"), pos("sec-ch-ua-platform") + 1);
    assert_eq!(pos("x-extra-6"), pos("user-agent") + 1);
    assert_eq!(pos("x-extra-7"), pos("content-type") + 1);
    assert!(
        req.header_count("cookie") == 0 || pos("cookie") > pos("x-extra-7"),
        "cookie must ride at the tail if present: {wire}"
    );
    server.finish().await;
}

/// A plain `.header("authorization", ...)` rides at the inferred `AfterUserAgent` slot without the caller naming it, because `leyline::profile::infer_anchor` recognises the well-known name.
#[tokio::test]
async fn plain_authorization_rides_after_user_agent() {
    let mut server = RawServer::start(vec![RawResponse::ok()]).await;
    let session = Session::builder().http1().build().unwrap();

    let resp = session
        .request(http::Method::GET, server.url("/auth"))
        .preset(Preset::Xhr)
        .header("authorization", "Bearer tok")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let req = server.next_request().await;
    let names: Vec<String> = req.headers.iter().map(|(k, _)| k.clone()).collect();
    let ua = names
        .iter()
        .position(|h| h.eq_ignore_ascii_case("user-agent"))
        .expect("preset emits user-agent");
    let auth = names
        .iter()
        .position(|h| h.eq_ignore_ascii_case("authorization"))
        .expect("caller-set authorization on wire");
    assert_eq!(auth, ua + 1, "{}", req.text());
    server.finish().await;
}
