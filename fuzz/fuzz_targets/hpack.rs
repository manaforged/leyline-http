#![no_main]

use leyline::internals::h2::hpack::{Decoder, Encoder};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut decoder = Decoder::new();
    if let Ok(headers) = decoder.decode_header_block(data) {
        let pairs: Option<Vec<(&str, &str)>> = headers
            .iter()
            .map(|h| {
                Some((
                    std::str::from_utf8(&h.name).ok()?,
                    std::str::from_utf8(&h.value).ok()?,
                ))
            })
            .collect();
        if let Some(pairs) = pairs {
            let mut encoder = Encoder::new();
            let encoded = encoder.encode_header_block(&pairs);
            let mut verify = Decoder::new();
            let decoded = verify
                .decode_header_block(&encoded)
                .expect("re-encoded HPACK block must decode");
            assert_eq!(decoded, headers);
        }
    }
});
