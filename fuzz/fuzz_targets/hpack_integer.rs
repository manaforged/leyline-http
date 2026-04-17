#![no_main]
//! Fuzz HPACK integer + header-block decoding.
//!
//! The integer primitive is exercised through `Decoder::decode_header_block`
//! because the `integer` module is crate-private. Malformed varints, huffman
//! strings, and table refs all flow through the same entry point.

use leyline_h2::hpack::Decoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut decoder = Decoder::new();
    let _ = decoder.decode_header_block(data);
});
