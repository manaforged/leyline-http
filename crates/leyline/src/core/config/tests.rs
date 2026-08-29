use super::*;

#[test]
fn no_proxy_matches_domains_and_literals() {
    let no_proxy = NoProxy::from_string(".example.com,127.0.0.1,[::1]:8080").unwrap();
    assert!(no_proxy.matches("api.example.com"));
    assert!(no_proxy.matches("127.0.0.1"));
    assert!(no_proxy.matches("[::1]"));
    assert!(!no_proxy.matches("other.test"));
}

#[test]
fn no_proxy_ipv6_literal_matches_bare_host() {
    // Naive `split(':')` truncation collapsed `::1` to an empty
    // string, silently breaking loopback bypass for IPv6. The
    // bracketed and bare pattern forms must match a bracketless
    // IPv6 URL host.
    assert!(NoProxy::from_string("::1").unwrap().matches("::1"));
    assert!(NoProxy::from_string("[::1]").unwrap().matches("::1"));
    assert!(NoProxy::from_string("[::1]:8080").unwrap().matches("::1"));
    assert!(NoProxy::from_string("fe80::1").unwrap().matches("fe80::1"));
    assert!(
        NoProxy::from_string("2001:db8::1,192.0.2.0")
            .unwrap()
            .matches("2001:db8::1")
    );
}

#[test]
fn no_proxy_ipv6_bracketed_host_matches_pattern() {
    // `url::Host::parse` rejects brackets; the normaliser strips
    // them so `[::1]` on either side compares equal to `::1`.
    assert!(NoProxy::from_string("::1").unwrap().matches("[::1]"));
    assert!(NoProxy::from_string("[::1]").unwrap().matches("[::1]"));
}

#[test]
fn no_proxy_ipv4_port_stripping_still_works() {
    assert!(
        NoProxy::from_string("192.0.2.1:8080")
            .unwrap()
            .matches("192.0.2.1")
    );
    assert!(
        NoProxy::from_string("example.com:443")
            .unwrap()
            .matches("example.com")
    );
    assert!(
        NoProxy::from_string(".example.com:443")
            .unwrap()
            .matches("sub.example.com")
    );
}

#[test]
fn no_proxy_does_not_match_unrelated_ipv6() {
    // Critical negative: `::1` must NOT match `::2`. Guard against
    // bare IPv6 patterns collapsing to a value that matches every
    // IPv6 host.
    assert!(!NoProxy::from_string("::1").unwrap().matches("::2"));
    assert!(
        !NoProxy::from_string("2001:db8::1")
            .unwrap()
            .matches("2001:db8::2")
    );
}

// ── no-proxy provenance gates ──────────────
//
// Env-inherited NO_PROXY may only bypass env-discovered proxies. A
// stray NO_PROXY on the box silently turning explicitly-proxied
// traffic DIRECT is a real-IP leak, not a convenience.

/// A config whose `no_proxy` came from the environment (not the
/// `.no_proxy()` builder).
fn cfg_with_env_no_proxy(patterns: &str) -> ProxyConfig {
    ProxyConfig {
        rules: Vec::new(),
        no_proxy: NoProxy::from_string(patterns).unwrap(),
        no_proxy_explicit: false,
        use_env: true,
    }
}

#[test]
fn env_no_proxy_never_bypasses_explicit_proxies() {
    let cfg = cfg_with_env_no_proxy("target.test");
    let url = url::Url::parse("https://target.test/x").unwrap();
    assert_eq!(
        cfg.proxy_for(&url, Some("http://req:1"), None, false),
        Some("http://req:1"),
        "env NO_PROXY bypassed a per-request proxy override"
    );
    assert_eq!(
        cfg.proxy_for(&url, None, Some("http://sess:1"), false),
        Some("http://sess:1"),
        "env NO_PROXY bypassed an explicit session proxy"
    );
    let cfg = cfg_with_env_no_proxy("target.test").all("http://rule:1");
    assert_eq!(
        cfg.proxy_for(&url, None, None, false),
        Some("http://rule:1"),
        "env NO_PROXY bypassed an explicit proxy rule"
    );
}

#[test]
fn env_no_proxy_bypasses_env_derived_proxy() {
    let cfg = cfg_with_env_no_proxy("target.test");
    let url = url::Url::parse("https://target.test/x").unwrap();
    assert_eq!(cfg.proxy_for(&url, None, Some("http://env:1"), true), None);
    // Non-matching hosts still go through the env proxy.
    let other = url::Url::parse("https://other.test/x").unwrap();
    assert_eq!(
        cfg.proxy_for(&other, None, Some("http://env:1"), true),
        Some("http://env:1")
    );
}

#[test]
fn explicit_no_proxy_bypasses_all_proxies() {
    // Set via the builder method → deliberate config → gates
    // everything, per-request overrides included (curl --noproxy
    // semantics, and the pre-fix behaviour for explicit users).
    let cfg = ProxyConfig::new().no_proxy(NoProxy::from_string("target.test").unwrap());
    let url = url::Url::parse("https://target.test/x").unwrap();
    assert_eq!(cfg.proxy_for(&url, Some("http://req:1"), None, false), None);
    assert_eq!(
        cfg.proxy_for(&url, None, Some("http://sess:1"), false),
        None
    );
    assert_eq!(
        cfg.all("http://rule:1").proxy_for(&url, None, None, false),
        None
    );
}

#[test]
fn compression_none_disables_known_codecs() {
    let cfg = CompressionConfig::none();
    assert!(!cfg.allows("gzip"));
    assert!(!cfg.allows("br"));
    assert!(cfg.allows("identity"));
}

#[test]
fn default_timeout_matches_browser_scale_patience() {
    assert_eq!(TimeoutConfig::default().total, Duration::from_secs(300));
}

#[test]
fn proxy_url_validates_supported_schemes_and_hosts() {
    assert_eq!(
        ProxyUrl::parse(" http://proxy.example:8080 ")
            .unwrap()
            .as_str(),
        "http://proxy.example:8080"
    );
    assert!(ProxyUrl::parse("ftp://proxy.example:21").is_err());
    assert!(ProxyUrl::parse("http://").is_err());
    assert_eq!(
        ProxyUrl::parse("https://proxy.example:8443")
            .unwrap()
            .as_str(),
        "https://proxy.example:8443"
    );
}

#[test]
fn proxy_url_constructors_enforce_scheme() {
    assert!(ProxyUrl::http("http://proxy.example:8080").is_ok());
    assert!(ProxyUrl::https("https://proxy.example:8443").is_ok());
    assert!(ProxyUrl::socks5("socks5://proxy.example:1080").is_ok());
    assert!(ProxyUrl::socks5h("socks5h://proxy.example:1080").is_ok());
    assert!(ProxyUrl::http("socks5://proxy.example:1080").is_err());
    assert!(ProxyUrl::https("http://proxy.example:8080").is_err());
    assert!(ProxyUrl::socks5("http://proxy.example:8080").is_err());
}
