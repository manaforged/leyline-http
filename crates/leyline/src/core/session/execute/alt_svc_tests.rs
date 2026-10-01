use http::{HeaderName, HeaderValue};

#[test]
fn alt_svc_older_than_its_max_age_is_not_cached() {
    let session = crate::Session::builder().build().unwrap();
    let url = url::Url::parse("https://example.com/").unwrap();
    let headers = [
        (
            HeaderName::from_static("alt-svc"),
            HeaderValue::from_static("h3=\":443\"; ma=60"),
        ),
        (
            HeaderName::from_static("age"),
            HeaderValue::from_static("120"),
        ),
    ];
    session.note_alt_svc(&url, &headers);
    assert!(!session.inner.pool.knows_h3("example.com", 443));
}

fn noted(fields: &[(&'static str, &'static str)]) -> bool {
    let session = crate::Session::builder().build().unwrap();
    let url = url::Url::parse("https://example.com/").unwrap();
    let headers: Vec<(HeaderName, HeaderValue)> = fields
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            )
        })
        .collect();
    session.note_alt_svc(&url, &headers);
    session.inner.pool.knows_h3("example.com", 443)
}

#[test]
fn alt_svc_reads_the_first_age_of_a_list() {
    assert!(!noted(&[
        ("alt-svc", "h3=\":443\"; ma=60"),
        ("age", "100, 100"),
    ]));
}

#[test]
fn alt_svc_keeps_the_longest_matching_entry() {
    assert!(noted(&[(
        "alt-svc",
        "h3=\":443\"; ma=0, h3=\":443\"; ma=3600"
    )]));
}

#[test]
fn alt_svc_parses_each_field_line_alone() {
    assert!(!noted(&[
        ("alt-svc", "h3=\":443\"; ma=86400; x=\""),
        ("alt-svc", "clear"),
    ]));
}

#[test]
fn alt_svc_reads_ma_in_any_case() {
    assert!(!noted(&[("alt-svc", "h3=\":443\"; MA=0")]));
}

#[test]
fn alt_svc_clear_survives_non_ascii_bytes() {
    let session = crate::Session::builder().build().unwrap();
    let url = url::Url::parse("https://example.com/").unwrap();
    session.note_alt_svc(
        &url,
        &[(
            HeaderName::from_static("alt-svc"),
            HeaderValue::from_static("h3=\":443\"; ma=3600"),
        )],
    );
    session.note_alt_svc(
        &url,
        &[(
            HeaderName::from_static("alt-svc"),
            HeaderValue::from_bytes(b"clear, x=\"caf\xc3\xa9\"").unwrap(),
        )],
    );
    assert!(!session.inner.pool.knows_h3("example.com", 443));
}

#[test]
fn alt_svc_ignores_an_entry_with_a_malformed_ma() {
    assert!(!noted(&[(
        "alt-svc",
        "h3=\":443\"; ma=0, h3=\":443\"; ma=abc"
    )]));
}

#[test]
fn alt_svc_age_overflow_is_the_largest_age() {
    assert!(!noted(&[
        ("alt-svc", "h3=\":443\"; ma=4294967295"),
        ("age", "18446744073709551616"),
    ]));
}
