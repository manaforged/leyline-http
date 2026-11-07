use super::*;
use http::StatusCode;

use crate::error::{Error, Kind};
use crate::util::wait::{parse_retry_after, unix_from_ymd_hms};

fn grid_response(status: u16, retry_after: Option<&str>) -> Response {
    let mut headers = http::HeaderMap::new();
    if let Some(value) = retry_after {
        headers.insert("retry-after", value.parse().unwrap());
    }
    Response::from_parts(crate::core::response::Parts {
        status: StatusCode::from_u16(status).unwrap(),
        version: http::Version::HTTP_2,
        headers: headers.into(),
        trailers: None,
        body: crate::core::response::ResponseBody::Buffered(Vec::new()),
        url: std::sync::Arc::new(url::Url::parse("https://example.test/").unwrap()),
        redirect_chain: Vec::new(),
        request_headers: Vec::new(),
        tls: None,
        request_method: Some(http::Method::GET),
        audit_tls: None,
        audit_cache: std::sync::OnceLock::new(),
        timing: crate::wire::ResponseTiming::default(),
        compression: crate::core::BodyRules::default(),
        attempts: 1,
        proxy: None,
        block: None,
    })
}

fn grid() -> Vec<(&'static str, Result<Response>)> {
    let handshake = std::io::Error::new(std::io::ErrorKind::TimedOut, "handshake");
    let reset = std::io::Error::from(std::io::ErrorKind::ConnectionReset);
    vec![
        ("200", Ok(grid_response(200, None))),
        ("409", Ok(grid_response(409, None))),
        ("429", Ok(grid_response(429, None))),
        ("500", Ok(grid_response(500, None))),
        ("503", Ok(grid_response(503, None))),
        ("503 retry-after 5", Ok(grid_response(503, Some("5")))),
        ("503 retry-after 120", Ok(grid_response(503, Some("120")))),
        ("504", Ok(grid_response(504, None))),
        ("block 403", Ok(grid_response(403, None))),
        ("connect refused", Err(Error::new(Kind::Connect))),
        (
            "tls handshake timeout",
            Err(Error::from(crate::tls::error::TlsError::HandshakeIo(
                handshake,
            ))),
        ),
        ("timeout after send", Err(Error::new(Kind::Timeout))),
        ("reset after send", Err(Error::from(reset))),
    ]
}

fn plan_label(plan: RetryPlan) -> &'static str {
    match plan {
        RetryPlan::Stop => "stop",
        RetryPlan::Exhausted => "exhausted",
        RetryPlan::Declined => "declined",
        RetryPlan::Backoff(wait) if wait == Duration::from_secs(5) => "wait 5s",
        RetryPlan::Backoff(_) => "backoff",
    }
}

#[test]
fn decisions_match_the_table_and_an_unsent_post_retries() {
    let mut table = String::new();
    for (name, retry) in [("transient", Retry::new(3)), ("none", Retry::new(0))] {
        for method in [http::Method::GET, http::Method::POST] {
            for attempt in [0, 3] {
                for (label, result) in grid() {
                    let plan = retry.plan_resend(&result, attempt, retry.allows(&method), || true);
                    table.push_str(&format!(
                        "{name} {method} attempt {attempt} {label}: {}\n",
                        plan_label(plan)
                    ));
                }
            }
        }
    }
    assert_eq!(table, include_str!("decisions.txt"));
}

#[test]
fn backoff_doubles_from_min_and_stops_at_max() {
    let backoff = Backoff::exponential(Duration::from_millis(100), Duration::from_millis(800));
    let ceilings: Vec<u128> = (0..6).map(|n| backoff.ceiling(n).as_millis()).collect();
    assert_eq!(ceilings, [100, 200, 400, 800, 800, 800]);
    assert_eq!(backoff.ceiling(u32::MAX), Duration::from_millis(800));
    assert!((0..50).all(|n| {
        let delay = backoff.delay(n);
        delay >= backoff.ceiling(n) / 2 && delay <= backoff.ceiling(n)
    }));
}

#[test]
fn when_replaces_the_default_rule() {
    let conflict = Ok(grid_response(409, None));
    let mut blocked = grid_response(429, None);
    blocked.parts.block = Some(crate::core::Block::Denied("status".into()));
    let limited = Ok(blocked);
    let on_conflict = Retry::new(3).when(|attempt| attempt.status() == Some(StatusCode::CONFLICT));
    let skip_blocks = Retry::new(3).when(|attempt| attempt.is_transient() && !attempt.is_block());
    let plan = |retry: &Retry, result: &Result<Response>| plan_label(retry.plan(result, 0));
    assert_eq!(plan(&on_conflict, &conflict), "backoff");
    assert_eq!(plan(&on_conflict, &limited), "stop");
    assert_eq!(plan(&skip_blocks, &limited), "stop");
    assert_eq!(plan(&Retry::new(3), &limited), "backoff");
}

#[test]
fn retry_after_parses_delta_seconds_only() {
    assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
    assert_eq!(parse_retry_after("  5 "), Some(Duration::from_secs(5)));
    assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
    assert_eq!(parse_retry_after("soon"), None);
}

#[test]
fn retry_after_parses_past_imf_fixdate_as_zero() {
    assert_eq!(
        parse_retry_after("Wed, 21 Oct 2025 07:28:00 GMT"),
        Some(Duration::ZERO)
    );
}

#[test]
fn imf_fixdate_epoch() {
    assert_eq!(
        unix_from_ymd_hms(1970, 1, 1, 0, 0, 0),
        Some(std::time::UNIX_EPOCH)
    );
}

#[test]
fn a_retry_after_date_past_the_platform_clock_does_not_panic() {
    let wait = parse_retry_after("Fri, 01 Jan 99999 00:00:00 GMT");
    assert!(wait.is_none_or(|wait| wait > Duration::from_secs(1 << 40)));
}

#[test]
fn wait_formats_parse_header_values() {
    assert_eq!(WaitFormat::Seconds.parse("7"), Some(Duration::from_secs(7)));
    let soon = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60;
    let wait = WaitFormat::UnixSeconds.parse(&soon.to_string()).unwrap();
    assert!(wait > Duration::from_secs(50) && wait <= Duration::from_secs(60));
    assert_eq!(WaitFormat::Seconds.parse("soon"), None);
}

#[test]
fn a_tls_handshake_timeout_is_unsent() {
    let timeout = std::io::Error::new(std::io::ErrorKind::TimedOut, "handshake");
    let error = Error::from(crate::tls::error::TlsError::HandshakeIo(timeout));
    assert_eq!(error.kind(), Kind::Timeout);
    assert!(error.is_unsent());
}

#[test]
fn a_failed_tls_handshake_is_unsent() {
    let error = Error::from(crate::tls::error::TlsError::Handshake("alert".into()));
    assert!(error.is_unsent());
}

#[test]
fn a_spent_budget_stops_retries_and_each_request_refills_it() {
    let retry = Retry::new(3).budget(0.5);
    let failed = || Err::<Response, _>(Error::new(Kind::Connect));
    let plan = |attempt| plan_label(retry.plan_resend(&failed(), attempt, true, || true));
    let allowed = (0..200).take_while(|_| plan(1) == "backoff").count();
    assert_eq!(allowed, 100);
    assert_eq!(plan(1), "stop");
    assert_eq!(plan(0), "stop");
    assert_eq!(plan(0), "backoff");
    assert_eq!(plan(1), "stop");
}
