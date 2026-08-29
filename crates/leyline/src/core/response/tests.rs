use super::*;

fn bare_response(audit_tls: Option<Arc<crate::audit::AuditTlsCache>>) -> Response {
    Response {
        status: 200,
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
    // JA4H is request-derived, so it must be non-empty and shaped a_b_c_d.
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
    // Same allocation on the second call — JA4H is hashed once, not per call.
    assert_eq!(first, second, "audit() must memoise, not recompute");
}

#[cfg(feature = "charset")]
#[test]
fn text_decodes_declared_charset() {
    let mut resp = bare_response(None);
    resp.headers = vec![(
        crate::core::HeaderStr::from_static("content-type"),
        crate::core::HeaderStr::from_static("text/html; charset=windows-1252"),
    )];
    // windows-1252: 0xE9 -> 'é', 0xA9 -> '©'. As raw UTF-8 these bytes are
    // invalid and would become U+FFFD without charset handling.
    resp.body = ResponseBody::Buffered(vec![0xE9, 0xA9]);
    assert_eq!(resp.text(), "é©");
    assert_eq!(resp.into_text(), "é©");
}

#[cfg(feature = "charset")]
#[test]
fn text_charset_param_is_case_insensitive_and_unquoted() {
    let mut resp = bare_response(None);
    resp.headers = vec![(
        crate::core::HeaderStr::from_static("content-type"),
        crate::core::HeaderStr::from_static("text/plain; Charset=\"Shift_JIS\""),
    )];
    // Shift_JIS 0x82 0xA0 -> 'あ' (U+3042).
    resp.body = ResponseBody::Buffered(vec![0x82, 0xA0]);
    assert_eq!(resp.text(), "あ");
}

#[cfg(feature = "charset")]
#[test]
fn text_defaults_to_utf8_without_charset() {
    let mut resp = bare_response(None);
    resp.body = ResponseBody::Buffered("héllo".as_bytes().to_vec());
    // No declared charset -> text() uses UTF-8.
    assert_eq!(resp.text(), "héllo");
}

#[cfg(feature = "charset")]
#[test]
fn declared_charset_overrides_text_with_charset_default() {
    let mut resp = bare_response(None);
    resp.headers = vec![(
        crate::core::HeaderStr::from_static("content-type"),
        crate::core::HeaderStr::from_static("text/plain; charset=utf-8"),
    )];
    resp.body = ResponseBody::Buffered("héllo".as_bytes().to_vec());
    // The declared utf-8 wins over the windows-1252 caller default.
    assert_eq!(resp.text_with_charset("windows-1252"), "héllo");
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
    // One transport hop (no redirect): the accumulated timing is exactly
    // that leg — the accumulator seed must be the identity.
    let mut acc = ResponseTiming::accumulator();
    let only = leg(true, None, 12, 15);
    acc.add_leg(&only);
    assert_eq!(acc, only);
}

#[test]
fn timing_cold_then_warm_redirect_sums_and_marks_not_reused() {
    // 302 on a fresh connection (cold) → 200 reused on the pooled conn.
    // total/send sum; connect_ms is the cold leg's handshake; one fresh
    // connect anywhere means the whole request was NOT all-reused.
    let mut acc = ResponseTiming::accumulator();
    acc.add_leg(&leg(false, Some(40), 60, 105)); // cold 302
    acc.add_leg(&leg(true, None, 800, 800)); // warm 200, big body
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
    // Both legs opened fresh connections (e.g. cross-origin redirect):
    // connect_ms is the sum, not the last.
    let mut acc = ResponseTiming::accumulator();
    acc.add_leg(&leg(false, Some(30), 10, 45));
    acc.add_leg(&leg(false, Some(25), 12, 40));
    assert_eq!(acc.connect_ms, Some(55));
    assert!(!acc.reused);
}

#[test]
fn error_for_status_caps_retained_body() {
    // A large body must not be stowed whole in Error::Status — it would
    // balloon logs and memory. Cap is 16 KiB.
    let mut resp = bare_response(None);
    resp.status = 500;
    resp.body = ResponseBody::Buffered(vec![b'x'; 2 * 1024 * 1024]);
    match resp.error_for_status().unwrap_err() {
        Error::Status { code, body, .. } => {
            assert_eq!(code, 500);
            assert_eq!(
                body.len(),
                16 * 1024,
                "Error::Status body must be capped at 16 KiB"
            );
            assert!(body.iter().all(|&b| b == b'x'), "prefix content preserved");
        }
        other => panic!("expected Error::Status, got {other:?}"),
    }
}

#[test]
fn error_for_status_keeps_short_body_intact() {
    // A body under the cap is retained verbatim.
    let mut resp = bare_response(None);
    resp.status = 404;
    resp.body = ResponseBody::Buffered(b"not found".to_vec());
    match resp.error_for_status().unwrap_err() {
        Error::Status { body, .. } => assert_eq!(body, b"not found"),
        other => panic!("expected Error::Status, got {other:?}"),
    }
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
