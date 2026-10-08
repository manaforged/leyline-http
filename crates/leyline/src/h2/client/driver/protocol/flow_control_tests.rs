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
    let step1 = checked_window_add(0, MAX_FLOW_WINDOW).unwrap();
    checked_window_add(step1, MAX_FLOW_WINDOW).expect_err("expected Err");
}

#[test]
fn negative_delta_from_settings_is_permitted() {
    assert_eq!(checked_window_add(10_000, -20_000), Ok(-10_000));
    assert_eq!(
        checked_window_add(MAX_FLOW_WINDOW, -(MAX_FLOW_WINDOW + 1)),
        Ok(-1)
    );
}

#[test]
fn saturating_add_prevents_signed_overflow() {
    let res = checked_window_add(i64::MAX - 10, 100);
    res.expect_err("expected Err");
}
