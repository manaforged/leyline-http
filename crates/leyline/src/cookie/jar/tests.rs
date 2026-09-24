use super::*;

#[test]
fn basic_set_and_get() {
    let jar = Jar::new();
    jar.set_cookie("https://example.com", "sid", "abc123");
    assert_eq!(
        jar.get_cookie("https://example.com", "sid"),
        Some("abc123".into())
    );
}

#[test]
fn cookie_ordering_chrome_style() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/app/page").unwrap();

    jar.store_set_cookie("a=1; Path=/", &url);
    jar.store_set_cookie("b=2; Path=/app", &url);
    jar.store_set_cookie("c=3; Path=/app/page", &url);

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

    jar.store_set_cookie("first=1; Path=/", &url);
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store_set_cookie("second=2; Path=/", &url);

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
    jar.load_cookies("a=1; b=2", "https://example.com/page");
    let export = jar.export_cookies("https://example.com/other");
    assert_eq!(export, "a=1; b=2");
}

#[test]
fn same_path_cookies_keep_creation_order() {
    let jar = Jar::new();
    jar.load_cookies(
        "zeta=r; alpha=i; mid=v",
        "https://www.example.com/page",
    );
    jar.set_cookie("https://www.example.com/", "late", "abc123");

    let export = jar.export_cookies("https://www.example.com/v1/items");
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
    jar.store_set_cookie("first=old; Path=/", &url);
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store_set_cookie("second=2; Path=/", &url);
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store_set_cookie("first=new; Path=/", &url);

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

    let export = jar.export_cookies("https://example.com/cart/items");
    assert_eq!(export, "deep=1; root=1; tail=1");
}

#[test]
fn expired_cookies_not_returned() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    jar.store_set_cookie("gone=bye; Max-Age=0", &url);
    assert_eq!(jar.get_cookie("https://example.com", "gone"), None);
}

#[test]
fn per_domain_eviction() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    for i in 0..=MAX_COOKIES_PER_DOMAIN {
        jar.store_set_cookie(&format!("c{}=v{}; Path=/", i, i), &url);
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

    jar.store_set_cookie("bad=val; SameSite=None", &url);
    assert_eq!(jar.get_cookie("https://example.com", "bad"), None);

    jar.store_set_cookie("good=val; SameSite=None; Secure", &url);
    assert_eq!(
        jar.get_cookie("https://example.com", "good"),
        Some("val".into())
    );
}

#[test]
fn samesite_enforced_on_cross_site_requests() {
    let jar = Jar::new();
    let set = Url::parse("https://example.com/").unwrap();
    jar.store_set_cookie("strict=1; SameSite=Strict", &set);
    jar.store_set_cookie("lax=1; SameSite=Lax", &set);
    jar.store_set_cookie("none=1; SameSite=None; Secure", &set);

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

    jar.store_set_cookie("session=good; Secure; Path=/", &https);
    jar.store_set_cookie("session=evil; Path=/", &http);
    assert_eq!(
        jar.get_cookie("https://example.com/", "session"),
        Some("good".into()),
        "plaintext overwrite of a Secure cookie must be refused"
    );

    jar.store_set_cookie("session=; Path=/; Max-Age=0", &http);
    assert_eq!(
        jar.get_cookie("https://example.com/", "session"),
        Some("good".into()),
        "plaintext deletion of a Secure cookie must be refused"
    );

    jar.store_set_cookie("session=rotated; Secure; Path=/", &https);
    assert_eq!(
        jar.get_cookie("https://example.com/", "session"),
        Some("rotated".into())
    );
    jar.store_set_cookie("session=; Path=/; Max-Age=0", &https);
    assert_eq!(jar.get_cookie("https://example.com/", "session"), None);
}

#[test]
fn secure_cookie_not_sent_over_http() {
    let jar = Jar::new();
    let https = Url::parse("https://example.com/").unwrap();
    jar.store_set_cookie("tok=secret; Secure; SameSite=None", &https);

    assert!(jar.get_cookie("https://example.com", "tok").is_some());
    assert!(jar.get_cookie("http://example.com", "tok").is_none());
}

#[test]
fn remove_named_removes_every_match() {
    let jar = Jar::new();
    jar.set_cookie("https://a.example.com", "k", "1");
    jar.set_cookie("https://b.example.com", "k", "2");
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
    );
    jar.store_set_cookie(
        "shared=value; Domain=example.com; Path=/; Secure",
        &Url::parse("https://www.example.com").unwrap(),
    );
    jar.store_set_cookie(
        "wwwonly=val; Path=/",
        &Url::parse("https://www.example.com").unwrap(),
    );
    assert_eq!(jar.all_cookies().len(), 3);

    let json = serde_json::to_string(&jar).expect("serialize");
    let restored: Jar = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.all_cookies().len(), 3);

    assert_eq!(
        restored.get_cookie("https://api.example.com", "auth"),
        Some("secret".into()),
        "auth must be visible on api.example.com"
    );
    assert_eq!(
        restored.get_cookie("https://www.example.com", "auth"),
        None,
        "auth must NOT leak to www.example.com (host-only)"
    );
    assert_eq!(
        restored.get_cookie("https://www.example.com", "shared"),
        Some("value".into())
    );
    assert_eq!(
        restored.get_cookie("https://api.example.com", "shared"),
        Some("value".into()),
        "Domain=example.com cookie must reach every subdomain"
    );
    assert_eq!(
        restored.get_cookie("https://www.example.com", "wwwonly"),
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
    jar.store_set_cookie("z=last; Path=/", &u1);
    jar.store_set_cookie("a=first; Path=/", &u2);
    jar.store_set_cookie("m=mid; Path=/account", &u3);
    jar.store_set_cookie("m=mid; Path=/", &u3);

    let s1 = serde_json::to_string(&jar).unwrap();
    let s2 = serde_json::to_string(&jar).unwrap();
    let s3 = serde_json::to_string(&jar).unwrap();
    assert_eq!(s1, s2, "serialization must be byte-stable");
    assert_eq!(s2, s3, "serialization must be byte-stable");
}
