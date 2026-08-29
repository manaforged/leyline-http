use super::*;

#[test]
fn default_policy_retries_5xx_subset() {
    let p = RetryPolicy::default();
    assert!(p.matches_status(502));
    assert!(p.matches_status(503));
    assert!(p.matches_status(504));
    assert!(!p.matches_status(500));
    assert!(!p.matches_status(400));
}

#[test]
fn server_error_trigger_matches_all_5xx() {
    let p = RetryPolicy {
        retry_on: vec![RetryTrigger::ServerError],
        ..RetryPolicy::default()
    };
    assert!(p.matches_status(500));
    assert!(p.matches_status(599));
    assert!(!p.matches_status(400));
}

#[test]
fn backoff_grows_and_caps() {
    let p = RetryPolicy {
        max_retries: 10,
        initial_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_millis(800),
        backoff_factor: 2.0,
        jitter: false,
        retry_on: vec![RetryTrigger::ConnectionError],
    };
    assert!(p.backoff(0).as_millis() <= 100);
    assert!(p.backoff(1).as_millis() <= 200);
    assert!(p.backoff(10).as_millis() <= 800);
}

#[test]
fn none_is_no_retry() {
    let p = RetryPolicy::none();
    assert!(p.is_none());
    assert_eq!(p.max_retries, 0);
}

#[test]
fn retry_after_parses_delta_seconds_only() {
    assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
    assert_eq!(parse_retry_after("  5 "), Some(Duration::from_secs(5)));
    assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
    // HTTP-date form is not parsed here — falls through to backoff.
    assert_eq!(parse_retry_after("Wed, 21 Oct 2025 07:28:00 GMT"), None);
    assert_eq!(parse_retry_after("soon"), None);
}

#[test]
fn idempotent_matches_rfc_set() {
    for m in ["GET", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"] {
        assert!(is_idempotent(m), "{m}");
        assert!(is_idempotent(&m.to_lowercase()), "{m}");
    }
    for m in ["POST", "PATCH"] {
        assert!(!is_idempotent(m), "{m}");
    }
}
