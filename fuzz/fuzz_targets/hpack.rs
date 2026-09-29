#![no_main]

use leyline::h2::hpack::Decoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut decoder = Decoder::new();
    let _ = decoder.decode_header_block(data);
});
