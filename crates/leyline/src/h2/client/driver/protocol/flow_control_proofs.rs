use super::{MAX_FLOW_WINDOW, checked_window_add};

#[kani::proof]
fn a_flow_window_is_exact_and_never_passes_the_protocol_maximum() {
    let current: i64 =
        kani::any_where(|window: &i64| (-MAX_FLOW_WINDOW - 1..=MAX_FLOW_WINDOW).contains(window));
    let delta: i64 =
        kani::any_where(|delta: &i64| (-MAX_FLOW_WINDOW..=MAX_FLOW_WINDOW).contains(delta));
    match checked_window_add(current, delta) {
        Ok(window) => {
            assert!(window <= MAX_FLOW_WINDOW);
            assert_eq!(window, current + delta);
        }
        Err(_) => assert!(current + delta > MAX_FLOW_WINDOW),
    }
}
