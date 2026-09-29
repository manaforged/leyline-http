use std::time::Duration;

use crate::cookie::record::SameSite;

use super::{CookieAttributes, parse_cookie_date};

pub(super) fn set(a: &mut CookieAttributes, name: &str, value: Option<&str>) {
    match (name.to_lowercase().as_str(), value) {
        ("secure", _) => a.secure = true,
        ("httponly", _) => a.http_only = true,
        (name, Some(value)) => set_valued(a, name, value),
        _ => {}
    }
}

fn set_valued(a: &mut CookieAttributes, name: &str, value: &str) {
    match name {
        "domain" => {
            let d = value.strip_prefix('.').unwrap_or(value);
            if !d.is_empty() {
                a.domain = Some(d.to_lowercase());
            }
        }
        "path" if value.starts_with('/') => a.path = Some(value.to_string()),
        "samesite" => a.same_site = same_site(value),
        "max-age" => {
            if let Some(d) = max_age(value) {
                a.max_age = Some(d);
            }
        }
        "expires" => a.expires_after_epoch = parse_cookie_date(value),
        _ => {}
    }
}

fn same_site(v: &str) -> Option<SameSite> {
    match v.to_lowercase().as_str() {
        "strict" => Some(SameSite::Strict),
        "lax" => Some(SameSite::Lax),
        "none" => Some(SameSite::None),
        _ => None,
    }
}

fn max_age(v: &str) -> Option<Duration> {
    let secs = v.parse::<i64>().ok()?;
    if secs <= 0 {
        Some(Duration::ZERO)
    } else {
        Some(Duration::from_secs(secs as u64))
    }
}
