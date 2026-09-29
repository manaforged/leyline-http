use std::time::{Duration, SystemTime};

use crate::cookie::record::{Cookie, SameSite, capped_expiry};
use crate::util::epoch_plus;

mod attr;
mod date;

use attr::set;
use date::{Date, stamp, token};

#[derive(Default)]
struct CookieAttributes {
    domain: Option<String>,
    path: Option<String>,
    secure: bool,
    http_only: bool,
    same_site: Option<SameSite>,
    max_age: Option<Duration>,
    expires_after_epoch: Option<Duration>,
}

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

fn secure_origin(request_url: &url::Url) -> bool {
    let scheme = request_url.scheme();
    let host = request_url.host_str().unwrap_or("");
    scheme == "https"
        || host == "localhost"
        || host == "127.0.0.1"
        || host == "[::1]"
        || host.ends_with(".localhost")
}

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

fn compute_expiry(
    now: SystemTime,
    max_age: Option<Duration>,
    expires_after_epoch: Option<Duration>,
) -> Option<SystemTime> {
    let requested = match max_age {
        Some(age) => now.checked_add(age),
        None => epoch_plus(expires_after_epoch?),
    };
    Some(capped_expiry(now, requested))
}

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

fn split_pair(header: &str) -> Option<(&str, &str, &str)> {
    let (name_value, attrs) = header.split_once(';').unwrap_or((header, ""));
    let (name, value) = name_value.split_once('=')?;
    let (name, value) = (name.trim(), value.trim());
    let has_ctl = |s: &str| s.bytes().any(|b| b < 0x20 || b == 0x7F);
    if name.is_empty() || has_ctl(name) || has_ctl(value) || name.len() + value.len() > 4096 {
        return None;
    }
    Some((name, value, attrs))
}

fn admitted_same_site(
    name: &str,
    attrs: &CookieAttributes,
    request_url: &url::Url,
) -> Option<SameSite> {
    if (attrs.secure && !secure_origin(request_url)) || prefix_rejected(name, attrs) {
        return None;
    }
    match attrs.same_site {
        Some(SameSite::None) if !attrs.secure => None,
        Some(same_site) => Some(same_site),
        None => Some(SameSite::Lax),
    }
}

pub fn parse_set_cookie(header: &str, request_url: &url::Url) -> Option<Cookie> {
    let now = SystemTime::now();
    let (name, value, attrs_str) = split_pair(header)?;
    let attrs = parse_attributes(attrs_str);
    let same_site = admitted_same_site(name, &attrs, request_url)?;

    let computed_expires = compute_expiry(now, attrs.max_age, attrs.expires_after_epoch);

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

fn is_public_suffix(domain: &str) -> bool {
    if !domain.contains('.') {
        return true;
    }
    psl::suffix(domain.as_bytes())
        .is_some_and(|s| s.as_bytes().eq_ignore_ascii_case(domain.as_bytes()))
}

pub(crate) fn registrable_domain(host: &str) -> Option<String> {
    psl::domain(host.as_bytes()).map(|d| String::from_utf8_lossy(d.as_bytes()).into_owned())
}

fn default_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return "/".to_string();
    }
    match request_path.rfind('/') {
        Some(i) if i > 0 => request_path[..i].to_string(),
        _ => "/".to_string(),
    }
}

pub fn parse_cookie_date(s: &str) -> Option<Duration> {
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
