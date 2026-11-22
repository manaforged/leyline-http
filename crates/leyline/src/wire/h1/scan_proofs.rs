use std::borrow::Cow;

use super::{BodyFraming, BodyHeaders, is_single_final_chunked};

#[kani::proof]
#[kani::unwind(6)]
fn non_ascii_content_length_is_always_rejected() {
    let bytes: [u8; 4] = kani::any();
    let len: usize = kani::any_where(|n| (1..=4).contains(n));
    let slice = &bytes[..len];
    kani::assume(slice.iter().any(|&b| b >= 127));
    let headers = BodyHeaders {
        lengths: 1,
        length: Some(Cow::Borrowed(slice)),
        coding: None,
    };
    assert!(headers.check_framing().is_err());
}

#[kani::proof]
#[kani::unwind(6)]
fn dual_content_length_and_transfer_encoding_is_always_rejected() {
    let cl: [u8; 2] = kani::any();
    let te: [u8; 2] = kani::any();
    let headers = BodyHeaders {
        lengths: 1,
        length: Some(Cow::Borrowed(&cl)),
        coding: Some(Cow::Borrowed(&te)),
    };
    assert!(headers.check_framing().is_err());
}

#[kani::proof]
fn transfer_encoding_requires_chunked_once_and_final() {
    assert!(is_single_final_chunked(b"chunked"));
    assert!(is_single_final_chunked(b"Chunked"));
    assert!(is_single_final_chunked(b"gzip, chunked"));
    assert!(!is_single_final_chunked(b"gzip"));
    assert!(!is_single_final_chunked(b"chunked, gzip"));
    assert!(!is_single_final_chunked(b"chunked, chunked"));
    assert!(!is_single_final_chunked(b"chunked, Chunked"));
    assert!(!is_single_final_chunked(b""));
    assert!(!is_single_final_chunked(b","));
}

#[kani::proof]
#[kani::unwind(10)]
fn valid_framing_matches_check_framing() {
    let to_close = BodyHeaders {
        lengths: 0,
        length: None,
        coding: None,
    };
    assert!(to_close.check_framing().is_ok());
    assert!(matches!(to_close.framing(), BodyFraming::ToClose));

    let fixed_zero = BodyHeaders {
        lengths: 1,
        length: Some(Cow::Borrowed(b"0")),
        coding: None,
    };
    assert!(fixed_zero.check_framing().is_ok());
    assert!(matches!(fixed_zero.framing(), BodyFraming::Fixed(0)));

    let fixed_val = BodyHeaders {
        lengths: 1,
        length: Some(Cow::Borrowed(b"42")),
        coding: None,
    };
    assert!(fixed_val.check_framing().is_ok());
    assert!(matches!(fixed_val.framing(), BodyFraming::Fixed(42)));

    let chunked = BodyHeaders {
        lengths: 0,
        length: None,
        coding: Some(Cow::Borrowed(b"chunked")),
    };
    assert!(chunked.check_framing().is_ok());
    assert!(matches!(chunked.framing(), BodyFraming::Chunked));

    let bad = BodyHeaders {
        lengths: 1,
        length: Some(Cow::Borrowed(b"bad")),
        coding: None,
    };
    assert!(bad.check_framing().is_err());
}
