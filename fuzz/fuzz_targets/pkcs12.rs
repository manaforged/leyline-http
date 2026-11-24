#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&pass_len, rest)) = data.split_first() else {
        return;
    };
    let split = usize::from(pass_len & 0x1f).min(rest.len());
    let (pass_bytes, der) = rest.split_at(split);
    let password = String::from_utf8_lossy(pass_bytes);
    leyline::internals::fuzz::parse_pkcs12(der, &password);
});
