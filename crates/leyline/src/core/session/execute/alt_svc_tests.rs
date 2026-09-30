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
