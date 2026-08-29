//! `WINDOW_UPDATE`, SETTINGS, and the handshake path all reuse the
//! same flow-window math via `checked_window_add`, giving all three
//! call sites a single unit-test gate. If this helper ever returns
//! `Ok` for a post-cap value, three RFC 9113 §6.9 invariants collapse
//! simultaneously.

use super::{MAX_FLOW_WINDOW, checked_window_add};

#[test]
fn exact_cap_is_ok() {
    assert_eq!(checked_window_add(0, MAX_FLOW_WINDOW), Ok(MAX_FLOW_WINDOW));
    assert_eq!(
        checked_window_add(65_535, MAX_FLOW_WINDOW - 65_535),
        Ok(MAX_FLOW_WINDOW)
    );
}

#[test]
fn cap_plus_one_errors_with_post_add_value() {
    assert_eq!(
        checked_window_add(MAX_FLOW_WINDOW, 1),
        Err(MAX_FLOW_WINDOW + 1)
    );
}

#[test]
fn two_max_increments_reject() {
    // Classic CVE-shape attack: WINDOW_UPDATE(0, 0x7FFFFFFF) twice
    // climbs the accumulator past the 2^31-1 ceiling.
    let step1 = checked_window_add(0, MAX_FLOW_WINDOW).unwrap();
    assert!(checked_window_add(step1, MAX_FLOW_WINDOW).is_err());
}

#[test]
fn negative_delta_from_settings_is_permitted() {
    // RFC 9113 §6.9.2: a SETTINGS frame can drive the stream
    // window *negative*. The helper must NOT confuse that with
    // overflow — only the positive-cap is enforced.
    assert_eq!(checked_window_add(10_000, -20_000), Ok(-10_000));
    assert_eq!(
        checked_window_add(MAX_FLOW_WINDOW, -(MAX_FLOW_WINDOW + 1)),
        Ok(-1)
    );
}

#[test]
fn saturating_add_prevents_signed_overflow() {
    // A genuinely pathological peer sending multiple max-sized
    // increments must not crash us via i64 overflow before the
    // cap check — `saturating_add` pins to i64::MAX which is
    // still > MAX_FLOW_WINDOW and trips the error branch.
    let res = checked_window_add(i64::MAX - 10, 100);
    assert!(res.is_err());
}
