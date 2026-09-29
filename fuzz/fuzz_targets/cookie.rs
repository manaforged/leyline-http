#![no_main]

use std::hint::black_box;

use leyline::fuzz::{parse_cookie_date, parse_set_cookie};
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
    drop(parse_set_cookie(&line, origin));
    black_box(parse_cookie_date(&line));
});
