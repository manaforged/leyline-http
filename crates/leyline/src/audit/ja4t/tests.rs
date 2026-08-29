use super::*;

#[test]
fn ja4t_linux() {
    let fp = compute_ja4t(29200, 1460, 7, false);
    assert_eq!(fp, "29200_2-4-8-1-3_1460_7");
}

#[test]
fn ja4t_windows() {
    let fp = compute_ja4t(64240, 1460, 8, true);
    assert_eq!(fp, "64240_2-1-3-1-1-4_1460_8");
}
