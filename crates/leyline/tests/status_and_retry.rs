#[path = "core_support/raw_server.rs"]
mod raw_server;

use std::time::Duration;

use leyline::audit::{FieldOutcome, Observed};
use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{Browser, ErrorCategory, RetryPolicy, Session, WaitFormat};
use raw_server::{RawResponse, RawServer};

fn plain() -> Session {
    Session::builder().build().unwrap()
}

#[tokio::test]
async fn error_for_status_on_the_request_keeps_body_and_headers() {
    let server = RawServer::start(vec![
        RawResponse::status(404, "Not Found")
            .header("x-request-id", "abc")
            .body(b"no such repo".to_vec()),
    ])
    .await;
    let err = plain()
        .get(server.url("/repos/x"))
        .error_for_status()
        .await
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::Status);
    assert_eq!(err.status().map(|s| s.as_u16()), Some(404));
    assert_eq!(err.body(), Some(&b"no such repo"[..]));
    let headers = err.headers().unwrap();
    assert_eq!(headers.get("x-request-id").unwrap(), "abc");
}

#[tokio::test]
async fn retry_if_and_wait_header_handle_a_rate_limit() {
    let server = RawServer::start(vec![
        RawResponse::status(403, "Forbidden")
            .header("x-ratelimit-remaining", "0")
            .header("x-wait", "0"),
        RawResponse::ok(),
    ])
    .await;
    let session = Session::builder()
        .retry(
            RetryPolicy::transient()
                .initial_backoff(Duration::from_secs(30))
                .retry_if(|resp| {
                    resp.status().as_u16() == 403
                        && resp.header("x-ratelimit-remaining") == Some("0")
                })
                .wait_header("x-wait", WaitFormat::Seconds),
        )
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let resp = session.get(server.url("/limited")).await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.attempts(), 2);
}

#[tokio::test]
async fn a_response_names_the_proxy_that_carried_it() {
    let proxy = RawServer::start(vec![RawResponse::ok()]).await;
    let resp = Session::builder()
        .proxy(proxy.url(""))
        .build()
        .unwrap()
        .get("http://origin.test/")
        .await
        .unwrap();
    assert_eq!(resp.proxy(), Some(proxy.url("").as_str()));
    assert_eq!(plain_direct_proxy().await, None);
}

async fn plain_direct_proxy() -> Option<String> {
    let server = RawServer::start(vec![RawResponse::ok()]).await;
    plain()
        .get(server.url("/"))
        .await
        .unwrap()
        .proxy()
        .map(str::to_owned)
}

#[tokio::test]
async fn audit_compares_with_an_echo_report() {
    let server = TestServer::https(queue([TestResponse::new(200)]))
        .await
        .unwrap();
    let resp = Session::builder()
        .browser(Browser::default())
        .tls_trust(server.trust())
        .audit(true)
        .build()
        .unwrap()
        .get(server.url("/"))
        .await
        .unwrap();
    let audit = resp.audit().unwrap();

    let same = Observed::new()
        .ja4(audit.ja4.clone())
        .h2_fingerprint(audit.h2_fingerprint.clone());
    let report = audit.compare(&same);
    assert!(report.is_match());
    assert_eq!(report.ja4, FieldOutcome::Match);
    assert_eq!(report.ja3, FieldOutcome::NotReported);
    let text = report.to_string();
    assert!(
        text.contains("ja4: match") && text.contains("ja3: not reported"),
        "{text}"
    );

    let shuffled = Observed::new()
        .ja4(audit.ja4.clone())
        .ja3("0123456789abcdef");
    let report = audit.compare(&shuffled);
    assert!(report.is_match());
    assert!(matches!(report.ja3, FieldOutcome::Informational { .. }));

    let other = Observed::new().ja4("t13d0000h2_000000000000_000000000000");
    let report = audit.compare(&other);
    assert!(!report.is_match());
    assert!(report.ja4.is_mismatch());

    let agent = audit
        .request_headers
        .iter()
        .find(|(name, _)| name == "user-agent")
        .map(|(_, value)| value.clone())
        .unwrap();
    let echoed = Observed::from_json(&format!(
        r#"{{"http1":{{"headers":["User-Agent: {agent}","Accept-Language: xx"]}}}}"#
    ))
    .unwrap();
    let report = audit.compare(&echoed);
    let outcome = |name: &str| {
        report
            .headers
            .iter()
            .find(|h| h.name == name)
            .map(|h| h.outcome.clone())
    };
    assert_eq!(outcome("user-agent"), Some(FieldOutcome::Match));
    assert!(outcome("accept-language").unwrap().is_mismatch());
    assert!(!report.is_match());

    let parsed = Observed::from_json(&format!(
        r#"{{"tls":{{"ja4":"{}","ja3_hash":"x"}},"http2":{{"akamai_fingerprint":"{}"}}}}"#,
        audit.ja4, audit.h2_fingerprint
    ))
    .unwrap();
    assert_eq!(audit.compare(&parsed).ja4, FieldOutcome::Match);
}

#[test]
fn urls_serialize_for_saved_state() {
    let url: leyline::Url = "https://shop.example/cart".parse().unwrap();
    let json = serde_json::to_string(&url).unwrap();
    assert_eq!(serde_json::from_str::<leyline::Url>(&json).unwrap(), url);
}

#[test]
fn retry_policy_stays_unwind_safe() {
    fn unwind_safe<T: std::panic::UnwindSafe + std::panic::RefUnwindSafe>(_: &T) {}
    let policy = RetryPolicy::transient().retry_if(|resp| resp.status().as_u16() == 403);
    unwind_safe(&policy);
}

#[tokio::test]
async fn a_status_error_reports_its_header_attempts_and_proxy() {
    let proxy = RawServer::start(vec![
        RawResponse::status(503, "Service Unavailable"),
        RawResponse::status(503, "Service Unavailable").header("retry-at", "later"),
    ])
    .await;
    let session = Session::builder()
        .proxy(proxy.url(""))
        .retry(
            RetryPolicy::none()
                .max_retries(1)
                .on_status(503)
                .initial_backoff(Duration::from_millis(1)),
        )
        .build()
        .unwrap();
    let err = session
        .get("http://origin.test/")
        .error_for_status()
        .await
        .unwrap_err();
    assert_eq!(err.header("retry-at"), Some("later"));
    assert_eq!(err.attempts(), 2);
    assert_eq!(err.proxy(), Some(proxy.url("").as_str()));
    assert_eq!(err.category().as_str(), "status");
}

#[tokio::test]
async fn the_session_body_cap_turns_a_large_body_into_a_body_limit_error() {
    let server = RawServer::start(vec![RawResponse::ok().body(vec![b'x'; 4096])]).await;
    let err = Session::builder()
        .max_body_size(1024)
        .build()
        .unwrap()
        .get(server.url("/big"))
        .await
        .unwrap_err();
    assert_eq!(err.category(), ErrorCategory::BodyLimit);
    assert_eq!(err.category().as_str(), "body_limit");
}

#[test]
fn relay_headers_work_on_any_header_map() {
    let mut headers = leyline::http::HeaderMap::new();
    headers.insert("connection", "close".parse().unwrap());
    headers.insert("content-encoding", "gzip".parse().unwrap());
    headers.insert("content-length", "10".parse().unwrap());
    headers.insert("x-keep", "1".parse().unwrap());
    let decoded = leyline::relay_headers(&headers, leyline::RelayBody::Decoded);
    assert_eq!(decoded.len(), 1);
    assert!(decoded.contains_key("x-keep"));
    let raw = leyline::relay_headers(&headers, leyline::RelayBody::AsReceived);
    assert_eq!(raw.len(), 3);
}

#[test]
fn a_mobile_only_browser_builds_on_its_own_platform() {
    let session = Session::builder()
        .browser(Browser::SafariIOS27)
        .build()
        .unwrap();
    assert_eq!(session.identity().platform(), leyline::Platform::IOS);
    let chrome = Session::builder()
        .browser(Browser::default())
        .build()
        .unwrap();
    assert_eq!(chrome.identity().platform(), leyline::Platform::Windows);
}
