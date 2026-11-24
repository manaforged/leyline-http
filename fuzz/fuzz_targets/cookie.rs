#![no_main]

use std::hint::black_box;
use std::time::{Duration, SystemTime};

use leyline::cookie::SameSite;
use leyline::internals::fuzz::{parse_cookie_date, parse_set_cookie};
use libfuzzer_sys::fuzz_target;

const ORIGINS: [&str; 4] = [
    "https://www.example.com/a/b",
    "http://localhost:8080/",
    "https://example.co.uk/",
    "https://127.0.0.1/x",
];

fn is_rfc6265_max_age(value: &str) -> bool {
    if value.len() > 1024 {
        return false;
    }
    let bytes = value.as_bytes();
    let digits = bytes.strip_prefix(b"-").unwrap_or(bytes);
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

fuzz_target!(|data: &[u8]| {
    let Some((pick, rest)) = data.split_first() else {
        return;
    };
    let line = String::from_utf8_lossy(rest);
    let origin = ORIGINS[*pick as usize % ORIGINS.len()];
    if let Some(cookie) = parse_set_cookie(&line, origin) {
        assert!(!cookie.name().is_empty());
        assert!(!cookie.name().bytes().any(|b| b < 0x20 || b == 0x7f));
        assert!(!cookie.value().bytes().any(|b| b < 0x20 || b == 0x7f));
        assert!(cookie.name().len() + cookie.value().len() <= 4096);
        if cookie.same_site() == SameSite::None {
            assert!(cookie.secure());
        }
        let lower = cookie.name().to_ascii_lowercase();
        if lower.starts_with("__secure-") {
            assert!(cookie.secure());
        }
        if lower.starts_with("__host-") {
            assert!(cookie.secure() && cookie.host_only() && cookie.path() == "/");
        }
        if origin == "https://127.0.0.1/x" {
            assert!(cookie.host_only() && cookie.domain() == "127.0.0.1");
        }
        if let Some(expires) = cookie.expires() {
            let cap = SystemTime::now() + Duration::from_secs(400 * 86_400 + 5);
            assert!(expires <= cap, "cookie expiry exceeded 400-day cap");
        }
        let mut has_valid_max_age = false;
        let mut has_expires = false;
        for attr in line.split(';').skip(1) {
            let trimmed = attr.trim();
            if let Some((k, v)) = trimmed.split_once('=') {
                let k = k.trim();
                let v = v.trim();
                if k.eq_ignore_ascii_case("max-age") && is_rfc6265_max_age(v) {
                    has_valid_max_age = true;
                } else if k.eq_ignore_ascii_case("expires") {
                    has_expires = true;
                }
            }
        }
        if has_valid_max_age {
            assert!(
                cookie.expires().is_some(),
                "valid RFC 6265 Max-Age must set cookie expiry"
            );
        } else if !has_expires {
            assert!(
                cookie.expires().is_none(),
                "invalid Max-Age without Expires must not set cookie expiry"
            );
        }
    }
    black_box(parse_cookie_date(&line));
});
