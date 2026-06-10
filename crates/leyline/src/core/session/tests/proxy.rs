use super::super::proxy::{env_proxy_from, CGI_SIGNAL_ENV_VARS};

// NO_PROXY host-matching gates live next to `NoProxy` in
// `core::config` (the dead duplicate matcher was removed).

// ---- httpoxy regression gates ----
//
// The env_proxy logic takes
// getters as parameters so we can test it without mutating
// `std::env`. If a refactor ever re-couples this to the
// process environment, these tests should scream first.

use std::collections::HashMap;

fn mock_env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    let map: HashMap<&str, &str> = pairs.iter().copied().collect();
    move |k: &str| map.get(k).map(|s| s.to_string())
}
fn mock_env_has<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> bool + 'a {
    let map: HashMap<&str, &str> = pairs.iter().copied().collect();
    move |k: &str| map.contains_key(k)
}

#[test]
fn env_proxy_no_env_returns_none() {
    assert_eq!(env_proxy_from(mock_env(&[]), mock_env_has(&[])), None);
}

#[test]
fn env_proxy_prefers_https_proxy_upper() {
    let pairs = &[
        ("HTTPS_PROXY", "http://up.example:3128"),
        ("https_proxy", "http://lo.example:3128"),
        ("HTTP_PROXY", "http://httpup.example:3128"),
    ];
    let v = env_proxy_from(mock_env(pairs), mock_env_has(pairs));
    assert_eq!(v, Some("http://up.example:3128".to_string()));
}

#[test]
fn env_proxy_honours_http_proxy_when_no_cgi() {
    let pairs = &[("HTTP_PROXY", "http://ok.example:3128")];
    let v = env_proxy_from(mock_env(pairs), mock_env_has(pairs));
    assert_eq!(v, Some("http://ok.example:3128".to_string()));
}

#[test]
fn env_proxy_trims_whitespace_and_skips_empty() {
    let pairs = &[
        ("HTTPS_PROXY", "   "),
        ("https_proxy", ""),
        ("HTTP_PROXY", "  http://trim.example:3128  "),
    ];
    let v = env_proxy_from(mock_env(pairs), mock_env_has(pairs));
    assert_eq!(v, Some("http://trim.example:3128".to_string()));
}

#[test]
fn env_proxy_uses_all_proxy_as_fallback() {
    let pairs = &[("ALL_PROXY", "socks5://proxy.example:1080")];
    let v = env_proxy_from(mock_env(pairs), mock_env_has(pairs));
    assert_eq!(v, Some("socks5://proxy.example:1080".to_string()));
}

#[test]
fn env_proxy_prefers_scheme_specific_proxy_over_all_proxy() {
    let pairs = &[
        ("ALL_PROXY", "socks5://fallback.example:1080"),
        ("HTTPS_PROXY", "http://https.example:3128"),
    ];
    let v = env_proxy_from(mock_env(pairs), mock_env_has(pairs));
    assert_eq!(v, Some("http://https.example:3128".to_string()));
}

/// httpoxy: when any CGI-style variable is set AND `HTTP_PROXY`
/// is also set, `HTTP_PROXY` MUST be ignored. `HTTPS_PROXY` is
/// not spoofable via HTTP request headers (no `Https-Proxy:`
/// header mapping exists) and is still honoured.
#[test]
fn env_proxy_ignores_http_proxy_under_cgi() {
    for signal in CGI_SIGNAL_ENV_VARS {
        let pairs = &[
            (*signal, "set"),
            ("HTTP_PROXY", "http://evil.attacker:3128"),
        ];
        let v = env_proxy_from(mock_env(pairs), mock_env_has(pairs));
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
    let v = env_proxy_from(mock_env(pairs), mock_env_has(pairs));
    assert_eq!(v, Some("http://legit.example:3128".to_string()));
}

/// Exercise the asymmetry between `get_var` and
/// `has_var` that the dependency-injected helper explicitly
/// permits. A non-UTF-8 value in the real env has
/// `env::var(k) == Err` (`get_var` returns None) but
/// `env::var_os(k) == Some` (`has_var` returns true). The
/// candidate MUST be treated as unreadable - skipped by the
/// candidate loop but still honoured for the CGI sniff.
#[test]
fn env_proxy_skips_present_but_unreadable_vars() {
    // Simulate: both HTTPS_PROXY and HTTP_PROXY *present* but
    // unreadable as UTF-8. Non-CGI; neither candidate should
    // yield a proxy URL - NOT an empty-string fallback that
    // quietly disables proxying without telling the caller.
    let get_var = |k: &str| -> Option<String> {
        let _ = k;
        None
    };
    let has_var = |k: &str| matches!(k, "HTTPS_PROXY" | "HTTP_PROXY");
    assert_eq!(env_proxy_from(get_var, has_var), None);
}

#[test]
fn env_proxy_under_cgi_with_unreadable_http_proxy() {
    // httpoxy mitigation must still trigger when HTTP_PROXY is
    // present-but-unreadable under CGI. The candidate list
    // excludes uppercase HTTP_PROXY; `get_var` returning None
    // for the remaining candidates, so the overall result is None.
    let get_var = |k: &str| -> Option<String> {
        let _ = k;
        None
    };
    let has_var = |k: &str| matches!(k, "GATEWAY_INTERFACE" | "HTTP_PROXY");
    assert_eq!(env_proxy_from(get_var, has_var), None);
}

#[test]
fn env_proxy_under_cgi_with_unreadable_http_proxy_but_readable_https() {
    // Even under CGI with a suspicious unreadable HTTP_PROXY,
    // a legitimately readable HTTPS_PROXY must still win -
    // HTTPS_PROXY is not spoofable via HTTP request headers.
    let legit = "http://legit.example:3128";
    let get_var = move |k: &str| -> Option<String> {
        if k == "HTTPS_PROXY" {
            Some(legit.to_string())
        } else {
            None
        }
    };
    let has_var = |k: &str| matches!(k, "GATEWAY_INTERFACE" | "HTTPS_PROXY" | "HTTP_PROXY");
    assert_eq!(env_proxy_from(get_var, has_var), Some(legit.to_string()));
}

#[test]
fn env_proxy_under_cgi_still_honours_lowercase_http_proxy() {
    // CGI does not populate lowercase `http_proxy` (CGI writes
    // HTTP_PROXY from a `Proxy:` header); so a shell-set
    // lowercase value remains a legit user config.
    let pairs = &[
        ("REQUEST_METHOD", "GET"),
        ("http_proxy", "http://user.example:3128"),
        ("HTTP_PROXY", "http://evil.attacker:3128"),
    ];
    let v = env_proxy_from(mock_env(pairs), mock_env_has(pairs));
    assert_eq!(v, Some("http://user.example:3128".to_string()));
}
