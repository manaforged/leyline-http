use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use leyline::testing::{self, queue};
use leyline::trace::{Event, Trace};
use leyline::{Browser, Kind, Platform, Protocol, Proxy, RequestKind, Retry, Session};

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
        testing::Response::new(200)
            .body("ok")
            .close()
            .header("content-encoding", "gzip")
            .body(gzip(&text))
    };
    let server = testing::Server::http(queue(vec![gz(), gz()]))
        .await
        .unwrap();

    let mut out = Vec::new();
    let written = plain()
        .get(server.url("/a"))
        .stream()
        .send()
        .await
        .unwrap()
        .write_to(&mut out)
        .await
        .unwrap();
    assert_eq!(out, text);
    assert_eq!(written, text.len() as u64);

    let mut capped = Vec::new();
    let err = plain()
        .get(server.url("/b"))
        .max_body_size(1_000)
        .stream()
        .send()
        .await
        .unwrap()
        .write_to(&mut capped)
        .await
        .unwrap_err();
    assert!(err.kind() == Kind::BodyLimit, "{err:?}");
    assert!(capped.len() <= 1_000);
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
        .send()
        .await
        .unwrap_err();
    assert!(err.is_proxy(), "{err:?}");

    let gateway = testing::Server::http(queue(vec![testing::Response::new(502).close()]))
        .await
        .unwrap();
    let err = Session::builder()
        .proxy(format!("http://{}", gateway.addr()))
        .build()
        .unwrap()
        .get("https://origin.invalid/")
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::ProxyTarget);
    assert!(!err.is_proxy(), "{err:?}");
}

#[test]
fn saved_identity_rebuilds_the_same_device() {
    let first = Session::builder()
        .browser(Browser::edge())
        .platform(Platform::MacOS)
        .build()
        .unwrap();
    let json = serde_json::to_string(&first).unwrap();
    let restored: leyline::SessionBuilder = serde_json::from_str(&json).unwrap();
    let second = restored.build().unwrap();
    assert!(
        second.user_agent().contains(" Edg/"),
        "{}",
        second.user_agent()
    );
    assert_eq!(second.platform(), Platform::MacOS);
    assert_eq!(first.profile_id(), second.profile_id());
    assert!(first.profile_id().is_some());

    let other = Session::builder()
        .browser(Browser::firefox())
        .build()
        .unwrap();
    assert_ne!(first.profile_id(), other.profile_id());

    let round = serde_json::to_string(&first).unwrap();
    let again = serde_json::from_str::<leyline::SessionBuilder>(&round)
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(again.profile_id(), first.profile_id());
}

#[tokio::test]
async fn session_bearer_auth_and_base_url() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200).body("ok").close(),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let api = Session::builder()
        .base_url(server.url("/api/v1/"))
        .bearer_auth("session-token")
        .build()
        .unwrap();
    api.get("repos/x").send().await.unwrap();
    api.get("/health")
        .bearer_auth("request-token")
        .send()
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

    let err = plain().get("repos/x").send().await.unwrap_err();
    assert_eq!(err.kind(), Kind::Url);
}

type Seen = (String, Option<u16>, u32, bool);

#[derive(Default)]
struct Summaries(Mutex<Vec<Seen>>);

impl Trace for Summaries {
    fn event(&self, event: &Event<'_>) {
        if let Event::Summary {
            url,
            status,
            attempts,
            outcome,
            ..
        } = event
        {
            self.0.lock().unwrap().push((
                url.map(|u| u.path().to_owned()).unwrap_or_default(),
                status.map(|s| s.as_u16()),
                *attempts,
                outcome.is_ok(),
            ));
        }
    }
}

#[tokio::test]
async fn one_summary_per_request_counts_retries() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(503).close(),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let hook = Arc::new(Summaries::default());
    let session = Session::builder()
        .trace(Arc::clone(&hook))
        .retry(Retry::new(3).backoff(Duration::from_millis(1), Duration::from_secs(1)))
        .build()
        .unwrap();
    let resp = session.get(server.url("/flaky")).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        *hook.0.lock().unwrap(),
        [("/flaky".to_owned(), Some(200), 2, true)]
    );
}

#[tokio::test]
async fn request_cookie_jar_leaves_the_session_jar_alone() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200)
            .body("ok")
            .close()
            .header("set-cookie", "sid=abc; Path=/"),
    ]))
    .await
    .unwrap();
    let session = plain();
    let jar = leyline::cookie::Jar::new();
    session
        .to_builder()
        .cookies(jar.clone())
        .build()
        .unwrap()
        .get(server.url("/login"))
        .send()
        .await
        .unwrap();
    let url = url::Url::parse(&server.url("/")).unwrap();
    assert_eq!(jar.get(&url, "sid").as_deref(), Some("abc"));
    assert_eq!(session.cookies().get(&url, "sid"), None);
}

#[tokio::test]
async fn initiator_sets_referer_origin_and_fetch_site() {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(Protocol::Http1)
        .build()
        .unwrap();
    session
        .post(server.url("/cart"))
        .json(&serde_json::json!({"sku": 1}))
        .kind(RequestKind::Xhr)
        .from_page("http://shop.example/product/1")
        .send()
        .await
        .unwrap();
    let req = server.next_request().await.unwrap();
    assert_eq!(req.header_values("referer"), ["http://shop.example/"]);
    assert_eq!(req.header_values("origin"), ["http://shop.example"]);
    assert_eq!(req.header_values("sec-fetch-site"), ["cross-site"]);
}

#[tokio::test]
async fn a_retry_moves_to_the_next_pool_proxy() {
    let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_port = dead.local_addr().unwrap().port();
    drop(dead);
    let good = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let resp = Session::builder()
        .proxy(Proxy::list([
            format!("http://127.0.0.1:{dead_port}"),
            format!("http://{}", good.addr()),
        ]))
        .retry(Retry::new(3).backoff(Duration::from_millis(1), Duration::from_secs(1)))
        .build()
        .unwrap()
        .get("http://origin.test/item")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        good.next_request().await.unwrap().request_line,
        "GET http://origin.test/item HTTP/1.1"
    );
}

#[tokio::test]
async fn new_is_plain_and_browser_is_one_line() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200).body("ok").close(),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let plain = Session::new();
    assert_eq!(plain.browser(), None);
    plain.get(server.url("/plain")).send().await.unwrap();
    let sent = server.next_request().await.unwrap();
    assert!(sent.header_values("sec-ch-ua").is_empty());
    assert!(!sent.header_values("user-agent")[0].contains("Chrome"));

    let chrome = Session::from(Browser::default());
    assert_eq!(chrome.platform(), Platform::Windows);
    chrome.get(server.url("/chrome")).send().await.unwrap();
    let sent = server.next_request().await.unwrap();
    assert!(sent.header_values("user-agent")[0].contains("Chrome"));
    assert!(!sent.header_values("sec-ch-ua").is_empty());
}

#[test]
fn every_browser_builds_a_one_line_session() {
    for &browser in Browser::all() {
        let session = Session::from(browser);
        assert_eq!(session.browser(), Some(browser), "{browser}");
        assert_ne!(session.platform(), Platform::Host, "{browser}");
    }
}

#[test]
fn edge_and_opera_are_browsers() {
    assert_eq!("edge-154".parse::<Browser>().unwrap(), Browser::Edge154);
    assert_eq!(Browser::Opera139.to_string(), "Opera 139");
    assert_eq!(Browser::Opera139.version(), 139);
    assert!(Browser::all().contains(&Browser::Opera139));
    let edge = Session::from(Browser::edge());
    assert!(edge.user_agent().contains(" Edg/"), "{}", edge.user_agent());
    assert_eq!(edge.browser(), Some(Browser::edge()));
    let opera = Session::from(Browser::Opera139);
    assert!(
        opera.user_agent().contains(" OPR/139."),
        "{}",
        opera.user_agent()
    );
}

#[test]
fn browser_platform_hello_and_languages_are_session_setters() {
    let hello = Browser::Chrome154.next_hello().unwrap();
    let session = Session::builder()
        .platform(Platform::Linux)
        .browser(Browser::Chrome154)
        .hello(hello)
        .languages(["de-DE", "de"])
        .build()
        .unwrap();
    assert_eq!(session.browser(), Some(Browser::Chrome154));
    assert_eq!(session.hello(), Some(hello));
    assert_ne!(hello, Browser::Chrome154);
    assert_eq!(session.platform(), Platform::Linux);
    assert_eq!(
        session.languages(),
        Some(&["de-DE".to_owned(), "de".to_owned()][..])
    );
    let mixed = Session::builder()
        .browser(Browser::Chrome154)
        .hello(Browser::firefox())
        .build()
        .unwrap_err();
    assert_eq!(mixed.kind(), Kind::Config);
}

#[test]
fn profile_toml_and_fingerprint_strings_are_session_setters() {
    let chrome = Session::from(Browser::Chrome148);
    let toml = chrome.profile().unwrap();
    let loaded = Session::builder()
        .profile(toml)
        .platform(chrome.platform())
        .build()
        .unwrap();
    assert_eq!(loaded.profile_id(), chrome.profile_id());
    let ja3 = "771,4865-4866-4867-49195-49199-49196-49200-52393-52392-49171-49172-156-157-47,\
               0-23-65281-11-35-16-51-43-45-10-13-5-18-27-17613-51764-65037,4588-29-23-24,0";
    let mixed = Session::builder()
        .browser(Browser::Chrome148)
        .ja3(ja3)
        .build()
        .unwrap();
    assert!(mixed.profile_id().is_some());
    assert_ne!(mixed.profile_id(), chrome.profile_id());
    for replaced in [
        Session::builder()
            .profile(toml)
            .browser(Browser::Firefox148),
        mixed.to_builder().browser(Browser::Firefox148),
    ] {
        assert_eq!(
            replaced.build().unwrap().browser(),
            Some(Browser::Firefox148)
        );
    }
    for bad in [
        Session::builder().profile("not a profile"),
        Session::builder().browser(Browser::Chrome148).ja3("771,x"),
        Session::builder()
            .browser(Browser::Chrome148)
            .ja3(ja3)
            .browser(Browser::Firefox148),
    ] {
        assert_eq!(bad.build().unwrap_err().kind(), Kind::Config);
    }
    let headers = Browser::Chrome148.headers(Platform::Windows).unwrap();
    assert_eq!(headers[0].0, "user-agent");
    assert!(headers.iter().any(|(name, _)| *name == "sec-ch-ua"));
}
