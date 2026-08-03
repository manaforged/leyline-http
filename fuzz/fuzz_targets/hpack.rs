//! HPACK decoder fuzz: arbitrary bytes → `Decoder::decode_header_block` must
//! never panic. Decode errors are the correct outcome for malformed input —
//! panics are the bug.
//!
//! Cross-block dynamic-table state is exercised by the `h2_continuation`
//! target, which drives one persistent decoder over many blocks.
#![no_main]

use leyline::h2::hpack::Decoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut decoder = Decoder::new();
    let _ = decoder.decode_header_block(data);
});
