use super::*;

fn test_url(s: &str) -> url::Url {
    url::Url::parse(s).unwrap()
}

#[test]
fn parse_basic_cookie() {
    let url = test_url("https://example.com/path");
    let c = parse_set_cookie("name=value", &url).unwrap();
    assert_eq!(c.name, "name");
    assert_eq!(c.value, "value");
    assert_eq!(c.domain, "example.com");
    assert_eq!(c.path, "/");
    assert_eq!(c.same_site, SameSite::Lax); // default
    assert!(c.host_only);
}

#[test]
fn rejected_cookie_name_value_reports_server_sent_but_unstorable() {
    // Storage-invalid Domain (RFC 6265bis §5.3.6 mismatch): the jar must
    // refuse it, yet the response view still reports what the server sent.
    let rejected = "sid=abc|1|0|def; Path=/; Max-Age=1577847600; Domain=other.example; Secure";
    let (name, value) = rejected_cookie_name_value(rejected).unwrap();
    assert_eq!(name, "sid");
    assert_eq!(value, "abc|1|0|def");

    // Quote-stripping matches the main parser (no divergence).
    let (name, value) = rejected_cookie_name_value("n=\"quoted value\"").unwrap();
    assert_eq!(name, "n");
    assert_eq!(value, "quoted value");

    // Deletions stay hidden (Max-Age=0 and negative both mean delete).
    assert!(rejected_cookie_name_value("n=v; Max-Age=0").is_none());
    assert!(rejected_cookie_name_value("n=v; Max-Age=-1").is_none());

    // Malformed headers have nothing to report.
    assert!(rejected_cookie_name_value("no-equals-sign").is_none());
    assert!(rejected_cookie_name_value("=v").is_none());
}

#[test]
fn parse_full_attributes() {
    let url = test_url("https://example.com/app/page");
    let c = parse_set_cookie(
        "tok=abc; Domain=example.com; Path=/app; Secure; HttpOnly; SameSite=None; Max-Age=3600",
        &url,
    )
    .unwrap();
    assert_eq!(c.name, "tok");
    assert_eq!(c.value, "abc");
    assert_eq!(c.domain, "example.com");
    assert_eq!(c.path, "/app");
    assert!(c.secure);
    assert!(c.http_only);
    assert_eq!(c.same_site, SameSite::None);
    assert!(!c.host_only);
    assert!(c.expires.is_some()); // from Max-Age
}

#[test]
fn psl_public_suffix_uses_real_list() {
    // Multi-label public suffixes (co.uk) resolve via the PSL.
    assert!(is_public_suffix("co.uk"));
    assert!(is_public_suffix("github.io"));
    assert!(is_public_suffix("com"));
    assert!(is_public_suffix("localhost")); // single-label
    // Registrable domains are not public suffixes.
    assert!(!is_public_suffix("example.co.uk"));
    assert!(!is_public_suffix("example.com"));
    assert!(!is_public_suffix("foo.github.io"));
}

#[test]
fn psl_registrable_domain() {
    assert_eq!(
        registrable_domain("www.example.co.uk").as_deref(),
        Some("example.co.uk")
    );
    assert_eq!(
        registrable_domain("a.b.example.com").as_deref(),
        Some("example.com")
    );
    assert_eq!(
        registrable_domain("example.com").as_deref(),
        Some("example.com")
    );
    // A bare public suffix / single label has no registrable parent.
    assert_eq!(registrable_domain("co.uk"), None);
    assert_eq!(registrable_domain("localhost"), None);
}

#[test]
fn set_cookie_on_public_suffix_is_rejected() {
    let url = test_url("https://example.co.uk/");
    // A Domain attribute pointing at the public suffix is a supercookie.
    assert!(parse_set_cookie("evil=1; Domain=co.uk", &url).is_none());
    // The registrable domain is fine.
    assert!(parse_set_cookie("ok=1; Domain=example.co.uk", &url).is_some());
}

#[test]
fn samesite_none_requires_secure() {
    let url = test_url("https://example.com/");
    let result = parse_set_cookie("bad=val; SameSite=None", &url);
    assert!(result.is_none()); // rejected
}

#[test]
fn host_prefix_validation() {
    let url = test_url("https://example.com/");
    // Valid __Host- cookie.
    let c = parse_set_cookie("__Host-id=1; Secure; Path=/", &url);
    assert!(c.is_some());

    // Invalid: __Host- with Domain.
    let c = parse_set_cookie("__Host-id=1; Secure; Path=/; Domain=example.com", &url);
    assert!(c.is_none());

    // Invalid: __Host- without Secure.
    let c = parse_set_cookie("__Host-id=1; Path=/", &url);
    assert!(c.is_none());
}

#[test]
fn max_age_caps_at_400_days() {
    let url = test_url("https://example.com/");
    let c = parse_set_cookie("x=1; Max-Age=999999999", &url).unwrap();
    let max_400_days = SystemTime::now() + Duration::from_secs(400 * 86400 + 1);
    assert!(c.expires.unwrap() < max_400_days);
}

#[test]
fn value_with_equals() {
    let url = test_url("https://example.com/");
    let c = parse_set_cookie("token=abc=def=ghi; Path=/", &url).unwrap();
    assert_eq!(c.name, "token");
    assert_eq!(c.value, "abc=def=ghi");
}

#[test]
fn cookie_date_parsing() {
    let t = parse_cookie_date("Thu, 01 Jan 2026 00:00:00 GMT");
    assert!(t.is_some());
}

/// Pre-epoch `Expires` dates must not panic: a negative
/// `days_from_epoch` cast to `u64` and multiplied by 86400
/// overflows under `debug_assertions`. Such cookies are already
/// expired and should resolve to `UNIX_EPOCH` (or be silently
/// discarded by the jar's normal expiry logic) — never panic.
#[test]
fn pre_epoch_expires_does_not_overflow() {
    let url = test_url("https://example.com/");
    // The fuzzer's minimised input:
    let header = "session=deadbeef; Expires=Wed, 21 Oct 1013 07:28:00 GMT; Path=/";
    // Must not panic. The cookie is already expired, so the jar
    // may drop it; either way we want a Result not a crash.
    let _ = parse_set_cookie(header, &url);

    // A closer edge case — exactly 1 Jan 1970 boundary.
    let t = parse_cookie_date("Thu, 01 Jan 1970 00:00:00 GMT");
    assert!(t.is_some());
    let t = parse_cookie_date("Wed, 31 Dec 1969 23:59:59 GMT");
    assert_eq!(t, Some(SystemTime::UNIX_EPOCH));
}

/// Strict Secure Cookies (Chrome 52+): a Secure cookie set from a
/// plaintext origin is ignored. Localhost is trustworthy.
#[test]
fn secure_cookie_requires_secure_origin() {
    assert!(parse_set_cookie("a=1; Secure; Path=/", &test_url("http://example.com/")).is_none());
    assert!(parse_set_cookie("a=1; Secure; Path=/", &test_url("http://localhost/")).is_some());
    assert!(parse_set_cookie("a=1; Secure; Path=/", &test_url("http://127.0.0.1/")).is_some());
    assert!(parse_set_cookie("a=1; Path=/", &test_url("http://example.com/")).is_some());
}

/// RFC 6265bis §5.5: on an IP-literal host, a `Domain` attribute is
/// ignored and the cookie is host-only. Without this, a server at
/// `1.2.3.4` could set `Domain=3.4` and broadcast cookies to every
/// raw-IP host ending in `.3.4` via the suffix match.
#[test]
fn ip_host_ignores_domain_attribute() {
    let url = test_url("https://1.2.3.4/");
    let c = parse_set_cookie("k=v; Domain=3.4; Path=/", &url).expect("stored");
    assert!(c.host_only, "IP-host cookie must be host-only");
    assert_eq!(c.domain, "1.2.3.4");

    // IPv6 literal, same rule (url wraps the host in brackets).
    let url6 = test_url("https://[2001:db8::1]/");
    let c6 = parse_set_cookie("k=v; Domain=db8::1; Path=/", &url6).expect("stored");
    assert!(c6.host_only);
}

/// The jar-level consequence of `ip_host_ignores_domain_attribute`:
/// a cookie set on one raw-IP host (even with a crafted `Domain=`)
/// is never visible on a sibling IP host.
#[test]
fn ip_domain_cookie_does_not_reach_sibling_ip() {
    use crate::cookie::Jar;
    let jar = Jar::new();
    let a = url::Url::parse("https://1.2.3.4/").unwrap();
    jar.store_set_cookie("k=v; Domain=3.4; Path=/", &a);
    assert_eq!(jar.get_cookie("https://5.6.3.4/", "k"), None);
    assert_eq!(jar.get_cookie("https://1.2.3.4/", "k"), Some("v".into()));
}

/// RFC 6265bis §4.1.3: `__Secure-`/`__Host-` prefix matching is
/// case-insensitive. A hostile server must not dodge the prefix
/// rules by varying the case.
#[test]
fn cookie_prefixes_are_case_insensitive() {
    let url = test_url("https://example.com/");

    // Lowercase `__secure-` without Secure → rejected.
    assert!(parse_set_cookie("__secure-a=1; Path=/", &url).is_none());
    // Lowercase `__host-` without Secure or with a path → rejected.
    assert!(parse_set_cookie("__host-a=1; Path=/", &url).is_none());
    assert!(parse_set_cookie("__host-a=1; Secure; Path=/x", &url).is_none());
    assert!(parse_set_cookie("__host-a=1; Secure; Domain=example.com; Path=/", &url).is_none());

    // Uppercase/lowercase spellings of a VALID `__Host-` cookie pass.
    for name in ["__Host-a", "__HOST-a", "__hOsT-a"] {
        let c = parse_set_cookie(&format!("{name}=1; Secure; Path=/"), &url)
            .expect("valid __Host- cookie stored");
        assert!(c.host_only && c.secure);
    }
}

/// RFC 6265bis §5.6: control characters in a cookie name or value
/// must be rejected at parse. A CTL entering the jar would be echoed
/// on later requests and rejected by the H1 send-path validation,
/// breaking the client's own requests to that server.
#[test]
fn ctl_bytes_in_name_or_value_are_rejected() {
    let url = test_url("https://example.com/");
    assert!(parse_set_cookie("a=\u{1}b; Path=/", &url).is_none());
    assert!(parse_set_cookie("a=ok\u{7}value; Path=/", &url).is_none());
    assert!(parse_set_cookie("a\u{7f}=v; Path=/", &url).is_none());
    // Printable ASCII (including spaces and quotes) still parses.
    let c = parse_set_cookie("a=hello world; Path=/", &url).expect("printable value stored");
    assert_eq!(c.value, "hello world");
}
