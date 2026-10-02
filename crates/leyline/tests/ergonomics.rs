use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use leyline::testing::{TestResponse, TestServer, queue};
use leyline::trace::{Summary, Trace};
use leyline::{
    Browser, ChromiumBrand, CompressionConfig, Family, Identity, Kind, Platform, Preset,
    ProtocolPolicy, RetryPolicy, Session,
};

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn plain() -> Session {
    Session::builder().build().unwrap()
}

#[tokio::test]
async fn decoded_copy_writes_plain_bytes_and_stops_at_the_limit() {
    let text = b"leyline ".repeat(4_000);
    let gz = || {
        TestResponse::new(200)
            .body("ok")
            .close()
            .header("content-encoding", "gzip")
            .body(gzip(&text))
    };
    let server = TestServer::http(queue(vec![gz(), gz()])).await.unwrap();

    let mut out = Vec::new();
    let written = plain()
        .get(server.url("/a"))
        .stream()
        .await
        .unwrap()
        .copy_decoded_to(&mut out, None)
        .await
        .unwrap();
    assert_eq!(out, text);
    assert_eq!(written, text.len() as u64);

    let mut capped = Vec::new();
    let err = plain()
        .get(server.url("/b"))
        .stream()
        .await
        .unwrap()
        .copy_decoded_to(&mut capped, Some(1_000))
        .await
        .unwrap_err();
    assert!(err.is_body_limit(), "{err:?}");
    assert!(capped.len() <= 1_000);
}

#[tokio::test]
async fn body_limit_is_its_own_error() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).close().body(vec![b'x'; 4_096]),
        TestResponse::new(500).close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .compression(CompressionConfig::default().max_body_size(1_024))
        .build()
        .unwrap();
    let over = session.get(server.url("/big")).await.unwrap_err();
    assert!(over.is_body_limit(), "{over:?}");

    let status = session
        .get(server.url("/fail"))
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err();
    assert!(!status.is_body_limit());
}

#[tokio::test]
async fn status_error_keeps_the_body() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(404)
            .close()
            .body(b"no such repo".to_vec()),
    ]))
    .await
    .unwrap();
    let err = plain()
        .get(server.url("/repos/x"))
        .error_for_status()
        .await
        .unwrap_err();
    assert_eq!(err.status().map(|s| s.as_u16()), Some(404));
    assert_eq!(err.body(), Some(&b"no such repo"[..]));
}

#[tokio::test]
async fn proxy_failures_are_told_apart_from_origin_failures() {
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);
    let err = Session::builder()
        .proxy(format!("http://127.0.0.1:{dead_port}"))
        .build()
        .unwrap()
        .get("https://origin.invalid/")
        .await
        .unwrap_err();
    assert!(err.is_proxy(), "{err:?}");

    let gateway = TestServer::http(queue(vec![TestResponse::new(502).close()]))
        .await
        .unwrap();
    let err = Session::builder()
        .proxy(format!("http://{}", gateway.addr()))
        .build()
        .unwrap()
        .get("https://origin.invalid/")
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Proxy);
    assert!(!err.is_proxy(), "{err:?}");
}

#[test]
fn identity_strings_round_trip_for_every_value() {
    for browser in Browser::all() {
        assert_eq!(browser.id().parse::<Browser>().unwrap(), *browser);
        let json = serde_json::to_string(browser).unwrap();
        assert_eq!(serde_json::from_str::<Browser>(&json).unwrap(), *browser);
    }
    for family in Family::all() {
        assert_eq!(family.id().parse::<Family>().unwrap(), *family);
    }
    for platform in Platform::all() {
        assert_eq!(platform.id().parse::<Platform>().unwrap(), *platform);
    }
    for brand in [
        ChromiumBrand::Chrome,
        ChromiumBrand::Edge,
        ChromiumBrand::Opera,
    ] {
        let json = serde_json::to_string(&brand).unwrap();
        assert_eq!(serde_json::from_str::<ChromiumBrand>(&json).unwrap(), brand);
    }
}

#[test]
fn saved_identity_rebuilds_the_same_device() {
    let saved = Identity::locked(Browser::latest(Family::Chrome), Platform::MacOS)
        .with_brand(ChromiumBrand::Edge);
    let json = serde_json::to_string(&saved).unwrap();
    let restored: Identity = serde_json::from_str(&json).unwrap();

    let first = Session::builder().identity(saved).build().unwrap();
    let second = Session::builder().identity(restored).build().unwrap();
    assert_eq!(second.identity().brand(), Some(ChromiumBrand::Edge));
    assert_eq!(second.identity().platform(), Platform::MacOS);
    assert_eq!(
        first.identity().profile_id(),
        second.identity().profile_id()
    );
    assert!(first.identity().profile_id().is_some());

    let other = Session::builder()
        .browser(Browser::latest(Family::Firefox))
        .build()
        .unwrap();
    assert_ne!(first.identity().profile_id(), other.identity().profile_id());

    let round = first.identity().to_identity().unwrap();
    let again = Session::builder().identity(round).build().unwrap();
    assert_eq!(again.identity().profile_id(), first.identity().profile_id());
}

#[tokio::test]
async fn session_bearer_auth_and_base_url() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let api = Session::builder()
        .base_url(server.url("/api/v1/"))
        .bearer_auth("session-token")
        .build()
        .unwrap();
    api.get("repos/x").await.unwrap();
    api.get("/health")
        .bearer_auth("request-token")
        .await
        .unwrap();

    let first = server.next_request().await.unwrap();
    assert_eq!(first.request_line, "GET /api/v1/repos/x HTTP/1.1");
    assert_eq!(
        first.header_values("authorization"),
        ["Bearer session-token"]
    );
    let second = server.next_request().await.unwrap();
    assert_eq!(second.request_line, "GET /health HTTP/1.1");
    assert_eq!(
        second.header_values("authorization"),
        ["Bearer request-token"]
    );

    let err = plain().get("repos/x").await.unwrap_err();
    assert_eq!(err.kind(), Kind::Url);
}

type Seen = (String, Option<u16>, u32, bool);

#[derive(Default)]
struct Summaries(Mutex<Vec<Seen>>);

impl Trace for Summaries {
    fn summary(&self, ev: &Summary<'_>) {
        self.0.lock().unwrap().push((
            ev.url.map(|u| u.path().to_owned()).unwrap_or_default(),
            ev.status.map(|s| s.as_u16()),
            ev.attempts,
            ev.outcome.is_ok(),
        ));
    }
}

#[tokio::test]
async fn one_summary_per_request_counts_retries() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(503).close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let hook = Arc::new(Summaries::default());
    let session = Session::builder()
        .trace(Arc::clone(&hook))
        .retry(
            RetryPolicy::transient()
                .initial_backoff(Duration::from_millis(1))
                .jitter(false),
        )
        .build()
        .unwrap();
    let resp = session.get(server.url("/flaky")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        *hook.0.lock().unwrap(),
        [("/flaky".to_owned(), Some(200), 2, true)]
    );
}

#[tokio::test]
async fn request_cookie_jar_leaves_the_session_jar_alone() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200)
            .body("ok")
            .close()
            .header("set-cookie", "sid=abc; Path=/"),
    ]))
    .await
    .unwrap();
    let session = plain();
    let jar = leyline::cookie::Jar::new();
    session
        .get(server.url("/login"))
        .cookie_jar(jar.clone())
        .await
        .unwrap();
    let url = url::Url::parse(&server.url("/")).unwrap();
    assert_eq!(jar.get_cookie(&url, "sid").as_deref(), Some("abc"));
    assert_eq!(session.cookies().get_cookie(&url, "sid"), None);
}

#[tokio::test]
async fn initiator_sets_referer_origin_and_fetch_site() {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap();
    session
        .post(server.url("/cart"))
        .json(&serde_json::json!({"sku": 1}))
        .preset(Preset::Xhr)
        .initiator("http://shop.example/product/1")
        .await
        .unwrap();
    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("referer"), ["http://shop.example/"]);
    assert_eq!(req.header_values("origin"), ["http://shop.example"]);
    assert_eq!(req.header_values("sec-fetch-site"), ["cross-site"]);
}

#[tokio::test]
async fn retries_rotate_to_the_next_proxy() {
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);
    let good = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let resp = Session::builder()
        .proxy(format!("http://127.0.0.1:{dead_port}"))
        .retry(
            RetryPolicy::transient()
                .initial_backoff(Duration::from_millis(1))
                .jitter(false)
                .rotate_proxies([format!("http://{}", good.addr())]),
        )
        .build()
        .unwrap()
        .get("http://origin.test/item")
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        good.next_request().await.unwrap().request_line,
        "GET http://origin.test/item HTTP/1.1"
    );
}

#[test]
fn backoff_is_public_and_capped() {
    let policy = RetryPolicy::transient()
        .initial_backoff(Duration::from_millis(100))
        .backoff_factor(2.0)
        .max_backoff(Duration::from_millis(300))
        .jitter(false);
    assert_eq!(policy.backoff(0), Duration::from_millis(100));
    assert_eq!(policy.backoff(1), Duration::from_millis(200));
    assert_eq!(policy.backoff(5), Duration::from_millis(300));
}

#[tokio::test]
async fn top_level_get_is_a_plain_request() {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let resp = leyline::get(server.url("/hello")).await.unwrap();
    assert_eq!(resp.text().await.unwrap(), "ok");
    let sent = server.next_request().await.unwrap();
    assert!(sent.header_values("sec-ch-ua").is_empty());
}

#[tokio::test]
async fn new_is_plain_and_browser_is_one_line() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let plain = Session::new();
    assert_eq!(plain.identity().browser(), None);
    plain.get(server.url("/plain")).await.unwrap();
    let sent = server.next_request().await.unwrap();
    assert!(sent.header_values("sec-ch-ua").is_empty());
    assert!(!sent.header_values("user-agent")[0].contains("Chrome"));

    let chrome = Session::browser(Browser::default());
    assert_eq!(chrome.identity().platform(), Platform::Windows);
    chrome.get(server.url("/chrome")).await.unwrap();
    let sent = server.next_request().await.unwrap();
    assert!(sent.header_values("user-agent")[0].contains("Chrome"));
    assert!(!sent.header_values("sec-ch-ua").is_empty());
}

#[test]
fn every_browser_builds_a_one_line_session() {
    for &browser in Browser::all() {
        let identity = Session::browser(browser).identity();
        assert_eq!(identity.browser(), Some(browser), "{browser}");
        assert_ne!(identity.platform(), Platform::Host, "{browser}");
    }
}
