use super::*;

fn bare_response(audit_tls: Option<Arc<crate::audit::AuditTlsCache>>) -> Response {
    Response {
        status: http::StatusCode::OK,
        version: HttpVersion::Http2,
        headers: Vec::new(),
        trailers: Vec::new(),
        body: ResponseBody::Buffered(Vec::new()),
        cookies: HashMap::new(),
        url: "https://example.test/".to_string(),
        redirect_chain: Vec::new(),
        request_headers: vec![
            (":method".to_string(), "GET".to_string()),
            ("accept-language".to_string(), "en-US,en;q=0.9".to_string()),
            ("referer".to_string(), "https://example.test/".to_string()),
        ],
        tls_alpn: None,
        tls_peer_certificate: None,
        tls_version: None,
        tls_cipher: None,
        request_method: "GET".to_string(),
        audit_tls,
        audit_cache: OnceLock::new(),
        timing: ResponseTiming::default(),
        compression: crate::core::CompressionConfig::default(),
    }
}

fn sample_cache() -> Arc<crate::audit::AuditTlsCache> {
    Arc::new(crate::audit::AuditTlsCache {
        ja4: "t13d1516h2_8daaf6152771_d8a2da3f94cd".to_string(),
        ja3: "771,4865-4866,0-23,29-23,0".to_string(),
        h2_fingerprint: "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p".to_string(),
        ja4t: "64240_2-1-3-1-1-4_1460_8".to_string(),
    })
}

#[test]
fn audit_is_none_without_tls_context() {
    let resp = bare_response(None);
    assert!(resp.audit().is_none());
}

#[test]
fn audit_surfaces_cached_connection_fingerprints() {
    let cache = sample_cache();
    let resp = bare_response(Some(cache.clone()));
    let audit = resp.audit().expect("audit present when tls context set");
    assert_eq!(audit.ja4, cache.ja4);
    assert_eq!(audit.ja3, cache.ja3);
    assert_eq!(audit.h2_fingerprint, cache.h2_fingerprint);
    assert_eq!(audit.ja4t, cache.ja4t);
    assert_eq!(
        audit.ja4h.split('_').count(),
        4,
        "JA4H shape: {}",
        audit.ja4h
    );
}

#[test]
fn audit_memoises_across_calls() {
    let resp = bare_response(Some(sample_cache()));
    let first = resp.audit().unwrap() as *const _;
    let second = resp.audit().unwrap() as *const _;
    assert_eq!(first, second, "audit() must memoise, not recompute");
}

#[cfg(feature = "charset")]
#[tokio::test]
async fn text_decodes_declared_charset() {
    let mut resp = bare_response(None);
    resp.headers = vec![(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("text/html; charset=windows-1252"),
    )];
    resp.body = ResponseBody::Buffered(vec![0xE9, 0xA9]);
    assert_eq!(resp.text().await.unwrap(), "é©");
    assert_eq!(resp.into_text().await.unwrap(), "é©");
}

#[cfg(feature = "charset")]
#[tokio::test]
async fn text_charset_param_is_case_insensitive_and_unquoted() {
    let mut resp = bare_response(None);
    resp.headers = vec![(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("text/plain; Charset=\"Shift_JIS\""),
    )];
    resp.body = ResponseBody::Buffered(vec![0x82, 0xA0]);
    assert_eq!(resp.text().await.unwrap(), "あ");
}

#[cfg(feature = "charset")]
#[tokio::test]
async fn text_defaults_to_utf8_without_charset() {
    let mut resp = bare_response(None);
    resp.body = ResponseBody::Buffered("héllo".as_bytes().to_vec());
    assert_eq!(resp.text().await.unwrap(), "héllo");
}

#[cfg(feature = "charset")]
#[tokio::test]
async fn declared_charset_overrides_text_with_charset_default() {
    let mut resp = bare_response(None);
    resp.headers = vec![(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("text/plain; charset=utf-8"),
    )];
    resp.body = ResponseBody::Buffered("héllo".as_bytes().to_vec());
    assert_eq!(
        resp.text_with_charset("windows-1252").await.unwrap(),
        "héllo"
    );
}

fn leg(reused: bool, connect_ms: Option<u32>, send_ms: u32, total_ms: u32) -> ResponseTiming {
    ResponseTiming {
        reused,
        connect_ms,
        send_ms,
        total_ms,
    }
}

#[test]
fn timing_single_leg_equals_that_leg() {
    let mut acc = ResponseTiming::accumulator();
    let only = leg(true, None, 12, 15);
    acc.add_leg(&only);
    assert_eq!(acc, only);
}

#[test]
fn timing_cold_then_warm_redirect_sums_and_marks_not_reused() {
    let mut acc = ResponseTiming::accumulator();
    acc.add_leg(&leg(false, Some(40), 60, 105));
    acc.add_leg(&leg(true, None, 800, 800));
    assert!(!acc.reused);
    assert_eq!(acc.connect_ms, Some(40));
    assert_eq!(acc.send_ms, 860);
    assert_eq!(acc.total_ms, 905);
}

#[test]
fn timing_all_warm_stays_reused_with_no_connect() {
    let mut acc = ResponseTiming::accumulator();
    acc.add_leg(&leg(true, None, 5, 6));
    acc.add_leg(&leg(true, None, 7, 8));
    assert!(acc.reused);
    assert_eq!(acc.connect_ms, None);
    assert_eq!(acc.total_ms, 14);
}

#[test]
fn timing_two_cold_legs_sum_connect() {
    let mut acc = ResponseTiming::accumulator();
    acc.add_leg(&leg(false, Some(30), 10, 45));
    acc.add_leg(&leg(false, Some(25), 12, 40));
    assert_eq!(acc.connect_ms, Some(55));
    assert!(!acc.reused);
}

#[tokio::test]
async fn bytes_on_a_taken_stream_reports_a_body_error() {
    let mut resp = bare_response(None);
    resp.body = ResponseBody::Taken;
    let err = resp.bytes().await.unwrap_err();
    assert_eq!(err.kind(), Kind::Body, "expected a body error, got {err:?}");
    assert!(err.to_string().contains("into_stream"), "{err}");
}

#[tokio::test]
async fn as_bytes_sees_a_buffered_body_and_skips_a_stream() {
    let mut resp = bare_response(None);
    resp.body = ResponseBody::Buffered(b"hi".to_vec());
    assert_eq!(resp.as_bytes(), Some(&b"hi"[..]));
    assert_eq!(resp.as_text().expect("buffered").unwrap(), "hi");

    resp.body = ResponseBody::Streaming(BodyStream::from_bytes(bytes::Bytes::from_static(b"hi")));
    assert!(resp.as_bytes().is_none());
    assert_eq!(resp.text().await.unwrap(), "hi");
}

#[test]
fn error_for_status_caps_retained_body() {
    let mut resp = bare_response(None);
    resp.status = http::StatusCode::INTERNAL_SERVER_ERROR;
    resp.body = ResponseBody::Buffered(vec![b'x'; 2 * 1024 * 1024]);
    let err = resp.error_for_status().unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::Status,
        "expected a status error, got {err:?}"
    );
    assert_eq!(err.status().map(|s| s.as_u16()), Some(500));
    let body = err.body_prefix().expect("status errors keep a body prefix");
    assert_eq!(
        body.len(),
        16 * 1024,
        "status body must be capped at 16 KiB"
    );
    assert!(body.iter().all(|&b| b == b'x'), "prefix content preserved");
}

#[test]
fn error_for_status_keeps_short_body_intact() {
    let mut resp = bare_response(None);
    resp.status = http::StatusCode::NOT_FOUND;
    resp.body = ResponseBody::Buffered(b"not found".to_vec());
    let err = resp.error_for_status().unwrap_err();
    assert_eq!(
        err.kind(),
        Kind::Status,
        "expected a status error, got {err:?}"
    );
    assert_eq!(err.body_prefix(), Some(&b"not found"[..]));
}

#[test]
fn timing_add_leg_saturates_not_wraps() {
    let mut acc = ResponseTiming::accumulator();
    acc.add_leg(&leg(false, Some(u32::MAX), u32::MAX, u32::MAX));
    acc.add_leg(&leg(false, Some(10), 10, 10));
    assert_eq!(acc.total_ms, u32::MAX);
    assert_eq!(acc.send_ms, u32::MAX);
    assert_eq!(acc.connect_ms, Some(u32::MAX));
}
