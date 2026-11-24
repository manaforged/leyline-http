#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    leyline::internals::fuzz::parse_dns_response(data);
});
