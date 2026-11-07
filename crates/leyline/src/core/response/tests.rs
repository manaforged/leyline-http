use super::*;

fn bare_response(audit_tls: Option<Arc<crate::audit::AuditTlsCache>>) -> Response {
    Response::from_parts(crate::core::response::Parts {
        status: http::StatusCode::OK,
        version: http::Version::HTTP_2,
        headers: http::HeaderMap::new().into(),
        trailers: None,
        body: ResponseBody::Buffered(Vec::new()),
        url: std::sync::Arc::new(Url::parse("https://example.test/").unwrap()),
        redirect_chain: Vec::new(),
        request_headers: vec![
            (":method".to_string(), "GET".to_string()),
            ("accept-language".to_string(), "en-US,en;q=0.9".to_string()),
            ("referer".to_string(), "https://example.test/".to_string()),
        ],
        tls: None,
        request_method: Some(http::Method::GET),
        audit_tls,
        audit_cache: OnceLock::new(),
        timing: ResponseTiming::default(),
        compression: crate::core::BodyRules::default(),
        attempts: 1,
        proxy: None,
        block: None,
    })
}

fn sample_cache() -> Arc<crate::audit::AuditTlsCache> {
    let profile = crate::profile::Browser::Chrome154.profile();
    let h2 = crate::h2::H2Config::from_profile(&profile.h2).unwrap();
    Arc::new(crate::audit::AuditTlsCache::new(
        profile,
        &h2,
        &crate::profile::TcpProfile::default(),
        crate::audit::ClientHello::TcpH2,
        None,
    ))
}

#[test]
fn audit_is_none_without_tls_context() {
    let resp = bare_response(None);
    assert!(resp.fingerprint().is_none());
}

#[test]
fn audit_surfaces_cached_connection_fingerprints() {
    let cache = sample_cache();
    let resp = bare_response(Some(cache.clone()));
    let audit = resp
        .fingerprint()
        .expect("audit present when tls context set");
    assert_eq!(audit.ja4, cache.tcp().ja4);
    assert_eq!(audit.ja3, cache.tcp().ja3);
    assert_eq!(audit.h2_fingerprint, cache.tcp().h2_fingerprint);
    assert_eq!(audit.ja4t, cache.tcp().ja4t);
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
    let first = resp.fingerprint().unwrap() as *const _;
    let second = resp.fingerprint().unwrap() as *const _;
    assert_eq!(first, second, "audit() must memoise, not recompute");
}

#[cfg(feature = "charset")]
#[tokio::test]
async fn text_decodes_declared_charset() {
    let mut resp = bare_response(None);
    resp.parts.headers = [(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("text/html; charset=windows-1252"),
    )]
    .into_iter()
    .collect::<http::HeaderMap>()
    .into();
    resp.parts.body = ResponseBody::Buffered(vec![0xE9, 0xA9]);
    assert_eq!(resp.text().await.unwrap(), "é©");
}

#[cfg(feature = "charset")]
#[tokio::test]
async fn text_charset_param_is_case_insensitive_and_unquoted() {
    let mut resp = bare_response(None);
    resp.parts.headers = [(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("text/plain; Charset=\"Shift_JIS\""),
    )]
    .into_iter()
    .collect::<http::HeaderMap>()
    .into();
    resp.parts.body = ResponseBody::Buffered(vec![0x82, 0xA0]);
    assert_eq!(resp.text().await.unwrap(), "あ");
}

#[cfg(feature = "charset")]
#[tokio::test]
async fn text_defaults_to_utf8_without_charset() {
    let mut resp = bare_response(None);
    resp.parts.body = ResponseBody::Buffered("héllo".as_bytes().to_vec());
    assert_eq!(resp.text().await.unwrap(), "héllo");
}

#[cfg(feature = "charset")]
#[tokio::test]
async fn declared_charset_overrides_text_with_charset_default() {
    let mut resp = bare_response(None);
    resp.parts.headers = [(
        http::HeaderName::from_static("content-type"),
        http::HeaderValue::from_static("text/plain; charset=utf-8"),
    )]
    .into_iter()
    .collect::<http::HeaderMap>()
    .into();
    resp.parts.body = ResponseBody::Buffered("héllo".as_bytes().to_vec());
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
    resp.parts.body = ResponseBody::Taken;
    let err = resp.bytes().await.unwrap_err();
    assert_eq!(
        err.cause(),
        Kind::Body,
        "expected a body error, got {err:?}"
    );
    assert!(err.to_string().contains("bytes_stream"), "{err}");
}

#[tokio::test]
async fn text_reads_a_buffered_body_and_a_stream() {
    let mut resp = bare_response(None);
    resp.parts.body = ResponseBody::Buffered(b"hi".to_vec());
    assert_eq!(resp.text().await.unwrap(), "hi");

    let mut resp = bare_response(None);
    resp.parts.body = ResponseBody::Streaming(Box::new(BodyStream::from_bytes(
        bytes::Bytes::from_static(b"hi"),
    )));
    assert_eq!(resp.text().await.unwrap(), "hi");
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

#[test]
fn debug_shows_the_http_version_as_the_wire_name() {
    let shown = format!("{:?}", bare_response(None));
    assert!(shown.contains("version: HTTP/2.0"), "{shown}");
}

#[test]
fn content_length_reads_only_decimal_digits() {
    let mut resp = bare_response(None);
    for (value, expected) in [(" 5 ", Some(5)), ("+5", None), ("5,5", None), ("", None)] {
        resp.parts.headers = [(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_static(value),
        )]
        .into_iter()
        .collect::<http::HeaderMap>()
        .into();
        assert_eq!(resp.content_length(), expected, "{value:?}");
    }
}
