use crate::deadline::Timeout;
use crate::net::ProxyUrl;
use std::time::Duration;

use super::proxy::NoProxy;
use super::*;
use crate::profile::Encoding;

fn url_for(host: &str) -> url::Url {
    url::Url::parse(&format!("http://{host}/")).unwrap()
}

#[test]
fn no_proxy_matches_domains_and_literals() {
    let no_proxy = NoProxy::from_string(".example.com,127.0.0.1,[::1]:8080").unwrap();
    assert!(no_proxy.matches_url(&url_for("api.example.com")));
    assert!(no_proxy.matches_url(&url_for("127.0.0.1")));
    assert!(no_proxy.matches_url(&url_for("[::1]:8080")));
    assert!(!no_proxy.matches_url(&url_for("[::1]:9090")));
    assert!(!no_proxy.matches_url(&url_for("other.test")));
}

#[test]
fn no_proxy_ipv6_literal_matches_bare_host() {
    assert!(
        NoProxy::from_string("::1")
            .unwrap()
            .matches_url(&url_for("[::1]"))
    );
    assert!(
        NoProxy::from_string("[::1]")
            .unwrap()
            .matches_url(&url_for("[::1]"))
    );
    assert!(
        NoProxy::from_string("[::1]:8080")
            .unwrap()
            .matches_url(&url_for("[::1]:8080"))
    );
    assert!(
        NoProxy::from_string("fe80::1")
            .unwrap()
            .matches_url(&url_for("[fe80::1]"))
    );
    assert!(
        NoProxy::from_string("2001:db8::1,192.0.2.0")
            .unwrap()
            .matches_url(&url_for("[2001:db8::1]"))
    );
}

#[test]
fn no_proxy_ipv6_bracketed_host_matches_pattern() {
    assert!(
        NoProxy::from_string("::1")
            .unwrap()
            .matches_url(&url_for("[::1]"))
    );
    assert!(
        NoProxy::from_string("[::1]")
            .unwrap()
            .matches_url(&url_for("[::1]"))
    );
}

#[test]
fn no_proxy_port_matching_and_overflow_rejection() {
    assert!(
        NoProxy::from_string("192.0.2.1:8080")
            .unwrap()
            .matches_url(&url_for("192.0.2.1:8080"))
    );
    assert!(
        !NoProxy::from_string("192.0.2.1:8080")
            .unwrap()
            .matches_url(&url_for("192.0.2.1:9090"))
    );
    assert!(
        !NoProxy::from_string("127.0.0.1:99999")
            .unwrap()
            .matches_url(&url_for("127.0.0.1:8080"))
    );
    assert!(
        !NoProxy::from_string("[::1]:99999")
            .unwrap()
            .matches_url(&url_for("[::1]:8080"))
    );
    assert!(
        NoProxy::from_string("example.com:443")
            .unwrap()
            .matches_url(&url::Url::parse("https://example.com/").unwrap())
    );
    assert!(
        NoProxy::from_string(".example.com:443")
            .unwrap()
            .matches_url(&url::Url::parse("https://sub.example.com/").unwrap())
    );
}

#[test]
fn no_proxy_does_not_match_unrelated_ipv6() {
    assert!(
        !NoProxy::from_string("::1")
            .unwrap()
            .matches_url(&url_for("[::2]"))
    );
    assert!(
        !NoProxy::from_string("2001:db8::1")
            .unwrap()
            .matches_url(&url_for("[2001:db8::2]"))
    );
}

#[test]
fn no_proxy_port_and_ipv6_canonicalization_and_ipv4_suffix_guard() {
    let url_match = url::Url::parse("http://localhost:8080/a").unwrap();
    let url_other = url::Url::parse("http://localhost:9999/a").unwrap();
    let np = NoProxy::from_string("localhost:8080,[2001:0db8::1]:8443,2.1").unwrap();
    assert!(np.matches_url(&url_match));
    assert!(!np.matches_url(&url_other));

    let v6_canonical = url::Url::parse("https://[2001:db8::1]:8443/").unwrap();
    let v6_other_port = url::Url::parse("https://[2001:db8::1]:443/").unwrap();
    assert!(np.matches_url(&v6_canonical));
    assert!(!np.matches_url(&v6_other_port));

    let v4 = url::Url::parse("http://192.0.2.1/").unwrap();
    assert!(!np.matches_url(&v4));

    assert_eq!(crate::net::normalize_host("[2001:0db8::1]"), "2001:db8::1");
    assert_eq!(crate::net::normalize_host("[0:0:0:0:0:0:0:1]"), "::1");
    assert!(!crate::util::domain_match("192.0.2.1", "2.1"));
}

#[test]
fn proxy_scheme_rules_override_all_and_replace_prior_scheme() {
    let http_url = url::Url::parse("http://example.com/").unwrap();
    let https_url = url::Url::parse("https://example.com/").unwrap();

    let p = Proxy::new()
        .all("http://all:1")
        .https("http://tls:2")
        .http("http://old:3")
        .http("http://new:4");
    assert_eq!(
        p.proxy_for(&http_url).unwrap().unwrap().url().as_str(),
        "http://new:4/"
    );
    assert_eq!(
        p.proxy_for(&https_url).unwrap().unwrap().url().as_str(),
        "http://tls:2/"
    );

    let p2 = Proxy::new().all("http://all:1").https("http://tls:2");
    assert_eq!(
        p2.proxy_for(&http_url).unwrap().unwrap().url().as_str(),
        "http://all:1/"
    );
    assert_eq!(
        p2.proxy_for(&https_url).unwrap().unwrap().url().as_str(),
        "http://tls:2/"
    );
}

fn cfg_with_env_no_proxy(patterns: &str) -> Proxy {
    Proxy {
        no_proxy: NoProxy::from_string(patterns).unwrap(),
        use_env: true,
        ..Proxy::default()
    }
}

#[test]
fn env_no_proxy_never_bypasses_explicit_proxies() {
    let url = url::Url::parse("https://target.test/x").unwrap();
    assert_eq!(
        Proxy::from("http://req:1")
            .proxy_for(&url)
            .unwrap()
            .as_deref()
            .map(ProxyUrl::as_str),
        Some("http://req:1"),
        "env NO_PROXY bypassed a per-request proxy override"
    );
    let sess = cfg_with_env_no_proxy("target.test").all("http://sess:1");
    assert_eq!(
        sess.proxy_for(&url)
            .unwrap()
            .as_deref()
            .map(ProxyUrl::as_str),
        Some("http://sess:1"),
        "env NO_PROXY bypassed an explicit session proxy"
    );
    let cfg = cfg_with_env_no_proxy("target.test").all("http://rule:1");
    assert_eq!(
        cfg.proxy_for(&url)
            .unwrap()
            .as_deref()
            .map(ProxyUrl::as_str),
        Some("http://rule:1"),
        "env NO_PROXY bypassed an explicit proxy rule"
    );
}

#[test]
fn env_no_proxy_bypasses_env_derived_proxy() {
    let cfg = cfg_with_env_no_proxy("target.test")
        .all("http://env:1")
        .set_from_env();
    let url = url::Url::parse("https://target.test/x").unwrap();
    assert_eq!(
        cfg.proxy_for(&url)
            .unwrap()
            .as_deref()
            .map(ProxyUrl::as_str),
        None
    );
    let other = url::Url::parse("https://other.test/x").unwrap();
    assert_eq!(
        cfg.proxy_for(&other)
            .unwrap()
            .as_deref()
            .map(ProxyUrl::as_str),
        Some("http://env:1")
    );
}

#[test]
fn explicit_no_proxy_bypasses_all_proxies() {
    let cfg = Proxy::new().no_proxy(["target.test"]);
    let url = url::Url::parse("https://target.test/x").unwrap();
    assert_eq!(
        cfg.proxy_for(&url)
            .unwrap()
            .as_deref()
            .map(ProxyUrl::as_str),
        None
    );
    assert_eq!(
        cfg.clone().all("http://sess:1").proxy_for(&url).unwrap(),
        None
    );
    assert_eq!(
        cfg.all("http://rule:1")
            .proxy_for(&url)
            .unwrap()
            .as_deref()
            .map(ProxyUrl::as_str),
        None
    );
}

#[test]
fn compression_none_disables_known_codecs() {
    let mut cfg = BodyRules::default();
    cfg.offer([]);
    assert!(!cfg.allows(Encoding::Gzip));
    assert!(!cfg.allows(Encoding::Brotli));
    assert!(Codings::of([&http::HeaderValue::from_static("identity")]).is_identity());
}

#[test]
fn default_timeout_matches_browser_scale_patience() {
    assert_eq!(
        Timeout::default().total_limit(),
        Some(Duration::from_secs(300))
    );
}

#[test]
fn proxy_url_validates_supported_schemes_and_hosts() {
    assert_eq!(
        ProxyUrl::parse(" http://proxy.example:8080 ")
            .unwrap()
            .as_str(),
        "http://proxy.example:8080"
    );
    ProxyUrl::parse("ftp://proxy.example:21").expect_err("expected Err");
    ProxyUrl::parse("http://").expect_err("expected Err");
    assert_eq!(
        ProxyUrl::parse("https://proxy.example:8443")
            .unwrap()
            .as_str(),
        "https://proxy.example:8443"
    );
}

#[test]
fn proxy_debug_redacts_password() {
    let rule = ProxyRule::new(
        super::proxy::ProxyRuleScheme::All,
        "socks5://user:hunter2@proxy.example:1080",
        &[],
    );
    let shown = format!("{rule:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(shown.contains("user:***@proxy.example:1080"), "{shown}");
    let url = ProxyUrl::parse("http://u:secret@h:8080").unwrap();
    assert!(!format!("{url:?}").contains("secret"));
    assert_eq!(url.as_str(), "http://u:secret@h:8080");
}

#[test]
fn a_padded_proxy_url_is_shown_as_parsed() {
    let session = crate::Session::builder()
        .proxy(" http://proxy.example:8080 ")
        .build()
        .unwrap();
    assert_eq!(
        session.proxy().map(url::Url::as_str),
        Some("http://proxy.example:8080/")
    );
}

#[test]
fn proxy_same_for_doh_checks_all_fields() {
    let base = Proxy::new()
        .all("http://proxy.example:8080")
        .header("x-key", "1")
        .no_proxy(["internal.local"]);
    assert!(base.same_for_doh(&base.clone()));
    assert!(
        !base.same_for_doh(
            &Proxy::new()
                .all("http://other.example:8080")
                .header("x-key", "1")
                .no_proxy(["internal.local"])
        )
    );
    assert!(
        !base.same_for_doh(
            &Proxy::new()
                .all("http://proxy.example:8080")
                .header("x-key", "2")
                .no_proxy(["internal.local"])
        )
    );
    assert!(
        !base.same_for_doh(
            &Proxy::new()
                .all("http://proxy.example:8080")
                .header("x-key", "1")
                .no_proxy(["other.local"])
        )
    );
    assert!(
        !base.same_for_doh(
            &Proxy::new()
                .all("http://proxy.example:8080")
                .header("x-key", "1")
        )
    );
    assert!(!Proxy::new().env(false).same_for_doh(&Proxy::new()));

    let custom_a = Proxy::custom(|_| None);
    let custom_b = Proxy::custom(|_| None);
    assert!(custom_a.same_for_doh(&custom_a.clone()));
    assert!(!custom_a.same_for_doh(&custom_b));
    assert!(!custom_a.same_for_doh(&Proxy::new()));
}

#[test]
fn proxy_pool_bypasses_all_requires_non_empty_and_every_entry_bypassed() {
    use crate::core::proxy_pool::ProxyPool;
    let empty = ProxyPool::new(Vec::<Proxy>::new());
    assert!(!empty.bypasses_all_url(&url_for("example.com")));

    let single = ProxyPool::new([Proxy::new()
        .all("http://127.0.0.1:8080")
        .no_proxy(["skip.local"])]);
    assert!(single.bypasses_all_url(&url_for("skip.local")));
    assert!(!single.bypasses_all_url(&url_for("keep.local")));

    let mixed = ProxyPool::new([
        Proxy::new()
            .all("http://127.0.0.1:8080")
            .no_proxy(["skip.local"]),
        Proxy::new().all("http://127.0.0.1:8081"),
    ]);
    assert!(!mixed.bypasses_all_url(&url_for("skip.local")));
}

#[test]
fn proxy_pool_same_for_doh_checks_len_and_each_entry() {
    use crate::core::proxy_pool::ProxyPool;
    let p1 = Proxy::new().all("http://127.0.0.1:8080");
    let p2 = Proxy::new().all("http://127.0.0.1:8081");
    let one_a = ProxyPool::new([p1.clone()]);
    let one_a_copy = ProxyPool::new([p1.clone()]);
    let one_a_ban = ProxyPool::new([p1.clone()]).ban_after(1);
    let one_b = ProxyPool::new([p2.clone()]);
    let two = ProxyPool::new([p1, p2]);
    assert!(one_a.same_for_doh(&one_a_copy));
    assert!(!one_a.same_for_doh(&one_a_ban));
    assert!(!one_a.same_for_doh(&one_b));
    assert!(!one_a.same_for_doh(&two));
}

#[tokio::test]
async fn stats_host_lookup_matches_standard_ports_and_round_trips_hosts() {
    let gates = HostGates::new(HostLimits::new().max_in_flight(2));
    let https_url = url::Url::parse("https://example.com/a").expect("valid https url");
    let http_url = url::Url::parse("http://example.org/b").expect("valid http url");
    let _p1 = gates
        .admit_with(None, &https_url)
        .await
        .expect("admit https");
    let _p2 = gates.admit_with(None, &http_url).await.expect("admit http");

    let stats = crate::core::Stats::new(
        crate::pool::ConnectionStats::default(),
        gates.stats(),
        Vec::new(),
    );
    assert_eq!(stats.in_flight("https://example.com/"), 1);
    assert_eq!(stats.in_flight("https://example.com:443/other"), 1);
    assert_eq!(stats.in_flight("http://example.org/"), 1);
    assert_eq!(stats.in_flight("http://example.org:80/other"), 1);
    let hosts: Vec<&str> = stats.hosts().collect();
    assert_eq!(hosts.len(), 2);
    for host in hosts {
        assert_eq!(stats.in_flight(host), 1, "round-trip failed for {host}");
    }
}

#[cfg(feature = "html")]
#[tokio::test]
async fn host_gates_ready_with_preserves_delay_after_lru_eviction() {
    let gates = HostGates::new(HostLimits::new());
    let first_url = url::Url::parse("https://site0.example/").expect("valid first url");
    let first_origin = Origin::of(&first_url).expect("first origin");
    let delay = Duration::from_secs(60);
    gates.set_floor_origin(first_origin.clone(), Some(delay));
    let _pass = gates
        .admit_with(None, &first_url)
        .await
        .expect("admit first site");

    for idx in 1..=1100 {
        let other =
            url::Url::parse(&format!("https://site{idx}.example/")).expect("valid site url");
        let origin = Origin::of(&other).expect("site origin");
        gates.set_floor_origin(origin, Some(Duration::from_secs(5)));
    }

    assert!(!gates.ready_with(&first_origin, Some(delay)));
    assert!(gates.ready_at_with(&first_origin, Some(delay)).is_some());
}
