//! Set-Cookie header parser (RFC 6265bis).

use std::time::{Duration, SystemTime};

use crate::cookie::record::{Cookie, SameSite};

/// Max cookie lifetime: 400 days (Chrome enforcement).
const MAX_LIFETIME: Duration = Duration::from_secs(400 * 24 * 60 * 60);

/// Parse a Set-Cookie header value into a Cookie.
///
/// Returns None if the cookie is malformed or should be rejected.
pub fn parse_set_cookie(header: &str, request_url: &url::Url) -> Option<Cookie> {
    let now = SystemTime::now();

    // Split on first '=' to get name=value.
    let (name_value, attrs_str) = match header.find(';') {
        Some(i) => (&header[..i], &header[i + 1..]),
        None => (header, ""),
    };

    let (name, value) = {
        // No '=' → malformed, reject.
        let i = name_value.find('=')?;
        let v = name_value[i + 1..].trim();
        // Strip surrounding double-quotes (Chrome behavior).
        let v = v
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(v);
        (name_value[..i].trim(), v)
    };

    if name.is_empty() {
        return None;
    }

    // Parse attributes.
    let mut domain = None;
    let mut path = None;
    let mut secure = false;
    let mut http_only = false;
    let mut same_site = None;
    let mut max_age = None;
    let mut expires = None;

    for attr in attrs_str.split(';') {
        let attr = attr.trim();
        if attr.is_empty() {
            continue;
        }
        let (attr_name, attr_value) = match attr.find('=') {
            Some(i) => (attr[..i].trim(), Some(attr[i + 1..].trim())),
            None => (attr, None),
        };

        match attr_name.to_lowercase().as_str() {
            "domain" => {
                if let Some(v) = attr_value {
                    let d = v.strip_prefix('.').unwrap_or(v);
                    if !d.is_empty() {
                        domain = Some(d.to_lowercase());
                    }
                }
            }
            "path" => {
                if let Some(v) = attr_value {
                    if v.starts_with('/') {
                        path = Some(v.to_string());
                    }
                }
            }
            "secure" => secure = true,
            "httponly" => http_only = true,
            "samesite" => {
                if let Some(v) = attr_value {
                    same_site = match v.to_lowercase().as_str() {
                        "strict" => Some(SameSite::Strict),
                        "lax" => Some(SameSite::Lax),
                        "none" => Some(SameSite::None),
                        _ => None,
                    };
                }
            }
            "max-age" => {
                if let Some(v) = attr_value {
                    if let Ok(secs) = v.parse::<i64>() {
                        if secs <= 0 {
                            max_age = Some(Duration::ZERO); // expire immediately
                        } else {
                            max_age = Some(Duration::from_secs(secs as u64));
                        }
                    }
                }
            }
            "expires" => {
                if let Some(v) = attr_value {
                    expires = parse_cookie_date(v);
                }
            }
            _ => {} // ignore unknown attributes
        }
    }

    // SameSite=None requires Secure (Chrome enforcement).
    let same_site = match same_site {
        Some(SameSite::None) if !secure => return None, // reject
        Some(s) => s,
        None => SameSite::Lax, // Chrome default
    };

    // Cookie prefix validation.
    if name.starts_with("__Secure-") && !secure {
        return None;
    }
    if name.starts_with("__Host-") {
        if !secure || domain.is_some() {
            return None;
        }
        // __Host- cookies must have path=/
        if path.as_deref() != Some("/") {
            return None;
        }
    }

    // Compute expiry.
    let computed_expires = if let Some(ma) = max_age {
        // Max-Age takes precedence over Expires.
        if ma == Duration::ZERO {
            Some(now) // expire immediately
        } else {
            let capped = ma.min(MAX_LIFETIME);
            Some(now + capped)
        }
    } else if let Some(exp) = expires {
        // Cap at 400 days from now.
        let max_time = now + MAX_LIFETIME;
        Some(exp.min(max_time))
    } else {
        None // session cookie
    };

    // Default domain to request host (host-only cookie).
    let request_host = request_url.host_str().unwrap_or("").to_lowercase();
    let host_only = domain.is_none();
    let cookie_domain = domain.unwrap_or_else(|| request_host.clone());

    // Reject cookies set on public suffixes (basic check).
    if !host_only && is_public_suffix(&cookie_domain) {
        return None;
    }

    // RFC 6265bis Section 5.3.6: Domain must match the request host.
    // The cookie domain must be equal to or a parent domain of the request host.
    if !host_only {
        let cd = cookie_domain.to_lowercase();
        let rh = request_host.to_lowercase();
        if rh != cd && !rh.ends_with(&format!(".{cd}")) {
            return None; // Cross-domain cookie injection blocked.
        }
    }

    // Default path from request URL.
    let cookie_path = path.unwrap_or_else(|| default_path(request_url.path()));

    // Size limit: 4096 bytes.
    if name.len() + value.len() > 4096 {
        return None;
    }

    Some(Cookie {
        name: name.to_string(),
        value: value.to_string(),
        domain: cookie_domain,
        path: cookie_path,
        secure,
        http_only,
        same_site,
        expires: computed_expires,
        creation_time: now,
        last_access: now,
        host_only,
    })
}

/// Is `domain` a public suffix (per RFC 6265bis §5.2, which forbids setting a
/// cookie on one)? Resolves against the Mozilla Public Suffix List via
/// [`psl`], so it catches `co.uk`, `github.io`, `vercel.app`, and the
/// thousands of entries a gTLD allow-list misses.
fn is_public_suffix(domain: &str) -> bool {
    // A single-label domain (no dot) has no registrable parent — every browser
    // rejects `Domain=com` / `Domain=localhost`.
    if !domain.contains('.') {
        return true;
    }
    // It is a public suffix iff the PSL resolves it to itself.
    psl::suffix(domain.as_bytes())
        .is_some_and(|s| s.as_bytes().eq_ignore_ascii_case(domain.as_bytes()))
}

/// Extract the `(name, value)` of a Set-Cookie header the jar REFUSED to store
/// (bad domain, public suffix, `__Host-`/`__Secure-` violation) for the
/// response view. The jar's RFC 6265bis storage policy decides what may be
/// persisted and broadcast on later requests; the response view instead
/// reports what the server actually sent (reqwest parity), because storage
/// policy is about the jar's trust boundary, not about this response's bytes.
///
/// Deletions (`Max-Age` ≤ 0) and malformed headers yield `None`: a deletion is
/// not a live cookie, and a header with no name/value has nothing to report.
pub(crate) fn rejected_cookie_name_value(header: &str) -> Option<(String, String)> {
    let name_value = header.split(';').next()?;
    let (name, value) = name_value.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let value = value.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value);
    // Reject deletions: any `Max-Age` attribute ≤ 0 means "delete now".
    for attr in header.split(';').skip(1) {
        let attr = attr.trim();
        if let Some(v) = attr
            .split_once('=')
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("max-age"))
            .map(|(_, v)| v.trim())
        {
            if v.parse::<i64>().is_ok_and(|secs| secs <= 0) {
                return None;
            }
        }
    }
    Some((name.to_string(), value.to_string()))
}

/// The registrable domain (eTLD+1) of `host` per the Public Suffix List —
/// e.g. `www.example.co.uk` → `example.co.uk`. `None` when `host` is itself a
/// public suffix or has no registrable parent (an IP literal, `localhost`).
pub(crate) fn registrable_domain(host: &str) -> Option<String> {
    psl::domain(host.as_bytes()).map(|d| String::from_utf8_lossy(d.as_bytes()).into_owned())
}

/// Default cookie path from request URI (RFC 6265bis Section 5.1.4).
fn default_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return "/".to_string();
    }
    match request_path.rfind('/') {
        Some(i) if i > 0 => request_path[..i].to_string(),
        _ => "/".to_string(),
    }
}

/// Basic cookie date parser (handles common formats).
fn parse_cookie_date(s: &str) -> Option<SystemTime> {
    // Try RFC 1123: "Thu, 01 Dec 2025 00:00:00 GMT"
    // Try RFC 850: "Thursday, 01-Dec-25 00:00:00 GMT"
    // Try asctime: "Thu Dec  1 00:00:00 2025"
    // For robustness, we try to extract year/month/day/time components.
    let s = s.trim();

    // Quick heuristic parse — extract numbers and month name.
    let mut day = 0u32;
    let mut month = 0u32;
    let mut year = 0u32;
    let mut hour = 0u32;
    let mut minute = 0u32;
    let mut second = 0u32;
    let mut found_time = false;

    for token in s.split([' ', '-', ',']) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }

        if !found_time && token.contains(':') {
            // Time component: HH:MM:SS
            let parts: Vec<&str> = token.split(':').collect();
            if parts.len() >= 2 {
                hour = parts[0].parse().unwrap_or(0);
                minute = parts[1].parse().unwrap_or(0);
                second = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                found_time = true;
            }
            continue;
        }

        if let Ok(n) = token.parse::<u32>() {
            if n > 99 {
                year = n;
            } else if day == 0 {
                day = n;
            } else if year == 0 {
                year = if n > 68 { 1900 + n } else { 2000 + n };
            }
            continue;
        }

        // Month name.
        let m = match token.get(..3).map(|s| s.to_lowercase()).as_deref() {
            Some("jan") => 1,
            Some("feb") => 2,
            Some("mar") => 3,
            Some("apr") => 4,
            Some("may") => 5,
            Some("jun") => 6,
            Some("jul") => 7,
            Some("aug") => 8,
            Some("sep") => 9,
            Some("oct") => 10,
            Some("nov") => 11,
            Some("dec") => 12,
            _ => 0,
        };
        if m > 0 {
            month = m;
        }
    }

    if day == 0 || month == 0 || year == 0 {
        return None;
    }

    // Convert to SystemTime (rough — no full calendar math, but sufficient for cookies).
    //
    // Pre-1970 `Expires` values (e.g. `Expires=Wed, 21 Oct 1013 ...` which
    // the cookie fuzzer actually produced) yield a negative
    // `days_from_epoch`. Cookies with a pre-epoch expiry are already
    // expired — collapse them to `UNIX_EPOCH` rather than panicking on
    // `(negative as u64) * 86400` overflow under `debug_assertions`.
    let days_from_epoch = days_since_epoch(year, month, day)?;
    if days_from_epoch < 0 {
        return Some(SystemTime::UNIX_EPOCH);
    }
    let secs = (days_from_epoch as u64)
        .checked_mul(86400)?
        .checked_add(hour as u64 * 3600)?
        .checked_add(minute as u64 * 60)?
        .checked_add(second as u64)?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

/// Days since Unix epoch for a given date.
fn days_since_epoch(year: u32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Simplified: use a basic formula.
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let m = if month <= 2 { month + 9 } else { month - 3 } as i64;
    let d = day as i64;
    Some(365 * y + y / 4 - y / 100 + y / 400 + (m * 306 + 5) / 10 + d - 719469)
}

#[cfg(test)]
mod tests {
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
        let rejected =
            "late=abc|1|0|def; Path=/; Max-Age=1577847600; Domain=example.com; Secure";
        let (name, value) = rejected_cookie_name_value(rejected).unwrap();
        assert_eq!(name, "late");
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
        // Multi-label suffixes the old gTLD allow-list missed.
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

    /// Regression gate for a cookie_set fuzzer finding:
    /// pre-epoch `Expires` dates produced a negative `days_from_epoch`
    /// that was cast to `u64`, then multiplied by 86400, panicking on
    /// overflow under `debug_assertions`. Such cookies are already
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
}
