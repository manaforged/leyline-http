use std::borrow::Cow;

use super::{BodyHeaders, is_single_final_chunked};

#[kani::proof]
#[kani::unwind(5)]
fn non_ascii_content_length_is_always_rejected() {
    let bytes: [u8; 3] = kani::any();
    let len: usize = kani::any_where(|n| (1..=3).contains(n));
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
