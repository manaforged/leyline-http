#![no_main]

use leyline::internals::fuzz::parse_h1_head;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok((status, headers, minor)) = parse_h1_head(data) {
        assert!((100..=999).contains(&status));
        assert!(minor <= 1);
        assert!(
            !(headers.contains_key("content-length") && headers.contains_key("transfer-encoding"))
        );
        assert!(headers.get_all("content-length").iter().count() <= 1);
    }
});
