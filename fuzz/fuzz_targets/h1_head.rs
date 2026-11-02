#![no_main]

use leyline::fuzz::parse_h1_head;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(parse_h1_head(data));
});
