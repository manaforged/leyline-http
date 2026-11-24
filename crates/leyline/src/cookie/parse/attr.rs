use std::time::Duration;

use crate::cookie::record::{MAX_ATTRIBUTE_OCTETS, SameSite};

use super::{CookieAttributes, parse_cookie_date};

impl CookieAttributes {
    pub(super) fn set(&mut self, name: &str, value: Option<&str>) {
        match (name.to_lowercase().as_str(), value) {
            ("secure", _) => self.secure = true,
            ("httponly", _) => self.http_only = true,
            (_, Some(value)) if value.len() > MAX_ATTRIBUTE_OCTETS => {}
            (name, Some(value)) => self.set_valued(name, value),
            _ => {}
        }
    }

    fn set_valued(&mut self, name: &str, value: &str) {
        match name {
            "domain" => {
                let d = value.strip_prefix('.').unwrap_or(value);
                if !d.is_empty() {
                    self.domain = Some(d.to_lowercase());
                }
            }
            "path" if value.starts_with('/') => self.path = Some(value.to_string()),
            "samesite" => self.same_site = same_site(value),
            "max-age" => {
                if let Some(d) = max_age(value) {
                    self.max_age = Some(d);
                }
            }
            "expires" => self.expires_after_epoch = parse_cookie_date(value),
            _ => {}
        }
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
    let bytes = v.as_bytes();
    let (negative, digits) = match bytes.first() {
        Some(b'-') => (true, &bytes[1..]),
        _ => (false, bytes),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if negative {
        return Some(Duration::ZERO);
    }
    let secs = v.parse::<i64>().unwrap_or(i64::MAX);
    Some(Duration::from_secs(secs.max(0) as u64))
}
