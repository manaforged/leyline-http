#![no_main]

mod rfc_oracle;

use leyline::internals::fuzz::parse_h1_head;
use libfuzzer_sys::fuzz_target;
use rfc_oracle::{is_tchar, split_crlf_line};

fuzz_target!(|data: &[u8]| {
    if let Ok((status, headers, minor)) = parse_h1_head(data) {
        assert!((200..=999).contains(&status));
        assert!(minor <= 1);
        assert!(
            !(headers.contains_key("content-length") && headers.contains_key("transfer-encoding"))
        );
        assert!(headers.get_all("content-length").iter().count() <= 1);
        for (_, value) in &headers {
            let bytes = value.as_bytes();
            assert!(!matches!(bytes.first(), Some(b' ' | b'\t')));
            assert!(!matches!(bytes.last(), Some(b' ' | b'\t')));
        }
        let (_, mut rest) = split_crlf_line(data);
        while !rest.is_empty() {
            let (line, next) = split_crlf_line(rest);
            rest = next;
            if line.is_empty() || matches!(line[0], b' ' | b'\t') {
                continue;
            }
            let colon = line
                .iter()
                .position(|&b| b == b':')
                .expect("accepted header line must contain colon");
            let name = &line[..colon];
            assert!(
                !name.is_empty() && name.iter().copied().all(is_tchar),
                "accepted header line must have valid RFC 9110 token name"
            );
        }
    }
});
