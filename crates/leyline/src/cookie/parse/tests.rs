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
    assert_eq!(c.same_site, SameSite::Lax);
    assert!(c.host_only);
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
    assert!(c.expires.is_some());
}

#[test]
fn psl_public_suffix_uses_real_list() {
    assert!(is_public_suffix("co.uk"));
    assert!(is_public_suffix("github.io"));
    assert!(is_public_suffix("com"));
    assert!(is_public_suffix("localhost"));
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
    assert_eq!(registrable_domain("co.uk"), None);
    assert_eq!(registrable_domain("localhost"), None);
}

#[test]
fn set_cookie_on_public_suffix_is_rejected() {
    let url = test_url("https://example.co.uk/");
    assert!(parse_set_cookie("evil=1; Domain=co.uk", &url).is_none());
    assert!(parse_set_cookie("ok=1; Domain=example.co.uk", &url).is_some());
}

#[test]
fn samesite_none_requires_secure() {
    let url = test_url("https://example.com/");
    let result = parse_set_cookie("bad=val; SameSite=None", &url);
    assert!(result.is_none());
}

#[test]
fn host_prefix_validation() {
    let url = test_url("https://example.com/");
    let c = parse_set_cookie("__Host-id=1; Secure; Path=/", &url);
    assert!(c.is_some());

    let c = parse_set_cookie("__Host-id=1; Secure; Path=/; Domain=example.com", &url);
    assert!(c.is_none());

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

#[test]
fn pre_epoch_expires_does_not_overflow() {
    let url = test_url("https://example.com/");
    let header = "session=deadbeef; Expires=Wed, 21 Oct 1013 07:28:00 GMT; Path=/";
    let _ = parse_set_cookie(header, &url);

    let t = parse_cookie_date("Thu, 01 Jan 1970 00:00:00 GMT");
    assert!(t.is_some());
    let t = parse_cookie_date("Wed, 31 Dec 1969 23:59:59 GMT");
    assert_eq!(t, Some(SystemTime::UNIX_EPOCH));
}

#[test]
fn secure_cookie_requires_secure_origin() {
    assert!(parse_set_cookie("a=1; Secure; Path=/", &test_url("http://example.com/")).is_none());
    assert!(parse_set_cookie("a=1; Secure; Path=/", &test_url("http://localhost/")).is_some());
    assert!(parse_set_cookie("a=1; Secure; Path=/", &test_url("http://127.0.0.1/")).is_some());
    assert!(parse_set_cookie("a=1; Path=/", &test_url("http://example.com/")).is_some());
}

#[test]
fn ip_host_ignores_domain_attribute() {
    let url = test_url("https://1.2.3.4/");
    let c = parse_set_cookie("k=v; Domain=3.4; Path=/", &url).expect("stored");
    assert!(c.host_only, "IP-host cookie must be host-only");
    assert_eq!(c.domain, "1.2.3.4");

    let url6 = test_url("https://[2001:db8::1]/");
    let c6 = parse_set_cookie("k=v; Domain=db8::1; Path=/", &url6).expect("stored");
    assert!(c6.host_only);
}

#[test]
fn ip_domain_cookie_does_not_reach_sibling_ip() {
    use crate::cookie::Jar;
    let jar = Jar::new();
    let a = url::Url::parse("https://1.2.3.4/").unwrap();
    jar.store_set_cookie("k=v; Domain=3.4; Path=/", &a);
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://5.6.3.4/").unwrap(), "k"),
        None
    );
    assert_eq!(
        jar.get_cookie(&url::Url::parse("https://1.2.3.4/").unwrap(), "k"),
        Some("v".into())
    );
}

#[test]
fn cookie_prefixes_are_case_insensitive() {
    let url = test_url("https://example.com/");

    assert!(parse_set_cookie("__secure-a=1; Path=/", &url).is_none());
    assert!(parse_set_cookie("__host-a=1; Path=/", &url).is_none());
    assert!(parse_set_cookie("__host-a=1; Secure; Path=/x", &url).is_none());
    assert!(parse_set_cookie("__host-a=1; Secure; Domain=example.com; Path=/", &url).is_none());

    for name in ["__Host-a", "__HOST-a", "__hOsT-a"] {
        let c = parse_set_cookie(&format!("{name}=1; Secure; Path=/"), &url)
            .expect("valid __Host- cookie stored");
        assert!(c.host_only && c.secure);
    }
}

#[test]
fn ctl_bytes_in_name_or_value_are_rejected() {
    let url = test_url("https://example.com/");
    assert!(parse_set_cookie("a=\u{1}b; Path=/", &url).is_none());
    assert!(parse_set_cookie("a=ok\u{7}value; Path=/", &url).is_none());
    assert!(parse_set_cookie("a\u{7f}=v; Path=/", &url).is_none());
    let c = parse_set_cookie("a=hello world; Path=/", &url).expect("printable value stored");
    assert_eq!(c.value, "hello world");
}
