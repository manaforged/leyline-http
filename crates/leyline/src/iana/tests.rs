use super::*;

#[test]
fn grease_detection() {
    assert!(is_grease(0x0a0a));
    assert!(is_grease(0x1a1a));
    assert!(is_grease(0xfafa));
    assert!(!is_grease(0x1301));
    assert!(!is_grease(0x0000));
}
