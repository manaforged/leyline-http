use super::*;

#[test]
fn basic_set_and_get() {
    let jar = Jar::new();
    jar.set_cookie(
        &url::Url::parse("https://example.com").unwrap(),
        "sid",
        "abc123",
    )
    .unwrap();
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://example.com").unwrap(), "sid")
            .unwrap(),
        Some("abc123".into())
    );
}

#[test]
fn cookie_ordering_chrome_style() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/app/page").unwrap();

    jar.store_set_cookie("a=1; Path=/", &url).unwrap();
    jar.store_set_cookie("b=2; Path=/app", &url).unwrap();
    jar.store_set_cookie("c=3; Path=/app/page", &url).unwrap();

    let header = jar.cookie_header(&url).unwrap();
    assert!(
        header.starts_with("c=3"),
        "expected c=3 first, got: {header}"
    );
    assert!(header.contains("b=2"));
    assert!(header.ends_with("a=1"), "expected a=1 last, got: {header}");
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "sync test needs wall-clock separation for SystemTime ordering; disallow-rule targets async blocking"
)]
fn creation_time_ordering() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    jar.store_set_cookie("first=1; Path=/", &url).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store_set_cookie("second=2; Path=/", &url).unwrap();

    let header = jar.cookie_header(&url).unwrap();
    assert!(
        header.find("first=1").unwrap() < header.find("second=2").unwrap(),
        "older cookie should come first: {header}"
    );
}

#[test]
fn set_cookie_response() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    jar.store_response_cookies(
        &[
            "session=abc; Path=/; Secure; HttpOnly",
            "theme=dark; Path=/",
        ],
        &url,
    );

    let header = jar.cookie_header(&url).unwrap();
    assert!(header.contains("session=abc"));
    assert!(header.contains("theme=dark"));
}

#[test]
fn load_and_export() {
    let jar = Jar::new();
    jar.load_cookies(
        "a=1; b=2",
        &url::Url::parse("https://example.com/page").unwrap(),
    )
    .unwrap();
    let export = jar
        .export_cookies(&url::Url::parse("https://example.com/other").unwrap())
        .unwrap();
    assert_eq!(export, "a=1; b=2");
}

#[test]
fn same_path_cookies_keep_creation_order() {
    let jar = Jar::new();
    jar.load_cookies(
        "zeta=r; alpha=i; mid=v",
        &url::Url::parse("https://www.example.com/page").unwrap(),
    )
    .unwrap();
    jar.set_cookie(
        &url::Url::parse("https://www.example.com/").unwrap(),
        "late",
        "abc123",
    )
    .unwrap();

    let export = jar
        .export_cookies(&url::Url::parse("https://www.example.com/v1/items").unwrap())
        .unwrap();
    assert_eq!(
        export,
        "zeta=r; alpha=i; mid=v; late=abc123"
    );
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "sync test needs wall-clock separation for SystemTime ordering; disallow-rule targets async blocking"
)]
fn replacement_preserves_original_creation_order() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();
    jar.store_set_cookie("first=old; Path=/", &url).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store_set_cookie("second=2; Path=/", &url).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store_set_cookie("first=new; Path=/", &url).unwrap();

    let header = jar.cookie_header(&url).unwrap();
    assert_eq!(header, "first=new; second=2");
}

#[test]
fn longer_path_cookies_precede_same_path_creation_order() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/cart/items").unwrap();
    jar.store_response_cookies(
        &["root=1; Path=/", "deep=1; Path=/cart", "tail=1; Path=/"],
        &url,
    );

    let export = jar
        .export_cookies(&url::Url::parse("https://example.com/cart/items").unwrap())
        .unwrap();
    assert_eq!(export, "deep=1; root=1; tail=1");
}

#[test]
fn expired_cookies_not_returned() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    jar.store_set_cookie("gone=bye; Max-Age=0", &url).unwrap();
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://example.com").unwrap(), "gone")
            .unwrap(),
        None
    );
}

#[test]
fn per_domain_eviction() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    for i in 0..=MAX_COOKIES_PER_DOMAIN {
        jar.store_set_cookie(&format!("c{}=v{}; Path=/", i, i), &url)
            .unwrap();
    }

    let inner = lock(&jar.inner);
    let count = inner
        .cookies
        .get("example.com")
        .map(|v| v.len())
        .unwrap_or(0);
    assert!(
        count <= MAX_COOKIES_PER_DOMAIN,
        "expected <= {MAX_COOKIES_PER_DOMAIN}, got {count}"
    );
}

#[test]
fn samesite_none_requires_secure() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    jar.store_set_cookie("bad=val; SameSite=None", &url)
        .unwrap();
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://example.com").unwrap(), "bad")
            .unwrap(),
        None
    );

    jar.store_set_cookie("good=val; SameSite=None; Secure", &url)
        .unwrap();
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://example.com").unwrap(), "good")
            .unwrap(),
        Some("val".into())
    );
}

#[test]
fn samesite_enforced_on_cross_site_requests() {
    let jar = Jar::new();
    let set = Url::parse("https://example.com/").unwrap();
    jar.store_set_cookie("strict=1; SameSite=Strict", &set)
        .unwrap();
    jar.store_set_cookie("lax=1; SameSite=Lax", &set).unwrap();
    jar.store_set_cookie("none=1; SameSite=None; Secure", &set)
        .unwrap();

    let req = Url::parse("https://example.com/page").unwrap();

    let same = jar.cookie_header_for(&req, false, true).unwrap();
    assert!(same.contains("strict=1") && same.contains("lax=1") && same.contains("none=1"));

    let cross_get = jar.cookie_header_for(&req, true, true).unwrap();
    assert!(!cross_get.contains("strict=1"), "{cross_get}");
    assert!(cross_get.contains("lax=1") && cross_get.contains("none=1"));

    let cross_post = jar.cookie_header_for(&req, true, false).unwrap();
    assert!(!cross_post.contains("strict=1") && !cross_post.contains("lax=1"));
    assert!(cross_post.contains("none=1"));
}

#[test]
fn insecure_origin_cannot_overwrite_secure_cookie() {
    let jar = Jar::new();
    let https = Url::parse("https://example.com/").unwrap();
    let http = Url::parse("http://example.com/").unwrap();

    jar.store_set_cookie("session=good; Secure; Path=/", &https)
        .unwrap();
    jar.store_set_cookie("session=evil; Path=/", &http).unwrap();
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://example.com/").unwrap(), "session")
            .unwrap(),
        Some("good".into()),
        "plaintext overwrite of a Secure cookie must be refused"
    );

    jar.store_set_cookie("session=; Path=/; Max-Age=0", &http)
        .unwrap();
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://example.com/").unwrap(), "session")
            .unwrap(),
        Some("good".into()),
        "plaintext deletion of a Secure cookie must be refused"
    );

    jar.store_set_cookie("session=rotated; Secure; Path=/", &https)
        .unwrap();
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://example.com/").unwrap(), "session")
            .unwrap(),
        Some("rotated".into())
    );
    jar.store_set_cookie("session=; Path=/; Max-Age=0", &https)
        .unwrap();
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://example.com/").unwrap(), "session")
            .unwrap(),
        None
    );
}

#[test]
fn secure_cookie_not_sent_over_http() {
    let jar = Jar::new();
    let https = Url::parse("https://example.com/").unwrap();
    jar.store_set_cookie("tok=secret; Secure; SameSite=None", &https)
        .unwrap();

    assert!(
        jar.get_cookie(&url::Url::parse("https://example.com").unwrap(), "tok")
            .unwrap()
            .is_some()
    );
    assert!(
        jar.get_cookie(&url::Url::parse("http://example.com").unwrap(), "tok")
            .unwrap()
            .is_none()
    );
}

#[test]
fn remove_named_removes_every_match() {
    let jar = Jar::new();
    jar.set_cookie(&url::Url::parse("https://a.example.com").unwrap(), "k", "1")
        .unwrap();
    jar.set_cookie(&url::Url::parse("https://b.example.com").unwrap(), "k", "2")
        .unwrap();
    assert_eq!(jar.all_cookies().len(), 2);
    assert_eq!(jar.remove_named("k"), 2);
    assert!(jar.all_cookies().is_empty());
    assert_eq!(jar.remove_named("k"), 0);
}

#[test]
fn serde_round_trip_preserves_cross_subdomain_attribution() {
    let jar = Jar::new();
    jar.store_set_cookie(
        "auth=secret; Path=/; Secure",
        &Url::parse("https://api.example.com").unwrap(),
    )
    .unwrap();
    jar.store_set_cookie(
        "shared=value; Domain=example.com; Path=/; Secure",
        &Url::parse("https://www.example.com").unwrap(),
    )
    .unwrap();
    jar.store_set_cookie(
        "wwwonly=val; Path=/",
        &Url::parse("https://www.example.com").unwrap(),
    )
    .unwrap();
    assert_eq!(jar.all_cookies().len(), 3);

    let json = serde_json::to_string(&jar).expect("serialize");
    let restored: Jar = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.all_cookies().len(), 3);

    assert_eq!(
        restored
            .get_cookie(&url::Url::parse("https://api.example.com").unwrap(), "auth")
            .unwrap(),
        Some("secret".into()),
        "auth must be visible on api.example.com"
    );
    assert_eq!(
        restored
            .get_cookie(&url::Url::parse("https://www.example.com").unwrap(), "auth")
            .unwrap(),
        None,
        "auth must NOT leak to www.example.com (host-only)"
    );
    assert_eq!(
        restored
            .get_cookie(
                &url::Url::parse("https://www.example.com").unwrap(),
                "shared"
            )
            .unwrap(),
        Some("value".into())
    );
    assert_eq!(
        restored
            .get_cookie(
                &url::Url::parse("https://api.example.com").unwrap(),
                "shared"
            )
            .unwrap(),
        Some("value".into()),
        "Domain=example.com cookie must reach every subdomain"
    );
    assert_eq!(
        restored
            .get_cookie(
                &url::Url::parse("https://www.example.com").unwrap(),
                "wwwonly"
            )
            .unwrap(),
        Some("val".into())
    );
}

#[test]
fn serde_empty_jar() {
    let jar = Jar::new();
    let json = serde_json::to_string(&jar).expect("serialize");
    assert_eq!(json, "[]");
    let restored: Jar = serde_json::from_str(&json).expect("deserialize");
    assert!(restored.all_cookies().is_empty());
}

#[test]
fn serialize_order_is_stable_across_runs() {
    let jar = Jar::new();
    let u1 = Url::parse("https://www.example.com/").unwrap();
    let u2 = Url::parse("https://api.example.com/").unwrap();
    let u3 = Url::parse("https://api.example.com/").unwrap();
    jar.store_set_cookie("z=last; Path=/", &u1).unwrap();
    jar.store_set_cookie("a=first; Path=/", &u2).unwrap();
    jar.store_set_cookie("m=mid; Path=/account", &u3).unwrap();
    jar.store_set_cookie("m=mid; Path=/", &u3).unwrap();

    let s1 = serde_json::to_string(&jar).unwrap();
    let s2 = serde_json::to_string(&jar).unwrap();
    let s3 = serde_json::to_string(&jar).unwrap();
    assert_eq!(s1, s2, "serialization must be byte-stable");
    assert_eq!(s2, s3, "serialization must be byte-stable");
}
