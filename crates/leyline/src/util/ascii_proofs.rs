use super::{bare_host, delta_seconds, has_ascii_uppercase_bytes, parse_ascii_u64};

#[kani::proof]
#[kani::unwind(18)]
fn has_ascii_uppercase_swar_matches_byte_scan() {
    let buf: [u8; 16] = kani::any();
    let len: usize = kani::any_where(|&n| n <= 16);
    let slice = &buf[..len];
    assert_eq!(
        has_ascii_uppercase_bytes(slice),
        slice.iter().any(u8::is_ascii_uppercase)
    );
}

#[kani::proof]
#[kani::unwind(5)]
fn parse_ascii_u64_and_delta_seconds_match_spec() {
    let buf: [u8; 3] = kani::any();
    let len: usize = kani::any_where(|&n| n <= 3);
    let slice = &buf[..len];
    kani::assume(slice.iter().all(|b| b.is_ascii()));
    let s = std::str::from_utf8(slice).expect("ascii");
    let all_digits = !slice.is_empty() && slice.iter().all(u8::is_ascii_digit);
    assert_eq!(parse_ascii_u64(s).is_some(), all_digits);
    let trimmed = s.trim();
    let trimmed_digits = !trimmed.is_empty() && trimmed.bytes().all(|b| b.is_ascii_digit());
    assert_eq!(delta_seconds(s).is_some(), trimmed_digits);
    if all_digits {
        assert_eq!(parse_ascii_u64(s), delta_seconds(s));
    }
}

#[kani::proof]
#[kani::unwind(8)]
fn bare_host_strips_only_matched_brackets() {
    let buf: [u8; 6] = kani::any();
    let len: usize = kani::any_where(|&n| n <= 6);
    let slice = &buf[..len];
    kani::assume(slice.iter().all(|b| b.is_ascii()));
    let s = std::str::from_utf8(slice).expect("ascii");
    let out = bare_host(s);
    if len >= 2 && slice[0] == b'[' && slice[len - 1] == b']' {
        assert_eq!(out.as_bytes(), &slice[1..len - 1]);
    } else {
        assert_eq!(out, s);
    }
}
