#![no_main]

mod rfc_oracle;

use leyline::internals::fuzz::read_chunked_body;
use libfuzzer_sys::fuzz_target;
use rfc_oracle::{is_tchar, next_strict_crlf, trim_ows};
use tokio::runtime::Builder;

const LIMIT: usize = 1 << 20;
const MAX_LINE: usize = 16 * 1024;
const MAX_TRAILERS: usize = 64 * 1024;

fn is_valid_header_value_byte(b: u8) -> bool {
    b == b'\t' || (0x20..=0x7e).contains(&b) || b >= 0x80
}

fn parse_chunk_size(line: &[u8]) -> Option<usize> {
    let digits = line.iter().take_while(|b| b.is_ascii_hexdigit()).count();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    let after_bws = rest
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t'))
        .map_or(&[][..], |idx| &rest[idx..]);
    if !after_bws.is_empty() && after_bws[0] != b';' {
        return None;
    }
    let mut size: u64 = 0;
    for &b in &line[..digits] {
        let d = u64::from(char::from(b).to_digit(16)?);
        size = size.checked_mul(16)?.checked_add(d)?;
    }
    usize::try_from(size).ok()
}

fn rfc9112_chunked(mut wire: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let end = next_strict_crlf(wire, MAX_LINE)?;
        let size = parse_chunk_size(&wire[..end])?;
        wire = &wire[end + 2..];
        if size == 0 {
            break;
        }
        if size > LIMIT.saturating_sub(out.len()) || wire.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&wire[..size]);
        if &wire[size..size + 2] != b"\r\n" {
            return None;
        }
        wire = &wire[size + 2..];
    }
    let mut trailer_bytes = 0usize;
    loop {
        let end = next_strict_crlf(wire, MAX_LINE)?;
        trailer_bytes = trailer_bytes.checked_add(end + 2)?;
        if trailer_bytes > MAX_TRAILERS {
            return None;
        }
        let line = &wire[..end];
        wire = &wire[end + 2..];
        if line.is_empty() {
            return Some(out);
        }
        let colon = line.iter().position(|&b| b == b':')?;
        let name = &line[..colon];
        if name.is_empty() || !name.iter().copied().all(is_tchar) {
            return None;
        }
        let val = trim_ows(&line[colon + 1..]);
        if !val.iter().copied().all(is_valid_header_value_byte) {
            return None;
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let rt = Builder::new_current_thread()
        .build()
        .expect("current-thread runtime");
    let expected = rfc9112_chunked(data);
    rt.block_on(async {
        let mut wire = data;
        match read_chunked_body(&mut wire, Vec::new(), LIMIT).await {
            Ok(body) => {
                assert!(body.len() <= LIMIT);
                let oracle = expected.expect("parser accepted invalid RFC 9112 chunked body");
                assert_eq!(body, oracle);
            }
            Err(_) => {
                assert!(
                    expected.is_none(),
                    "parser rejected valid RFC 9112 chunked body"
                );
            }
        }
    });
});
