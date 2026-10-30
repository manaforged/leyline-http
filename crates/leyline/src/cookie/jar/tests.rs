use super::evict::{MAX_COOKIES_GLOBAL, MAX_COOKIES_PER_DOMAIN};
use super::*;

#[test]
fn basic_set_and_get() {
    let jar = Jar::new();
    jar.set(
        &url::Url::parse("https://example.com").unwrap(),
        "sid",
        "abc123",
    );
    assert_eq!(
        jar.get(&url::Url::parse("https://example.com").unwrap(), "sid"),
        Some("abc123".into())
    );
}

#[test]
fn cookie_ordering_chrome_style() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/app/page").unwrap();

    jar.store("a=1; Path=/", &url);
    jar.store("b=2; Path=/app", &url);
    jar.store("c=3; Path=/app/page", &url);

    let header = jar.header_for(&url).unwrap();
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

    jar.store("first=1; Path=/", &url);
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store("second=2; Path=/", &url);

    let header = jar.header_for(&url).unwrap();
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

    let header = jar.header_for(&url).unwrap();
    assert!(header.contains("session=abc"));
    assert!(header.contains("theme=dark"));
}

#[test]
fn load_and_export() {
    let jar = Jar::new();
    let page = url::Url::parse("https://example.com/page").unwrap();
    jar.set(&page, "a", "1");
    jar.set(&page, "b", "2");
    let export = jar
        .header_for(&url::Url::parse("https://example.com/other").unwrap())
        .unwrap_or_default();
    assert_eq!(export, "a=1; b=2");
}

#[test]
fn same_path_cookies_keep_creation_order() {
    let jar = Jar::new();
    let page = url::Url::parse("https://www.example.com/page").unwrap();
    for (name, value) in [("zeta", "r"), ("alpha", "i"), ("mid", "v")] {
        jar.set(&page, name, value);
    }
    jar.set(
        &url::Url::parse("https://www.example.com/").unwrap(),
        "late",
        "abc123",
    );

    let export = jar
        .header_for(&url::Url::parse("https://www.example.com/v1/items").unwrap())
        .unwrap_or_default();
    assert_eq!(export, "zeta=r; alpha=i; mid=v; late=abc123");
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "sync test needs wall-clock separation for SystemTime ordering; disallow-rule targets async blocking"
)]
fn replacement_preserves_original_creation_order() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();
    jar.store("first=old; Path=/", &url);
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store("second=2; Path=/", &url);
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store("first=new; Path=/", &url);

    let header = jar.header_for(&url).unwrap();
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
        .header_for(&url::Url::parse("https://example.com/cart/items").unwrap())
        .unwrap_or_default();
    assert_eq!(export, "deep=1; root=1; tail=1");
}

#[test]
fn expired_cookies_not_returned() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    jar.store("gone=bye; Max-Age=0", &url);
    assert_eq!(
        jar.get(&url::Url::parse("https://example.com").unwrap(), "gone"),
        None
    );
}

#[test]
fn per_domain_eviction() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    for i in 0..=MAX_COOKIES_PER_DOMAIN {
        jar.store(&format!("c{}=v{}; Path=/", i, i), &url);
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

    jar.store("bad=val; SameSite=None", &url);
    assert_eq!(
        jar.get(&url::Url::parse("https://example.com").unwrap(), "bad"),
        None
    );

    jar.store("good=val; SameSite=None; Secure", &url);
    assert_eq!(
        jar.get(&url::Url::parse("https://example.com").unwrap(), "good"),
        Some("val".into())
    );
}

#[test]
fn samesite_enforced_on_cross_site_requests() {
    let jar = Jar::new();
    let set = Url::parse("https://example.com/").unwrap();
    jar.store("strict=1; SameSite=Strict", &set);
    jar.store("lax=1; SameSite=Lax", &set);
    jar.store("none=1; SameSite=None; Secure", &set);

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

    jar.store("session=good; Secure; Path=/", &https);
    jar.store("session=evil; Path=/", &http);
    assert_eq!(
        jar.get(&url::Url::parse("https://example.com/").unwrap(), "session"),
        Some("good".into()),
        "plaintext overwrite of a Secure cookie must be refused"
    );

    jar.store("session=; Path=/; Max-Age=0", &http);
    assert_eq!(
        jar.get(&url::Url::parse("https://example.com/").unwrap(), "session"),
        Some("good".into()),
        "plaintext deletion of a Secure cookie must be refused"
    );

    jar.store("session=rotated; Secure; Path=/", &https);
    assert_eq!(
        jar.get(&url::Url::parse("https://example.com/").unwrap(), "session"),
        Some("rotated".into())
    );
    jar.store("session=; Path=/; Max-Age=0", &https);
    assert_eq!(
        jar.get(&url::Url::parse("https://example.com/").unwrap(), "session"),
        None
    );
}

#[test]
fn secure_cookie_not_sent_over_http() {
    let jar = Jar::new();
    let https = Url::parse("https://example.com/").unwrap();
    jar.store("tok=secret; Secure; SameSite=None", &https);

    assert!(
        jar.get(&url::Url::parse("https://example.com").unwrap(), "tok")
            .is_some()
    );
    assert!(
        jar.get(&url::Url::parse("http://example.com").unwrap(), "tok")
            .is_none()
    );
}

#[test]
fn remove_everywhere_removes_every_match() {
    let jar = Jar::new();
    jar.set(&url::Url::parse("https://a.example.com").unwrap(), "k", "1");
    jar.set(&url::Url::parse("https://b.example.com").unwrap(), "k", "2");
    assert_eq!(jar.all().len(), 2);
    assert_eq!(jar.remove_everywhere("k"), 2);
    assert!(jar.all().is_empty());
    assert_eq!(jar.remove_everywhere("k"), 0);
}

#[test]
fn serde_round_trip_preserves_cross_subdomain_attribution() {
    let jar = Jar::new();
    jar.store(
        "auth=secret; Path=/; Secure",
        &Url::parse("https://api.example.com").unwrap(),
    );
    jar.store(
        "shared=value; Domain=example.com; Path=/; Secure",
        &Url::parse("https://www.example.com").unwrap(),
    );
    jar.store(
        "wwwonly=val; Path=/",
        &Url::parse("https://www.example.com").unwrap(),
    );
    assert_eq!(jar.all().len(), 3);

    let json = serde_json::to_string(&jar).expect("serialize");
    let restored: Jar = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.all().len(), 3);

    assert_eq!(
        restored.get(&url::Url::parse("https://api.example.com").unwrap(), "auth"),
        Some("secret".into()),
        "auth must be visible on api.example.com"
    );
    assert_eq!(
        restored.get(&url::Url::parse("https://www.example.com").unwrap(), "auth"),
        None,
        "auth must NOT leak to www.example.com (host-only)"
    );
    assert_eq!(
        restored.get(
            &url::Url::parse("https://www.example.com").unwrap(),
            "shared"
        ),
        Some("value".into())
    );
    assert_eq!(
        restored.get(
            &url::Url::parse("https://api.example.com").unwrap(),
            "shared"
        ),
        Some("value".into()),
        "Domain=example.com cookie must reach every subdomain"
    );
    assert_eq!(
        restored.get(
            &url::Url::parse("https://www.example.com").unwrap(),
            "wwwonly"
        ),
        Some("val".into())
    );
}

#[test]
fn serde_empty_jar() {
    let jar = Jar::new();
    let json = serde_json::to_string(&jar).expect("serialize");
    assert_eq!(json, "[]");
    let restored: Jar = serde_json::from_str(&json).expect("deserialize");
    assert!(restored.all().is_empty());
}

#[test]
fn serialize_order_is_stable_across_runs() {
    let jar = Jar::new();
    let u1 = Url::parse("https://www.example.com/").unwrap();
    let u2 = Url::parse("https://api.example.com/").unwrap();
    let u3 = Url::parse("https://api.example.com/").unwrap();
    jar.store("z=last; Path=/", &u1);
    jar.store("a=first; Path=/", &u2);
    jar.store("m=mid; Path=/account", &u3);
    jar.store("m=mid; Path=/", &u3);

    let s1 = serde_json::to_string(&jar).unwrap();
    let s2 = serde_json::to_string(&jar).unwrap();
    let s3 = serde_json::to_string(&jar).unwrap();
    assert_eq!(s1, s2, "serialization must be byte-stable");
    assert_eq!(s2, s3, "serialization must be byte-stable");
}

#[test]
fn churn_across_hosts_leaves_no_empty_domain_buckets() {
    let jar = Jar::new();
    let host = |i: usize| url::Url::parse(&format!("https://h{i}.example/")).unwrap();
    for i in 0..MAX_COOKIES_GLOBAL + 100 {
        jar.store("a=1", &host(i));
    }
    jar.store("a=1; Max-Age=0", &host(MAX_COOKIES_GLOBAL + 99));

    let live = jar.all().len();
    assert_eq!(
        format!("{jar:?}"),
        format!("Jar {{ domains: {live}, total: {live} }}")
    );
}

#[test]
fn deleting_a_cookie_leaves_other_sites_alone() {
    let bank = url::Url::parse("https://bank.test/").unwrap();
    let evil = url::Url::parse("http://evil.test/").unwrap();
    let jar = Jar::new();
    jar.store("sid=keep; Path=/; Secure", &bank);
    jar.store("sid=1; Path=/", &evil);
    jar.store("sid=; Path=/; Max-Age=0", &evil);

    assert_eq!(jar.get(&bank, "sid"), Some("keep".into()));
    assert_eq!(jar.get(&evil, "sid"), None);
}
