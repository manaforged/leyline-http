#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::sync::{Arc, Mutex};

use leyline::trace::{Sent, Trace};
use leyline::{DigestAuth, HeaderList, RedirectPolicy, Session};

#[path = "http_support/httpbin_lite.rs"]
mod httpbin_lite;

const SECRET: &str = "s3cret-value";

#[test]
fn digest_auth_debug_hides_the_password() {
    let shown = format!("{:?}", DigestAuth::new("alice", SECRET));
    assert!(shown.contains("alice"), "{shown}");
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
    let session = Session::new();
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
    let bound = Session::new().with_proxy(raw.as_str());
    let exit = bound.proxy_url().unwrap();
    let shown = format!("{exit} {exit:?}");
    assert!(shown.contains("127.0.0.1:9000"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
    let rebound = Session::new().with_proxy(exit);
    assert_eq!(rebound.proxy_url().map(String::from), Some(raw));
}

#[tokio::test]
async fn response_debug_hides_the_credentials_it_sent() {
    let base = httpbin_lite::spawn().await;
    let session = Session::builder().audit(true).build().unwrap();
    let resp = session
        .get(format!("{base}/get"))
        .bearer_auth(SECRET)
        .header("cookie", format!("sid={SECRET}"))
        .await
        .unwrap();
    let shown = format!("{resp:?}");
    assert!(shown.contains("200"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn header_list_debug_hides_credentials() {
    let mut headers = HeaderList::new();
    headers
        .append("authorization", format!("Bearer {SECRET}"))
        .unwrap();
    headers.append("x-trace", "1").unwrap();
    let shown = format!("{headers:?}");
    assert!(shown.contains("x-trace"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[test]
fn cookie_debug_hides_the_value() {
    let jar = leyline::cookie::Jar::new();
    let url = url::Url::parse("https://example.com/").unwrap();
    jar.store_set_cookie(&format!("sid={SECRET}; Path=/"), &url);
    let shown = format!("{:?}", jar.all_cookies());
    assert!(shown.contains("sid"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

#[tokio::test]
async fn response_debug_hides_a_received_set_cookie() {
    let base = httpbin_lite::spawn().await;
    let session = Session::builder()
        .redirect(RedirectPolicy::none())
        .build()
        .unwrap();
    let resp = session
        .get(format!("{base}/cookies/set?sid={SECRET}"))
        .await
        .unwrap();
    let shown = format!("{resp:?}");
    assert!(shown.contains("302"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}

struct Capture(Arc<Mutex<Vec<String>>>);

impl Trace for Capture {
    fn sent(&self, ev: &Sent<'_>) {
        self.0.lock().unwrap().push(format!("{ev:?}"));
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
        .await
        .unwrap();
    let shown = seen.lock().unwrap().join("\n");
    assert!(shown.contains("/get"), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");
}
