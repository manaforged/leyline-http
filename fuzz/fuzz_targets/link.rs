#![no_main]

use std::hint::black_box;

use leyline::fuzz::parse_links;
use libfuzzer_sys::fuzz_target;

const BASES: [&str; 3] = [
    "https://www.example.com/a/b",
    "http://localhost:8080/",
    "https://127.0.0.1/x?page=2",
];

fuzz_target!(|data: &[u8]| {
    let Some((pick, rest)) = data.split_first() else {
        return;
    };
    let header = String::from_utf8_lossy(rest);
    let base = BASES[*pick as usize % BASES.len()];
    black_box(parse_links(&header, base));
});
