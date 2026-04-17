#![no_main]
//! Fuzz HPACK header-block decoding with a persistent dynamic-table state
//! across two consecutive decode calls.
//!
//! The split-input model lets libFuzzer discover inputs where the first
//! block mutates the table and the second block references the mutated
//! state (e.g. evicts then re-references an entry).

use leyline_h2::hpack::Decoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    // Split point is encoded in the first byte so libFuzzer can steer it.
    let split = (data[0] as usize).min(data.len().saturating_sub(1));
    let (first, rest) = data[1..].split_at(split.min(data.len() - 1));

    let mut decoder = Decoder::new();
    let _ = decoder.decode_header_block(first);
    let _ = decoder.decode_header_block(rest);
});
