#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
#![expect(
    clippy::panic,
    reason = "test harness helper: explicit panic on unexpected error shape is the assertion"
)]
use leyline::Session;
use serde_json::Value;

#[path = "http_support/httpbin_lite.rs"]
mod httpbin_lite;

#[tokio::test]
async fn decompression_gzip() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let resp = session.get(format!("{base}/gzip")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value =
        serde_json::from_str(&resp.text().await.unwrap()).expect("gzip-decoded body not JSON");
    assert_eq!(json["gzipped"], true);
}

#[tokio::test]
async fn decompression_brotli() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let resp = session.get(format!("{base}/brotli")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value =
        serde_json::from_str(&resp.text().await.unwrap()).expect("brotli-decoded body not JSON");
    assert_eq!(json["brotli"], true);
}

#[tokio::test]
async fn decompression_deflate() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let resp = session.get(format!("{base}/deflate")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value =
        serde_json::from_str(&resp.text().await.unwrap()).expect("deflate-decoded body not JSON");
    assert_eq!(json["deflated"], true);
}

#[tokio::test]
async fn cookies_set_then_sent() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let resp1 = session
        .get(format!("{base}/cookies/set?token=abc123"))
        .await
        .unwrap();
    assert_eq!(resp1.status(), 200);

    let resp2 = session.get(format!("{base}/cookies")).await.unwrap();
    assert_eq!(resp2.status(), 200);
    let json: Value = serde_json::from_str(&resp2.text().await.unwrap()).unwrap();
    assert_eq!(
        json["cookies"]["token"].as_str(),
        Some("abc123"),
        "cookie not sent on second request"
    );
}

#[tokio::test]
async fn response_cookies_use_the_rfc_parser_not_a_hand_parser() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let resp = session
        .get(format!("{base}/set-cookie-quoted"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.cookies()
            .find(|c| c.name == "token")
            .map(|c| c.value)
            .as_deref(),
        Some("quoted value"),
        "Response::cookies() must reflect the RFC parser (quotes stripped)"
    );
    assert_eq!(
        session
            .cookies()
            .get_cookie(url::Url::parse(&format!("{base}/")).unwrap(), "token")
            .unwrap()
            .as_deref(),
        Some("quoted value"),
        "Response::cookies() and the jar must not diverge"
    );
}

#[tokio::test]
async fn redirect_follows_and_rewrites_url() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let resp = session.get(format!("{base}/redirect/3")).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.redirect_chain().len(),
        3,
        "expected 3 redirects in chain"
    );
    assert!(
        resp.url().ends_with("/get"),
        "final URL wrong: {}",
        resp.url()
    );
}

#[tokio::test]
async fn redirect_preserves_auth_same_host() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let resp = session
        .request(
            http::Method::GET,
            format!("{base}/redirect-to?url={base}/headers"),
        )
        .bearer_auth("secret-token-xyz")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert_eq!(
        json["headers"]["Authorization"].as_str(),
        Some("Bearer secret-token-xyz"),
        "same-host redirect should preserve Authorization"
    );
}

#[tokio::test]
async fn redirect_307_308_replays_buffered_body() {
    for status in [307u16, 308] {
        let base = httpbin_lite::spawn().await;
        let session = Session::new();
        let payload = "replay-me-please-i-am-a-request-body";
        let resp = session
            .post(format!("{base}/redirect-to?url=/post&status_code={status}"))
            .body(payload.as_bytes().to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            200,
            "{status}: should follow through to /post"
        );
        assert_eq!(
            resp.redirect_chain().len(),
            1,
            "{status}: exactly one redirect"
        );
        let json: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
        assert_eq!(
            json["data"], payload,
            "{status} redirect dropped the request body"
        );
    }
}

#[tokio::test]
async fn redirect_to_non_http_scheme_is_refused() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let result = session
        .get(format!("{base}/redirect-to?url=file:///etc/passwd"))
        .await;
    assert!(result.is_err(), "a file:// redirect target must be refused");
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("non-http(s)") || msg.to_lowercase().contains("scheme"),
        "expected a redirect-scheme refusal, got: {msg}"
    );
}

#[tokio::test]
async fn post_json_body_roundtrip() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let body = serde_json::json!({"test": "leyline", "n": 42});
    let resp = session
        .post(format!("{base}/post"))
        .json(&body)
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert_eq!(json["json"]["test"], "leyline");
    assert_eq!(json["json"]["n"], 42);
}

#[tokio::test]
async fn post_form_body_roundtrip() {
    let base = httpbin_lite::spawn().await;
    let session = Session::new();
    let resp = session
        .post(format!("{base}/post"))
        .form([("u", "alice"), ("p", "s3cret")])
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text().await.unwrap()).unwrap();
    assert_eq!(json["form"]["u"], "alice");
    assert_eq!(json["form"]["p"], "s3cret");
}
