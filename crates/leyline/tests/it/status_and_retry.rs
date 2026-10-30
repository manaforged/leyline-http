use std::time::Duration;

use leyline::testing::{self, queue};
use leyline::{Browser, Fingerprint, Kind, Retry, Session};

fn plain() -> Session {
    Session::builder().build().unwrap()
}

#[tokio::test]
async fn error_for_status_on_the_request_keeps_body_and_headers() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(404)
            .close()
            .header("x-request-id", "abc")
            .body(b"no such repo".to_vec()),
    ]))
    .await
    .unwrap();
    let err = plain()
        .get(server.url("/repos/x"))
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(err.kind(), Kind::Status);
    assert_eq!(err.status().map(|s| s.as_u16()), Some(404));
    assert_eq!(err.body(), Some(&b"no such repo"[..]));
    let headers = err.headers().unwrap();
    assert_eq!(headers.get("x-request-id").unwrap(), "abc");
}

#[tokio::test]
async fn retry_if_and_wait_header_handle_a_rate_limit() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(403)
            .close()
            .header("x-ratelimit-remaining", "0")
            .header("x-wait", "0"),
        testing::Response::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .retry(
            Retry::new(3)
                .backoff(Duration::from_secs(30), Duration::from_secs(30))
                .wait_header("x-wait")
                .when(|attempt| {
                    attempt.is_transient()
                        || attempt.response().is_some_and(|resp| {
                            resp.status().as_u16() == 403
                                && resp.header("x-ratelimit-remaining") == Some("0")
                        })
                }),
        )
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let resp = session.get(server.url("/limited")).send().await.unwrap();
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
    let proxy = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    let resp = Session::builder()
        .proxy(format!("http://{}", proxy.addr()))
        .build()
        .unwrap()
        .get("http://origin.test/")
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.proxy(),
        Some(format!("http://{}", proxy.addr()).as_str())
    );
    assert_eq!(plain_direct_proxy().await, None);
}

async fn plain_direct_proxy() -> Option<String> {
    let server = testing::Server::http(queue(vec![testing::Response::new(200).body("ok").close()]))
        .await
        .unwrap();
    plain()
        .get(server.url("/"))
        .send()
        .await
        .unwrap()
        .proxy()
        .map(str::to_owned)
}

#[tokio::test]
async fn audit_compares_with_an_echo_report() {
    let server = testing::Server::https(queue([testing::Response::new(200)]))
        .await
        .unwrap();
    let resp = Session::builder()
        .browser(Browser::default())
        .tls(server.tls())
        .audit(true)
        .build()
        .unwrap()
        .get(server.url("/"))
        .send()
        .await
        .unwrap();
    let sent = resp.fingerprint().unwrap();
    let fields = |diff: Vec<(String, String, String)>| -> Vec<String> {
        diff.into_iter().map(|(field, _, _)| field).collect()
    };

    let same = Fingerprint::from_json(&format!(
        r#"{{"tls":{{"ja4":"{}"}},"http2":{{"akamai_fingerprint":"{}"}}}}"#,
        sent.ja4(),
        sent.akamai()
    ))
    .unwrap();
    assert!(sent.diff(&same).is_empty());

    let shuffled = Fingerprint::from_json(&format!(
        r#"{{"tls":{{"ja4":"{}","ja3_hash":"0123456789abcdef"}}}}"#,
        sent.ja4()
    ))
    .unwrap();
    assert!(
        sent.diff(&shuffled).is_empty(),
        "ja3 order varies by design"
    );

    let other = Fingerprint::from_json(r#"{"tls":{"ja4":"t13d0000h2_000000000000_000000000000"}}"#)
        .unwrap();
    assert_eq!(fields(sent.diff(&other)), ["ja4"]);

    let agent = sent
        .headers()
        .iter()
        .find(|(name, _)| name == "user-agent")
        .map(|(_, value)| value.clone())
        .unwrap();
    let echoed = Fingerprint::from_json(&format!(
        r#"{{"http1":{{"headers":["User-Agent: {agent}","Accept-Language: xx"]}}}}"#
    ))
    .unwrap();
    let diff = fields(sent.diff(&echoed));
    assert!(diff.contains(&"accept-language".to_owned()), "{diff:?}");
    assert!(!diff.contains(&"user-agent".to_owned()), "{diff:?}");
}

#[tokio::test]
async fn a_status_error_reports_its_header_attempts_and_proxy() {
    let proxy = testing::Server::http(queue(vec![
        testing::Response::new(503).close(),
        testing::Response::new(503)
            .close()
            .header("retry-at", "later"),
    ]))
    .await
    .unwrap();
    let session = Session::builder()
        .proxy(format!("http://{}", proxy.addr()))
        .retry(
            Retry::new(1)
                .backoff(Duration::from_millis(1), Duration::from_secs(1))
                .when(|attempt| attempt.status().is_some_and(|s| s.as_u16() == 503)),
        )
        .build()
        .unwrap();
    let err = session
        .get("http://origin.test/")
        .error_for_status()
        .send()
        .await
        .unwrap_err();
    assert_eq!(
        err.headers()
            .and_then(|h| h.get("retry-at"))
            .and_then(|v| v.to_str().ok()),
        Some("later")
    );
    assert_eq!(err.attempts(), 2);
    assert_eq!(
        err.proxy(),
        Some(format!("http://{}", proxy.addr()).as_str())
    );
    assert_eq!(err.kind().as_str(), "status");
}

#[test]
fn relay_headers_work_on_any_header_map() {
    let mut headers = leyline::http::HeaderMap::new();
    headers.insert("connection", "close".parse().unwrap());
    headers.insert("content-encoding", "gzip".parse().unwrap());
    headers.insert("content-length", "10".parse().unwrap());
    headers.insert("x-keep", "1".parse().unwrap());
    let raw = leyline::relay_headers(&headers);
    assert_eq!(raw.len(), 3);
    assert!(!raw.contains_key("connection"));
}

#[test]
fn a_mobile_only_browser_builds_on_its_own_platform() {
    let session = Session::builder()
        .browser(Browser::SafariIOS27)
        .build()
        .unwrap();
    assert_eq!(session.platform(), leyline::Platform::IOS);
    let chrome = Session::builder()
        .browser(Browser::default())
        .build()
        .unwrap();
    assert_eq!(chrome.platform(), leyline::Platform::Windows);
}
