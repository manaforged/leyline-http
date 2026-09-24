use super::*;

#[test]
fn cipher_ids_correct() {
    assert_eq!(cipher_id("TLS_AES_128_GCM_SHA256"), Some(0x1301));
    assert_eq!(
        cipher_id("TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256"),
        Some(0xc02f)
    );
    assert_eq!(cipher_id("UNKNOWN"), None);
    assert_eq!(curve_id("X25519_KYBER768"), Some(0x6399));
    assert_eq!(curve_id("X25519Kyber768Draft00"), Some(0x6399));
}

#[test]
fn grease_detection() {
    assert!(is_grease(0x0a0a));
    assert!(is_grease(0x1a1a));
    assert!(is_grease(0xfafa));
    assert!(!is_grease(0x1301));
    assert!(!is_grease(0x0000));
}
