use super::*;

#[test]
fn basic_set_and_get() {
    let jar = Jar::new();
    jar.set_cookie("https://example.com", "_ab", "abc123");
    assert_eq!(
        jar.get_cookie("https://example.com", "_ab"),
        Some("abc123".into())
    );
}

#[test]
fn cookie_ordering_chrome_style() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/app/page").unwrap();

    // Set cookies with different paths and creation times.
    // Use store_set_cookie directly for control.
    jar.store_set_cookie("a=1; Path=/", &url);
    jar.store_set_cookie("b=2; Path=/app", &url);
    jar.store_set_cookie("c=3; Path=/app/page", &url);

    let header = jar.cookie_header(&url).unwrap();
    // Order: /app/page (longest path) first, then /app, then /
    assert!(
        header.starts_with("c=3"),
        "expected c=3 first, got: {header}"
    );
    assert!(header.contains("b=2"));
    assert!(header.ends_with("a=1"), "expected a=1 last, got: {header}");
}

#[test]
// Sync test: needs wall-clock separation for SystemTime creation-time
// ordering; the clippy.toml disallow-list targets async blocking.
#[expect(
    clippy::disallowed_methods,
    reason = "sync test needs wall-clock separation for SystemTime ordering; disallow-rule targets async blocking"
)]
fn creation_time_ordering() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    // Same path, different creation times. Oldest should come first.
    jar.store_set_cookie("first=1; Path=/", &url);
    std::thread::sleep(std::time::Duration::from_millis(10));
    jar.store_set_cookie("second=2; Path=/", &url);

    let header = jar.cookie_header(&url).unwrap();
    // Same path length → creation time ascending → first before second.
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
// Sync test: needs wall-clock separation for SystemTime creation-time
// ordering; the clippy.toml disallow-list targets async blocking.
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

    // Max-Age=0 means delete/expire immediately.
    jar.store_set_cookie("gone=bye; Max-Age=0", &url);
    assert_eq!(jar.get_cookie("https://example.com", "gone"), None);
}

#[test]
fn per_domain_eviction() {
    let jar = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();

    // Insert 181 cookies — should trigger eviction.
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

    // Same-site: all three are eligible.
    let same = jar.cookie_header_for(&req, false, true).unwrap();
    assert!(same.contains("strict=1") && same.contains("lax=1") && same.contains("none=1"));

    // Cross-site safe navigation (GET): Strict withheld; Lax + None sent.
    let cross_get = jar.cookie_header_for(&req, true, true).unwrap();
    assert!(!cross_get.contains("strict=1"), "{cross_get}");
    assert!(cross_get.contains("lax=1") && cross_get.contains("none=1"));

    // Cross-site unsafe navigation (POST): only None sent.
    let cross_post = jar.cookie_header_for(&req, true, false).unwrap();
    assert!(!cross_post.contains("strict=1") && !cross_post.contains("lax=1"));
    assert!(cross_post.contains("none=1"));
}

/// RFC 6265bis §5.7 "Leave Secure Cookies Alone": a plaintext response
/// cannot overwrite (or delete) a Secure cookie the HTTPS origin set.
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

    // Same for a plaintext deletion attempt.
    jar.store_set_cookie("session=; Path=/; Max-Age=0", &http);
    assert_eq!(
        jar.get_cookie("https://example.com/", "session"),
        Some("good".into()),
        "plaintext deletion of a Secure cookie must be refused"
    );

    // The HTTPS origin can still refresh its own Secure cookie and
    // delete it with Max-Age=0.
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

    // Available over HTTPS.
    assert!(jar.get_cookie("https://example.com", "tok").is_some());
    // NOT available over HTTP.
    assert!(jar.get_cookie("http://example.com", "tok").is_none());
}

#[test]
fn get_named_finds_across_domains() {
    let jar = Jar::new();
    let www = Url::parse("https://www.example.com/").unwrap();
    let api = Url::parse("https://api.example.com/").unwrap();
    jar.store_set_cookie("a=www; Path=/", &www);
    jar.store_set_cookie("b=api; Path=/", &api);
    assert_eq!(jar.get_named("a").as_deref(), Some("www"));
    assert_eq!(jar.get_named("b").as_deref(), Some("api"));
    assert_eq!(jar.get_named("missing"), None);
}

#[test]
fn set_named_updates_existing_only() {
    let jar = Jar::new();
    jar.set_cookie("https://example.com", "tok", "old");
    assert!(jar.set_named("tok", "new"));
    assert_eq!(jar.get_named("tok").as_deref(), Some("new"));
    assert!(!jar.set_named("absent", "v"));
    assert_eq!(jar.get_named("absent"), None);
}

#[test]
fn set_named_on_upserts() {
    let jar = Jar::new();
    jar.set_named_on("api.example.com", "session", "first");
    assert_eq!(jar.get_named("session").as_deref(), Some("first"));
    jar.set_named_on("api.example.com", "session", "second");
    assert_eq!(jar.get_named("session").as_deref(), Some("second"));
    assert_eq!(jar.len(), 1);
}

#[test]
fn set_named_on_cookie_is_host_only_no_cross_host_leak() {
    // The trusted-caller contract on `set_named_on` rests on the inserted
    // cookie being host-only: even a public-suffix `domain` cannot become a
    // supercookie because a host-only cookie is sent ONLY on an exact host
    // match. Lock that invariant so a future change to the insert path
    // (e.g. dropping `host_only: true`) can't silently turn these into
    // domain-broadcasting cookies.
    let jar = Jar::new();
    jar.set_named_on("example.com", "sess", "v");
    // Visible on the exact host.
    assert_eq!(
        jar.get_cookie("https://example.com/", "sess").as_deref(),
        Some("v")
    );
    // NOT visible on a subdomain or a sibling host.
    assert_eq!(jar.get_cookie("https://www.example.com/", "sess"), None);
    assert_eq!(jar.get_cookie("https://api.example.com/", "sess"), None);

    // Same guarantee even when `domain` is a public suffix: the cookie is
    // pinned to that exact host and does not broadcast to registrable
    // domains under it.
    jar.set_named_on("co.uk", "psl", "v");
    assert_eq!(
        jar.get_cookie("https://co.uk/", "psl").as_deref(),
        Some("v")
    );
    assert_eq!(jar.get_cookie("https://example.co.uk/", "psl"), None);
}

#[test]
fn remove_named_first_match() {
    let jar = Jar::new();
    jar.set_cookie("https://a.example.com", "k", "1");
    jar.set_cookie("https://b.example.com", "k", "2");
    assert_eq!(jar.len(), 2);
    assert!(jar.remove_named("k"));
    assert_eq!(jar.len(), 1);
    assert!(jar.remove_named("k"));
    assert!(!jar.remove_named("k"));
}

#[test]
fn remove_all_named_clears_every_domain() {
    let jar = Jar::new();
    jar.set_cookie("https://a.example.com", "k", "1");
    jar.set_cookie("https://b.example.com", "k", "2");
    jar.set_cookie("https://c.example.com", "other", "3");
    assert_eq!(jar.remove_all_named("k"), 2);
    assert_eq!(jar.len(), 1);
    assert!(jar.contains_named("other"));
}

#[test]
fn remove_named_for_host_spares_siblings() {
    let jar = Jar::new();
    jar.set_cookie("https://store.example.com", "session", "store");
    jar.set_cookie("https://www.example.com", "session", "www");
    jar.set_cookie("https://example.com", "session", "apex");
    jar.set_cookie("https://store.example.com", "other", "keep");

    // Clears the store host + the parent-domain (apex) mis-host, but leaves
    // the sibling www zone and unrelated cookies untouched.
    assert_eq!(
        jar.remove_named_for_host("store.example.com", "session"),
        2
    );
    let store_view = jar.export_cookies("https://store.example.com");
    assert!(
        !store_view.contains("session"),
        "store clearance removed, got {store_view:?}"
    );
    assert!(
        store_view.contains("other=keep"),
        "unrelated cookie kept, got {store_view:?}"
    );
    assert!(
        jar.export_cookies("https://www.example.com")
            .contains("session=www"),
        "sibling www zone must survive"
    );
}

#[test]
fn merge_combines_jars_last_write_wins() {
    let a = Jar::new();
    a.set_cookie("https://example.com", "k1", "from_a");
    a.set_cookie("https://example.com", "shared", "a_value");

    let b = Jar::new();
    b.set_cookie("https://example.com", "k2", "from_b");
    b.set_cookie("https://example.com", "shared", "b_value");

    a.merge(&b);
    assert_eq!(a.get_named("k1").as_deref(), Some("from_a"));
    assert_eq!(a.get_named("k2").as_deref(), Some("from_b"));
    assert_eq!(a.get_named("shared").as_deref(), Some("b_value"));
}

// ─── Persistence ─────────────────────────────────

#[test]
fn serde_round_trip_preserves_cross_subdomain_attribution() {
    // A host-only cookie on `api.example.com` must round-trip
    // losslessly through serde. A `String`-shaped export that filtered
    // against `https://www.example.com` would silently drop it.
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
    assert_eq!(jar.len(), 3);

    let json = serde_json::to_string(&jar).expect("serialize");
    let restored: Jar = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.len(), 3);

    // The host-only gsp cookie is back pinned to gsp, not www.
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
    assert!(restored.is_empty());
}

#[test]
fn deep_clone_is_independent() {
    let a = Jar::new();
    a.set_cookie("https://example.com", "k", "1");
    let b = a.deep_clone();
    a.set_named("k", "2");
    assert_eq!(a.get_named("k").as_deref(), Some("2"));
    assert_eq!(b.get_named("k").as_deref(), Some("1"));
}

// ─── set_named / merge invariants ───────────────────────────────────────────

#[test]
fn set_named_updates_every_match_across_domains() {
    // `set_named` must update every match and return true if any. Stopping
    // at the first match returned by HashMap iteration is non-deterministic
    // when the same cookie name lives on multiple domains.
    let jar = Jar::new();
    jar.set_cookie("https://example.com", "token", "old1");
    jar.set_cookie("https://api.example.com", "token", "old2");
    assert_eq!(jar.len(), 2);
    assert!(jar.set_named("token", "rotated"));
    // Both sites see the new token regardless of HashMap iteration order.
    assert_eq!(
        jar.get_cookie("https://example.com", "token")
            .as_deref(),
        Some("rotated")
    );
    assert_eq!(
        jar.get_cookie("https://api.example.com", "token")
            .as_deref(),
        Some("rotated")
    );
}

#[test]
fn set_named_on_updates_existing_path_not_just_root() {
    // `set_named_on` must match cookies on any path, not just `Path=/`. A
    // cookie minted by the server on `Path=/auth` would otherwise go
    // unmatched, and a duplicate would be inserted on `Path=/`, producing
    // two entries with the same name and silently emitting both in the
    // Cookie header.
    let jar = Jar::new();
    let url = Url::parse("https://example.com/auth/redirect").unwrap();
    jar.store_set_cookie("accessToken=initial; Path=/auth", &url);
    assert_eq!(jar.len(), 1);

    jar.set_named_on("example.com", "token", "rotated");
    assert_eq!(
        jar.len(),
        1,
        "must update existing entry, not insert duplicate"
    );
    // Original path attribute preserved — the cookie still applies on
    // /auth, not just /.
    assert_eq!(
        jar.get_cookie("https://example.com/auth/foo", "token")
            .as_deref(),
        Some("rotated")
    );
}

#[test]
// Sync test: needs wall-clock separation for SystemTime last_access
// ordering; the clippy.toml disallow-list targets async blocking.
#[expect(
    clippy::disallowed_methods,
    reason = "sync test needs wall-clock separation for SystemTime ordering; disallow-rule targets async blocking"
)]
fn set_named_bumps_last_access() {
    // `set_named` must bump `last_access`; leaving it stale would let a
    // recently-rotated auth token be evicted before truly-cold cookies
    // under LRU pressure.
    let jar = Jar::new();
    jar.set_cookie("https://example.com", "tok", "old");
    let before = {
        let inner = lock(&jar.inner);
        inner.cookies["example.com"][0].last_access
    };
    std::thread::sleep(std::time::Duration::from_millis(5));
    jar.set_named("tok", "new");
    let after = {
        let inner = lock(&jar.inner);
        inner.cookies["example.com"][0].last_access
    };
    assert!(after > before, "set_named should bump last_access");
}

#[test]
fn merge_drops_expired_cookies() {
    // `merge` must drop expired cookies, matching `store_set_cookie`'s
    // filtering. Accepting them lets a deserialized jar whose session has
    // aged past `expires` silently keep dead cookies in the live HTTP jar.
    let live = Jar::new();
    let stale = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();
    // Past-expiry cookie via Max-Age=-1 — the parser stamps this as
    // already-expired so `merge` must skip it.
    stale.store_set_cookie("dead=yes; Path=/; Max-Age=1", &url);
    // Manually rewind the cookie's `expires` to the past.
    {
        let mut inner = lock(&stale.inner);
        if let Some(entries) = inner.cookies.get_mut("example.com") {
            for c in entries.iter_mut() {
                c.expires = Some(SystemTime::UNIX_EPOCH);
            }
        }
    }
    stale.set_cookie("https://example.com", "alive", "ok");

    live.merge(&stale);
    assert_eq!(
        live.get_cookie("https://example.com", "dead"),
        None,
        "expired cookie must not survive merge"
    );
    assert_eq!(
        live.get_cookie("https://example.com", "alive").as_deref(),
        Some("ok"),
        "non-expired cookie must survive merge"
    );
}

#[test]
fn merge_enforces_per_domain_eviction_cap() {
    // `merge` must enforce eviction; skipping it lets long-lived
    // sessions blow past Chrome's 180-cookie per-domain ceiling.
    let live = Jar::new();
    let bulk = Jar::new();
    let url = Url::parse("https://example.com/").unwrap();
    // Stuff the source jar to MAX-1 so eviction triggers on merge.
    for i in 0..(MAX_COOKIES_PER_DOMAIN + 5) {
        bulk.store_set_cookie(&format!("c{i}=v{i}; Path=/"), &url);
    }

    live.merge(&bulk);
    let count = {
        let inner = lock(&live.inner);
        inner.cookies.get("example.com").map_or(0, |v| v.len())
    };
    assert!(
        count <= MAX_COOKIES_PER_DOMAIN,
        "merge must respect per-domain cap; got {count}"
    );
}

#[test]
fn serialize_order_is_stable_across_runs() {
    // `Serialize` must emit a stable order. Emitting
    // `HashMap.values().flatten()` gives a fresh ordering every run,
    // breaking diff-based session change detection and on-disk equality
    // checks.
    let jar = Jar::new();
    // Populate across multiple domains/paths so a HashMap shuffle
    // would actually change the serialization.
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
