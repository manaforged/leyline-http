/// `with_proxy` must actually override a proxy set at build time.
/// `proxy_for` returns the FIRST matching rule, so appending an
/// all-scheme rule would let the original build-time proxy keep
/// winning and silently no-op the rotation.
#[test]
fn with_proxy_overrides_build_time_proxy() {
    let session = crate::Session::builder()
        .proxy("http://first:1")
        .build()
        .expect("bare session builds");
    let rotated = session.with_proxy("http://second:2");

    let url = url::Url::parse("https://example.test/").unwrap();
    assert_eq!(
        rotated.effective_proxy_for(&url, None),
        Some("http://second:2"),
        "with_proxy must win over the build-time proxy"
    );
}
