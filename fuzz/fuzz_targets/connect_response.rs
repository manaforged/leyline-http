//! Proxy CONNECT response fuzz: arbitrary proxy replies → `validate_connect_response` must never panic.
#![no_main]

use leyline::fuzz::validate_connect_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
        return;
    };
    drop(validate_connect_response(data, end + 4));
});
