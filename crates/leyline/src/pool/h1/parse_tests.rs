use super::{chunk_size, status_line};
use crate::util::every_input;

const VERSIONS: [&[u8]; 6] = [
    b"HTTP/1.0",
    b"HTTP/1.1",
    b"HTTP/1.2",
    b"HTTP/1.10",
    b"HTTP/2.0",
    b"http/1.1",
];
const TAIL: [&[u8]; 8] = [b" ", b"0", b"1", b"2", b"9", b"+", b"-", b"x"];

fn rfc9112(version: &[u8], tail: &[u8]) -> Option<(u16, u8)> {
    let minor = match version {
        b"HTTP/1.0" => 0,
        b"HTTP/1.1" => 1,
        _ => return None,
    };
    let code = tail.strip_prefix(b" ")?;
    let digits = code.get(..3)?;
    let rest = &code[3..];
    let three_digits =
        (b'1'..=b'9').contains(&digits[0]) && digits[1..].iter().all(u8::is_ascii_digit);
    if !three_digits || !(rest.is_empty() || rest[0] == b' ') {
        return None;
    }
    let status = digits
        .iter()
        .fold(0u16, |status, digit| status * 10 + u16::from(digit - b'0'));
    Some((status, minor))
}

#[test]
fn every_short_status_line_matches_the_rfc_9112_grammar() {
    for version in VERSIONS {
        every_input(&TAIL, 6, |tail| {
            let line = [version, tail].concat();
            assert_eq!(
                status_line(&line).ok(),
                rfc9112(version, tail),
                "{:?}",
                String::from_utf8_lossy(&line)
            );
        });
    }
}

/// RFC 9112 section 7.1: 1*HEXDIG, then BWS before an ignored `;` extension.
fn rfc9112_chunk_size(line: &[u8]) -> Option<u64> {
    let digits = line
        .iter()
        .take_while(|byte| byte.is_ascii_hexdigit())
        .count();
    let rest = line[digits..].trim_ascii_start();
    let bws = line.len() - digits - rest.len();
    let blank = line[digits..digits + bws]
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\t'));
    if digits == 0 || !blank || !(rest.is_empty() || rest[0] == b';') {
        return None;
    }
    let value = line[..digits].iter().try_fold(0u128, |value, &byte| {
        let digit = u128::from(char::from(byte).to_digit(16)?);
        Some(value * 16 + digit)
    })?;
    u64::try_from(value).ok()
}

#[test]
fn every_short_chunk_size_line_matches_the_rfc_9112_grammar() {
    let alphabet: [&[u8]; 12] = [
        b"0", b"1", b"a", b"F", b"g", b" ", b"\t", b";", b"x", b"=", b"\x0c", b"\xff",
    ];
    every_input(&alphabet, 5, |line| {
        assert_eq!(
            chunk_size(line).ok(),
            rfc9112_chunk_size(line),
            "{line:02x?}"
        );
    });
    for line in [
        &b"ffffffffffffffff"[..],
        b"10000000000000000",
        b"00000000000000000001",
        b"0000000000000000000000ffffffffffffffff",
    ] {
        assert_eq!(
            chunk_size(line).ok(),
            rfc9112_chunk_size(line),
            "{line:02x?}"
        );
    }
}
