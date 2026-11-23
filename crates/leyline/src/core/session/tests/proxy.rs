use super::super::proxy::{CGI_SIGNAL_ENV_VARS, InvalidEnvProxy};
use crate::core::config::Proxy;

use std::collections::HashMap;

fn mock_env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    let map: HashMap<&str, &str> = pairs.iter().copied().collect();
    move |k: &str| map.get(k).map(|s| s.to_string())
}
fn mock_env_has<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> bool + 'a {
    let map: HashMap<&str, &str> = pairs.iter().copied().collect();
    move |k: &str| map.contains_key(k)
}

fn resolved_for(pairs: &[(&str, &str)], url: &str) -> Option<String> {
    let proxy = Proxy::new()
        .apply_env_proxy_from(
            InvalidEnvProxy::FailBuild,
            mock_env(pairs),
            mock_env_has(pairs),
        )
        .unwrap();
    let parsed = url::Url::parse(url).unwrap();
    proxy
        .proxy_for(&parsed)
        .unwrap()
        .map(|p| p.as_str().to_owned())
}

#[test]
fn env_proxy_no_env_returns_none() {
    assert_eq!(resolved_for(&[], "https://target.test/"), None);
    assert_eq!(resolved_for(&[], "http://target.test/"), None);
}

#[test]
fn env_proxy_prefers_https_proxy_upper() {
    let pairs = &[
        ("HTTPS_PROXY", "http://up.example:3128"),
        ("https_proxy", "http://lo.example:3128"),
        ("HTTP_PROXY", "http://httpup.example:3128"),
    ];
    assert_eq!(
        resolved_for(pairs, "https://target.test/"),
        Some("http://up.example:3128".to_string())
    );
    assert_eq!(
        resolved_for(pairs, "http://target.test/"),
        Some("http://httpup.example:3128".to_string())
    );
}

#[test]
fn env_proxy_honours_http_proxy_when_no_cgi() {
    let pairs = &[("HTTP_PROXY", "http://ok.example:3128")];
    assert_eq!(
        resolved_for(pairs, "http://target.test/"),
        Some("http://ok.example:3128".to_string())
    );
}

#[test]
fn env_proxy_trims_whitespace_and_skips_empty() {
    let pairs = &[
        ("HTTPS_PROXY", "   "),
        ("https_proxy", ""),
        ("HTTP_PROXY", "  http://trim.example:3128  "),
    ];
    assert_eq!(
        resolved_for(pairs, "http://target.test/"),
        Some("http://trim.example:3128".to_string())
    );
    assert_eq!(resolved_for(pairs, "https://target.test/"), None);
}

#[test]
fn env_proxy_uses_all_proxy_as_fallback() {
    let pairs = &[("ALL_PROXY", "socks5://proxy.example:1080")];
    assert_eq!(
        resolved_for(pairs, "https://target.test/"),
        Some("socks5://proxy.example:1080".to_string())
    );
    assert_eq!(
        resolved_for(pairs, "http://target.test/"),
        Some("socks5://proxy.example:1080".to_string())
    );
}

#[test]
fn env_proxy_prefers_scheme_specific_proxy_over_all_proxy() {
    let pairs = &[
        ("ALL_PROXY", "socks5://fallback.example:1080"),
        ("HTTPS_PROXY", "http://https.example:3128"),
    ];
    assert_eq!(
        resolved_for(pairs, "https://target.test/"),
        Some("http://https.example:3128".to_string())
    );
    assert_eq!(
        resolved_for(pairs, "http://target.test/"),
        Some("socks5://fallback.example:1080".to_string())
    );
}

#[test]
fn env_proxy_ignores_http_proxy_under_cgi() {
    for signal in CGI_SIGNAL_ENV_VARS {
        let pairs = &[
            (*signal, "set"),
            ("HTTP_PROXY", "http://evil.attacker:3128"),
        ];
        let v = resolved_for(pairs, "http://target.test/");
        assert_eq!(
            v, None,
            "CGI signal {signal} should have blocked HTTP_PROXY but got {v:?}"
        );
    }
}

#[test]
fn env_proxy_under_cgi_still_honours_https_proxy() {
    let pairs = &[
        ("GATEWAY_INTERFACE", "CGI/1.1"),
        ("HTTPS_PROXY", "http://legit.example:3128"),
        ("HTTP_PROXY", "http://evil.attacker:3128"),
    ];
    assert_eq!(
        resolved_for(pairs, "https://target.test/"),
        Some("http://legit.example:3128".to_string())
    );
    assert_eq!(resolved_for(pairs, "http://target.test/"), None);
}

#[test]
fn env_proxy_skips_present_but_unreadable_vars() {
    let get_var = |_k: &str| -> Option<String> { None };
    let has_var = |k: &str| matches!(k, "HTTPS_PROXY" | "HTTP_PROXY");
    let proxy = Proxy::new()
        .apply_env_proxy_from(InvalidEnvProxy::FailBuild, get_var, has_var)
        .unwrap();
    let url = url::Url::parse("https://target.test/").unwrap();
    assert!(proxy.proxy_for(&url).unwrap().is_none());
}

#[test]
fn env_proxy_under_cgi_with_unreadable_http_proxy() {
    let get_var = |_k: &str| -> Option<String> { None };
    let has_var = |k: &str| matches!(k, "GATEWAY_INTERFACE" | "HTTP_PROXY");
    let proxy = Proxy::new()
        .apply_env_proxy_from(InvalidEnvProxy::FailBuild, get_var, has_var)
        .unwrap();
    let url = url::Url::parse("http://target.test/").unwrap();
    assert!(proxy.proxy_for(&url).unwrap().is_none());
}

#[test]
fn env_proxy_under_cgi_with_unreadable_http_proxy_but_readable_https() {
    let legit = "http://legit.example:3128";
    let get_var = move |k: &str| -> Option<String> {
        if k == "HTTPS_PROXY" {
            Some(legit.to_string())
        } else {
            None
        }
    };
    let has_var = |k: &str| matches!(k, "GATEWAY_INTERFACE" | "HTTPS_PROXY" | "HTTP_PROXY");
    let proxy = Proxy::new()
        .apply_env_proxy_from(InvalidEnvProxy::FailBuild, get_var, has_var)
        .unwrap();
    let url = url::Url::parse("https://target.test/").unwrap();
    assert_eq!(
        proxy
            .proxy_for(&url)
            .unwrap()
            .map(|p| p.as_str().to_owned()),
        Some(legit.to_string())
    );
}

#[test]
fn env_proxy_under_cgi_still_honours_lowercase_http_proxy() {
    let pairs = &[
        ("REQUEST_METHOD", "GET"),
        ("http_proxy", "http://user.example:3128"),
        ("HTTP_PROXY", "http://evil.attacker:3128"),
    ];
    assert_eq!(
        resolved_for(pairs, "http://target.test/"),
        Some("http://user.example:3128".to_string())
    );
}

#[test]
fn apply_env_proxy_ignores_unused_invalid_all_proxy_when_both_scheme_vars_set() {
    let pairs = &[
        ("HTTP_PROXY", "http://plain.example:3128"),
        ("HTTPS_PROXY", "http://secure.example:8443"),
        ("ALL_PROXY", "http://[::1"),
    ];
    assert_eq!(
        resolved_for(pairs, "http://api.example/v1"),
        Some("http://plain.example:3128".to_string())
    );
    assert_eq!(
        resolved_for(pairs, "https://api.example/v1"),
        Some("http://secure.example:8443".to_string())
    );
}

#[test]
fn apply_env_proxy_routes_http_https_and_all_independently() {
    use super::super::proxy::InvalidEnvProxy;
    use crate::core::config::Proxy;

    let pairs = &[
        ("ALL_PROXY", "http://all.example:1080"),
        ("HTTP_PROXY", "http://plain.example:3128"),
        ("HTTPS_PROXY", "http://secure.example:8443"),
        ("NO_PROXY", "127.0.0.1:8080,.internal.test"),
    ];
    let proxy = Proxy::new()
        .apply_env_proxy_from(
            InvalidEnvProxy::FailBuild,
            mock_env(pairs),
            mock_env_has(pairs),
        )
        .unwrap();

    let http_url = url::Url::parse("http://api.example/v1").unwrap();
    let https_url = url::Url::parse("https://api.example/v1").unwrap();
    let bypassed_port = url::Url::parse("http://127.0.0.1:8080/ok").unwrap();
    let other_port = url::Url::parse("http://127.0.0.1:9090/ok").unwrap();

    assert_eq!(
        proxy.proxy_for(&http_url).unwrap().unwrap().url().as_str(),
        "http://plain.example:3128/"
    );
    assert_eq!(
        proxy.proxy_for(&https_url).unwrap().unwrap().url().as_str(),
        "http://secure.example:8443/"
    );
    assert!(proxy.proxy_for(&bypassed_port).unwrap().is_none());
    assert_eq!(
        proxy
            .proxy_for(&other_port)
            .unwrap()
            .unwrap()
            .url()
            .as_str(),
        "http://plain.example:3128/"
    );
}

#[test]
fn apply_env_proxy_fails_closed_when_any_scheme_var_is_invalid() {
    use super::super::proxy::InvalidEnvProxy;
    use crate::core::config::Proxy;

    let pairs = &[
        ("HTTPS_PROXY", "http://good.example:3128"),
        ("HTTP_PROXY", "http://[::1"),
    ];
    let err = Proxy::new()
        .apply_env_proxy_from(
            InvalidEnvProxy::FailBuild,
            mock_env(pairs),
            mock_env_has(pairs),
        )
        .unwrap_err();
    assert_eq!(err.kind(), crate::error::Kind::Config);
}
