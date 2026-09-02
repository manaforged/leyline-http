//! Set-Cookie header parser (RFC 6265bis).

use std::time::{Duration, SystemTime};

use crate::cookie::record::{Cookie, SameSite};

mod attr;
mod date;

use attr::set;
use date::{Date, stamp, token};

/// Max cookie lifetime: 400 days (Chrome enforcement).
const MAX_LIFETIME: Duration = Duration::from_secs(400 * 24 * 60 * 60);

/// Parse a Set-Cookie header value into a Cookie.
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

/// Parse the attribute section of a Set-Cookie header (everything after the first `;`).
fn parse_attributes(attrs_str: &str) -> CookieAttributes {
    let mut a = CookieAttributes::default();
    for attr in attrs_str.split(';') {
        let attr = attr.trim();
        if attr.is_empty() {
            continue;
        }
        let (name, value) = match attr.find('=') {
            Some(i) => (attr[..i].trim(), Some(attr[i + 1..].trim())),
            None => (attr, None),
        };
        set(&mut a, name, value);
    }
    a
}

/// Strict Secure Cookies (Chrome 52+): a Secure cookie may only be set by a secure origin.
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

/// Domain scoping (RFC 6265bis §5.5/§5.7): on an IP-literal host the `Domain` attribute is ignored (host-only); otherwise the domain must equal or parent the request host, and must not be a public suffix.
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

/// Parse one `Set-Cookie` header line against `request_url`, returning `None` when the cookie is rejected.
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

    let has_ctl = |s: &str| s.bytes().any(|b| b < 0x20 || b == 0x7F);
    if has_ctl(name) || has_ctl(value) {
        return None;
    }

    let attrs = parse_attributes(attrs_str);

    if attrs.secure && !secure_origin(request_url) {
        return None;
    }

    let same_site = match attrs.same_site {
        Some(SameSite::None) if !attrs.secure => return None,
        Some(s) => s,
        None => SameSite::Lax,
    };

    if prefix_rejected(name, &attrs) {
        return None;
    }

    let computed_expires = compute_expiry(now, attrs.max_age, attrs.expires);

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

/// Is `domain` a public suffix (per RFC 6265bis §5.2, which forbids setting a cookie on one)?
fn is_public_suffix(domain: &str) -> bool {
    if !domain.contains('.') {
        return true;
    }
    psl::suffix(domain.as_bytes())
        .is_some_and(|s| s.as_bytes().eq_ignore_ascii_case(domain.as_bytes()))
}

/// Extract the `(name, value)` of a Set-Cookie header the jar REFUSED to store (bad domain, public suffix, `__Host-`/`__Secure-` violation) for the response view.
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
    for attr in header.split(';').skip(1) {
        let attr = attr.trim();
        if let Some(v) = attr
            .split_once('=')
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("max-age"))
            .map(|(_, v)| v.trim())
            && v.parse::<i64>().is_ok_and(|secs| secs <= 0)
        {
            return None;
        }
    }
    Some((name.to_string(), value.to_string()))
}

/// The registrable domain (eTLD+1) of `host` per the Public Suffix List — e.g. `www.example.co.uk` → `example.co.uk`.
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
pub fn parse_cookie_date(s: &str) -> Option<SystemTime> {
    let mut d = Date::default();
    for tok in s.trim().split([' ', '-', ',']) {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        token(&mut d, tok);
    }
    stamp(&d)
}

/// Days since Unix epoch for a given date.
fn days_since_epoch(year: u32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year } as i64;
    let m = if month <= 2 { month + 9 } else { month - 3 } as i64;
    let d = day as i64;
    Some(365 * y + y / 4 - y / 100 + y / 400 + (m * 306 + 5) / 10 + d - 719469)
}

#[cfg(test)]
mod tests;
