use std::time::Duration;

use crate::cookie::record::SameSite;

use super::{CookieAttributes, parse_cookie_date};

pub(super) fn set(a: &mut CookieAttributes, name: &str, value: Option<&str>) {
    match name.to_lowercase().as_str() {
        "domain" => {
            if let Some(v) = value {
                let d = v.strip_prefix('.').unwrap_or(v);
                if !d.is_empty() {
                    a.domain = Some(d.to_lowercase());
                }
            }
        }
        "path" => {
            if let Some(v) = value
                && v.starts_with('/')
            {
                a.path = Some(v.to_string());
            }
        }
        "secure" => a.secure = true,
        "httponly" => a.http_only = true,
        "samesite" => {
            if let Some(v) = value {
                a.same_site = same_site(v);
            }
        }
        "max-age" => {
            if let Some(v) = value
                && let Some(d) = max_age(v)
            {
                a.max_age = Some(d);
            }
        }
        "expires" => {
            if let Some(v) = value {
                a.expires = parse_cookie_date(v);
            }
        }
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
