use std::time::Duration;

use leyline::audit::{FieldOutcome, Observed};
use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{Browser, ErrorCategory, RetryPolicy, Session, WaitFormat};

fn plain() -> Session {
    Session::builder().build().unwrap()
}

#[tokio::test]
async fn error_for_status_on_the_request_keeps_body_and_headers() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(404)
            .close()
            .header("x-request-id", "abc")
            .body(b"no such repo".to_vec()),
    ]))
    .await
    .unwrap();
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
    let server = TestServer::http(queue(vec![
        TestResponse::new(403)
            .close()
            .header("x-ratelimit-remaining", "0")
            .header("x-wait", "0"),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
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
    let targets: Vec<_> = server
        .requests()
        .await
        .into_iter()
        .map(|r| r.target)
        .collect();
    assert_eq!(targets, ["/limited", "/limited"]);
}

#[tokio::test]
async fn a_response_names_the_proxy_that_carried_it() {
    let proxy = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let resp = Session::builder()
        .proxy(format!("http://{}", proxy.addr()))
        .build()
        .unwrap()
        .get("http://origin.test/")
        .await
        .unwrap();
    assert_eq!(
        resp.proxy(),
        Some(format!("http://{}", proxy.addr()).as_str())
    );
    assert_eq!(plain_direct_proxy().await, None);
}

async fn plain_direct_proxy() -> Option<String> {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
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

#[tokio::test]
async fn a_status_error_reports_its_header_attempts_and_proxy() {
    let proxy = TestServer::http(queue(vec![
        TestResponse::new(503).close(),
        TestResponse::new(503).close().header("retry-at", "later"),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .proxy(format!("http://{}", proxy.addr()))
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
    assert_eq!(
        err.proxy(),
        Some(format!("http://{}", proxy.addr()).as_str())
    );
    assert_eq!(err.category().as_str(), "status");
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
