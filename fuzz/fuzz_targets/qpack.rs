#![no_main]

use leyline_quiche::h3::qpack::Decoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut decoder = Decoder::new();
    drop(decoder.decode(data, 65536));
});
