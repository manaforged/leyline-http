#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::sync::{Arc, Mutex};

use leyline::trace::{Event, Trace};
use leyline::{Redirect, Session};

use crate::http_support::httpbin_lite;

const SECRET: &str = "s3cret-value";

#[test]
fn digest_auth_debug_hides_the_password() {
    let request = Session::from(leyline::Browser::default())
        .get("https://example.com/")
        .digest_auth("alice", SECRET);
    let shown = format!("{request:?}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn session_builder_debug_hides_the_proxy_password() {
    let builder = Session::builder().proxy(format!("http://user:{SECRET}@127.0.0.1:9000"));
    let shown = format!("{builder:?}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn request_builder_debug_hides_credentials() {
    let session = Session::from(leyline::Browser::default());
    let request = session
        .get("https://example.com/")
        .bearer_auth(SECRET)
        .header("cookie", format!("sid={SECRET}"));
    let shown = format!("{request:?}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn proxy_url_shows_the_exit_without_its_password() {
    let raw = format!("http://user:{SECRET}@127.0.0.1:9000");
    let bound = Session::from(leyline::Browser::default())
        .to_builder()
        .proxy(raw.as_str())
        .build()
        .unwrap();
    let shown = bound.proxy().unwrap();
    assert!(shown.contains("127.0.0.1:9000"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
    let rebound = bound.to_builder().build().unwrap();
    assert_eq!(rebound.proxy(), Some(shown));
}

#[tokio::test]
async fn response_debug_hides_the_credentials_it_sent() {
    let base = httpbin_lite::spawn().await;
    let session = Session::builder().audit(true).build().unwrap();
    let resp = session
        .get(format!("{base}/get"))
        .bearer_auth(SECRET)
        .header("cookie", format!("sid={SECRET}"))
        .send()
        .await
        .unwrap();
    let shown = format!("{resp:?}");
    assert!(shown.contains("200"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[tokio::test]
async fn fingerprint_debug_hides_the_credentials_it_sent() {
    use leyline::testing::{self, queue};
    let server = testing::Server::https(queue([testing::Response::new(200)]))
        .await
        .unwrap();
    let resp = Session::builder()
        .browser(leyline::Browser::default())
        .tls(server.tls())
        .audit(true)
        .build()
        .unwrap()
        .get(server.url("/"))
        .bearer_auth(SECRET)
        .header("cookie", format!("sid={SECRET}"))
        .send()
        .await
        .unwrap();
    let shown = format!("{:?}", resp.fingerprint().unwrap());
    assert!(shown.contains("authorization"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn cookie_debug_hides_the_value() {
    let jar = leyline::cookie::Jar::new();
    let url = url::Url::parse("https://example.com/").unwrap();
    jar.store(&format!("sid={SECRET}; Path=/"), &url);
    let shown = format!("{:?}", jar.all());
    assert!(shown.contains("sid"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[tokio::test]
async fn response_debug_hides_a_received_set_cookie() {
    let base = httpbin_lite::spawn().await;
    let session = Session::builder()
        .redirect(Redirect::none())
        .build()
        .unwrap();
    let resp = session
        .get(format!("{base}/cookies/set?sid={SECRET}"))
        .send()
        .await
        .unwrap();
    let shown = format!("{resp:?}");
    assert!(shown.contains("302"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

struct Capture(Arc<Mutex<Vec<String>>>);

impl Trace for Capture {
    fn event(&self, event: &Event<'_>) {
        if let Event::Sent { .. } = event {
            self.0.lock().unwrap().push(format!("{event:?}"));
        }
    }
}

#[tokio::test]
async fn sent_event_debug_hides_the_query() {
    let base = httpbin_lite::spawn().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let session = Session::builder()
        .trace(Capture(Arc::clone(&seen)))
        .build()
        .unwrap();
    session
        .get(format!("{base}/get?token={SECRET}"))
        .send()
        .await
        .unwrap();
    let shown = seen.lock().unwrap().join("\n");
    assert!(shown.contains("/get"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn a_proxy_url_without_a_scheme_hides_its_password() {
    let refused = Session::from(leyline::Browser::default())
        .to_builder()
        .proxy(format!("user:{SECRET}@127.0.0.1:9000"))
        .build()
        .unwrap_err();
    let shown = format!("{refused} {refused:?}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn request_builder_debug_hides_query_values() {
    let session = Session::from(leyline::Browser::default());
    let request = session
        .get(format!("https://example.com/?token={SECRET}"))
        .query([("key", SECRET)]);
    let shown = format!("{request:?}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[tokio::test]
async fn redirect_attempt_debug_hides_the_location_query() {
    let base = httpbin_lite::spawn().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let session = Session::builder()
        .redirect(Redirect::custom(move |attempt| {
            sink.lock().unwrap().push(format!("{attempt:?}"));
            false
        }))
        .build()
        .unwrap();
    session
        .get(format!("{base}/redirect-to?url=/get?token={SECRET}"))
        .send()
        .await
        .unwrap();
    let shown = seen.lock().unwrap().join("\n");
    assert!(shown.contains("302"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}
