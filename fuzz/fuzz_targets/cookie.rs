#![no_main]

use std::hint::black_box;

use leyline::cookie::SameSite;
use leyline::internals::fuzz::{parse_cookie_date, parse_set_cookie};
use libfuzzer_sys::fuzz_target;

const ORIGINS: [&str; 4] = [
    "https://www.example.com/a/b",
    "http://localhost:8080/",
    "https://example.co.uk/",
    "https://127.0.0.1/x",
];

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
    }
    black_box(parse_cookie_date(&line));
});
