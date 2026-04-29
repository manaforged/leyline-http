/// Environment variable names whose presence indicates a CGI-like
/// request-handler context where uppercase `HTTP_PROXY` is untrusted.
pub(crate) const CGI_SIGNAL_ENV_VARS: &[&str] = &[
    "GATEWAY_INTERFACE",
    "REQUEST_METHOD",
    "SERVER_SOFTWARE",
    "SCRIPT_NAME",
    "SCRIPT_FILENAME",
    "PATH_INFO",
    "QUERY_STRING",
    "SERVER_PROTOCOL",
    "SERVER_NAME",
    "SERVER_PORT",
];

pub(super) fn env_proxy() -> Option<String> {
    env_proxy_from(|k| std::env::var(k).ok(), |k| std::env::var_os(k).is_some())
}

/// Pure-function core of [`env_proxy`]: given a value getter and a
/// presence getter, return the first non-empty proxy URL while
/// applying the httpoxy CGI sniff. Exposed to the test module so
/// we can unit-test the httpoxy mitigation without mutating the
/// process environment (which would race with other tests).
pub(crate) fn env_proxy_from<F, G>(get_var: F, has_var: G) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
    G: Fn(&str) -> bool,
{
    let in_cgi = CGI_SIGNAL_ENV_VARS.iter().any(|k| has_var(k));

    if in_cgi && has_var("HTTP_PROXY") {
        tracing::warn!(
            target: "leyline::env_proxy::cgi",
            "CGI environment detected — ignoring HTTP_PROXY (httpoxy mitigation)"
        );
    }

    let candidates: &[&str] = if in_cgi {
        // Skip the uppercase `HTTP_PROXY` variant that CGI collides
        // with. `HTTPS_PROXY` isn't an HTTP request header so it's
        // safe; `http_proxy` (lowercase) isn't populated by CGI.
        &["HTTPS_PROXY", "https_proxy", "http_proxy"]
    } else {
        &["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"]
    };

    for name in candidates {
        if let Some(val) = get_var(name) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Returns `true` when `host` matches a `NO_PROXY` / `no_proxy`
/// pattern and should therefore bypass the configured proxy.
///
/// Accepts a comma-separated list of patterns. Match rules follow
/// the widely-implemented convention (curl, Python requests, Go):
/// - `*` — bypass all hosts.
/// - `.example.com` / `example.com` — matches `example.com` and any
///   subdomain `*.example.com`.
/// - IP literals compare exactly.
/// - Trailing dots in both host and pattern are stripped before
///   comparison (`example.com.` == `example.com`).
/// - Port suffixes on patterns (`example.com:8080`) are stripped
///   before comparison — conventional behaviour ignores port.
/// - IDN pattern + ASCII host (and vice versa) compare equal after
///   punycode normalisation.
#[allow(dead_code)]
pub(crate) fn host_bypasses_proxy(host: &str) -> bool {
    let raw = std::env::var("NO_PROXY")
        .ok()
        .or_else(|| std::env::var("no_proxy").ok());
    let Some(raw) = raw else { return false };
    host_matches_no_proxy(host, &raw)
}

/// Pure-function core of [`host_bypasses_proxy`] that takes the
/// `NO_PROXY` string directly — easier to unit-test than the env-var
/// reading wrapper.
#[allow(dead_code)]
pub(crate) fn host_matches_no_proxy(host: &str, raw: &str) -> bool {
    let host = normalise_host(host);
    for token in raw.split(',') {
        let mut pat = token.trim().to_string();
        if pat.is_empty() {
            continue;
        }
        if pat == "*" {
            return true;
        }
        // Strip a `:port` suffix from the pattern so
        // `example.com:8080` matches any-port requests to
        // `example.com` — the conventional convention across
        // curl / requests / Go.
        //
        // A bare IPv6 literal like `::1` or `fe80::1` contains
        // multiple colons; naive `rfind(':')` stripping would
        // turn `::1` into `::` and silently break loopback
        // bypass. Handle three cases:
        //   1. bracketed IPv6 with port: `[::1]:8080` — strip
        //      everything from `]:` onward (normalisation below
        //      drops the brackets).
        //   2. IPv4 / domain with port: exactly one `:`, digits
        //      after — strip.
        //   3. bare IPv6 (two or more colons, no brackets) —
        //      leave as-is.
        if pat.starts_with('[') {
            if let Some(end) = pat.find("]:") {
                let suffix = &pat[end + 2..];
                if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                    pat.truncate(end + 1); // keep the trailing `]`
                }
            }
        } else if pat.matches(':').count() == 1 {
            if let Some(idx) = pat.rfind(':') {
                if pat[idx + 1..].bytes().all(|b| b.is_ascii_digit()) {
                    pat.truncate(idx);
                }
            }
        }
        let pat = normalise_host(&pat);
        let needle = pat.strip_prefix('.').unwrap_or(&pat).to_string();
        if host == needle || host.ends_with(&format!(".{needle}")) {
            return true;
        }
    }
    false
}

/// Canonicalise a host for `NO_PROXY` comparison: lowercase, strip
/// trailing dot, punycode IDNs so Unicode and A-label forms compare
/// equal. Falls back to the ASCII-lowered form on IDN failure.
#[allow(dead_code)]
fn normalise_host(host: &str) -> String {
    let stripped = host.trim().trim_end_matches('.');
    // Strip a matching pair of `[` `]` around IPv6 literals —
    // curl-style NO_PROXY accepts both `[::1]` and `::1` forms,
    // and `url::Host::parse` rejects bracketed input. Only strip
    // when both brackets are present.
    let stripped = stripped
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(stripped);
    let lowered = stripped.to_ascii_lowercase();
    // `url::Host` parses both IP literals and IDN domain names, so
    // it's a convenient one-stop canonicaliser for the NO_PROXY
    // matcher without pulling `idna` as a direct dep.
    match url::Host::parse(&lowered) {
        Ok(url::Host::Domain(d)) => d,
        Ok(url::Host::Ipv4(a)) => a.to_string(),
        Ok(url::Host::Ipv6(a)) => a.to_string(),
        Err(_) => lowered,
    }
}
