//! HTTP/1.1 response head fuzz: arbitrary bytes → `parse_h1_head` must never panic.
#![no_main]

use leyline::fuzz::parse_h1_head;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let head = String::from_utf8_lossy(data);
    drop(parse_h1_head(&head));
});
