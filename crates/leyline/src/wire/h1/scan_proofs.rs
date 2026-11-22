use std::borrow::Cow;

use super::BodyHeaders;

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
