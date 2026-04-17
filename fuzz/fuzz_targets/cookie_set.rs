#![no_main]
//! Fuzz Set-Cookie parsing via `CookieJar::store_set_cookie`.
//!
//! Pairs arbitrary bytes (interpreted as UTF-8 where valid, else skipped)
//! with a stable origin URL so parser edge cases — odd whitespace, long
//! attribute strings, malformed Expires values, pathological Max-Age —
//! all reach the parser without host-origin effects masking bugs.

use leyline_cookies::CookieJar;
use libfuzzer_sys::fuzz_target;
use url::Url;

fuzz_target!(|data: &[u8]| {
    let Ok(header) = std::str::from_utf8(data) else {
        return;
    };
    let url = Url::parse("https://example.test/").expect("static URL");
    let jar = CookieJar::new();
    jar.store_set_cookie(header, &url);
    // Also exercise the sibling read path so parser+jar interaction fuzzes.
    let _ = jar.cookie_header(&url);
});
