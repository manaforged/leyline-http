//! Set-Cookie header parser (RFC 6265bis).

use std::time::{Duration, SystemTime};

use crate::cookie::record::{Cookie, SameSite};

/// Max cookie lifetime: 400 days (Chrome enforcement).
const MAX_LIFETIME: Duration = Duration::from_secs(400 * 24 * 60 * 60);

/// Parse a Set-Cookie header value into a Cookie.
///
/// Returns None if the cookie is malformed or should be rejected.
/// Parsed `Secure`/`HttpOnly`/`SameSite`/`Domain`/`Path`/`Max-Age`/
/// `Expires` attributes. Unknown attributes are ignored.
#[derive(Default)]
struct CookieAttributes {
    domain: Option<String>,
    path: Option<String>,
    secure: bool,
    http_only: bool,
    same_site: Option<SameSite>,
    max_age: Option<Duration>,
    expires: Option<SystemTime>,
}

/// Parse the attribute section of a Set-Cookie header (everything after
/// the first `;`).
fn parse_attributes(attrs_str: &str) -> CookieAttributes {
    let mut a = CookieAttributes::default();
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
                        a.domain = Some(d.to_lowercase());
                    }
                }
            }
            "path" => {
                if let Some(v) = attr_value {
                    if v.starts_with('/') {
                        a.path = Some(v.to_string());
                    }
                }
            }
            "secure" => a.secure = true,
            "httponly" => a.http_only = true,
            "samesite" => {
                if let Some(v) = attr_value {
                    a.same_site = match v.to_lowercase().as_str() {
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
                            a.max_age = Some(Duration::ZERO);
                        } else {
                            a.max_age = Some(Duration::from_secs(secs as u64));
                        }
                    }
                }
            }
            "expires" => {
                if let Some(v) = attr_value {
                    a.expires = parse_cookie_date(v);
                }
            }
            _ => {}
        }
    }
    a
}

/// Strict Secure Cookies (Chrome 52+): a Secure cookie may only be set
/// by a secure origin. Localhost is trustworthy, matching browsers.
fn secure_origin(request_url: &url::Url) -> bool {
    let scheme = request_url.scheme();
    let host = request_url.host_str().unwrap_or("");
    scheme == "https"
        || host == "localhost"
        || host == "127.0.0.1"
        || host == "[::1]"
        || host.ends_with(".localhost")
}

/// RFC 6265bis §4.1.3 prefix rules, matched case-insensitively.
fn prefix_rejected(name: &str, attrs: &CookieAttributes) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("__secure-") {
        return !attrs.secure;
    }
    if lower.starts_with("__host-") {
        return !attrs.secure || attrs.domain.is_some() || attrs.path.as_deref() != Some("/");
    }
    false
}

/// Expiry: Max-Age wins over Expires; both cap at [`MAX_LIFETIME`].
fn compute_expiry(
    now: SystemTime,
    max_age: Option<Duration>,
    expires: Option<SystemTime>,
) -> Option<SystemTime> {
    match max_age {
        Some(Duration::ZERO) => Some(now),
        Some(ma) => Some(now + ma.min(MAX_LIFETIME)),
        None => expires.map(|exp| exp.min(now + MAX_LIFETIME)),
    }
}

/// Domain scoping (RFC 6265bis §5.5/§5.7): on an IP-literal host the
/// `Domain` attribute is ignored (host-only); otherwise the domain must
/// equal or parent the request host, and must not be a public suffix.
/// Returns `(host_only, cookie_domain)`.
fn resolve_cookie_domain(request_url: &url::Url, domain: Option<String>) -> Option<(bool, String)> {
    let host_only = domain.is_none();
    let request_host = request_url.host_str().unwrap_or("").to_lowercase();
    let cookie_domain = domain.unwrap_or_else(|| request_host.clone());

    if !host_only && is_public_suffix(&cookie_domain) {
        return None;
    }
    if !host_only {
        let rh = request_host.to_lowercase();
        if rh != cookie_domain && !rh.ends_with(&format!(".{cookie_domain}")) {
            return None;
        }
    }
    Some((host_only, cookie_domain))
}

pub fn parse_set_cookie(header: &str, request_url: &url::Url) -> Option<Cookie> {
    let now = SystemTime::now();

    let (name_value, attrs_str) = match header.find(';') {
        Some(i) => (&header[..i], &header[i + 1..]),
        None => (header, ""),
    };

    let (name, value) = {
        let i = name_value.find('=')?;
        let v = name_value[i + 1..].trim();
        let v = v
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(v);
        (name_value[..i].trim(), v)
    };

    if name.is_empty() {
        return None;
    }

    // RFC 6265bis §5.6: names and values must be free of control
    // characters; a CTL that enters the jar is echoed on later requests.
    let has_ctl = |s: &str| s.bytes().any(|b| b < 0x20 || b == 0x7F);
    if has_ctl(name) || has_ctl(value) {
        return None;
    }

    let attrs = parse_attributes(attrs_str);

    // Strict Secure Cookies (Chrome 52+): Secure requires a secure origin.
    if attrs.secure && !secure_origin(request_url) {
        return None;
    }

    // SameSite=None requires Secure (Chrome enforcement).
    let same_site = match attrs.same_site {
        Some(SameSite::None) if !attrs.secure => return None,
        Some(s) => s,
        None => SameSite::Lax,
    };

    if prefix_rejected(name, &attrs) {
        return None;
    }

    let computed_expires = compute_expiry(now, attrs.max_age, attrs.expires);

    // On an IP-literal host the Domain attribute is ignored (§5.5); see
    // `resolve_cookie_domain` for why that matters.
    let domain = if matches!(
        request_url.host(),
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_))
    ) {
        None
    } else {
        attrs.domain
    };

    let (host_only, cookie_domain) = resolve_cookie_domain(request_url, domain)?;

    let cookie_path = attrs
        .path
        .unwrap_or_else(|| default_path(request_url.path()));

    if name.len() + value.len() > 4096 {
        return None;
    }

    Some(Cookie {
        name: name.to_string(),
        value: value.to_string(),
        domain: cookie_domain,
        path: cookie_path,
        secure: attrs.secure,
        http_only: attrs.http_only,
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
    // Extract year/month/day/time components independently; anything
    // unparseable falls through to the rejection path.
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
mod tests;
