use super::*;

#[test]
fn ja3_format() {
    let input = Ja3Input {
        ciphers: &[
            "TLS_AES_128_GCM_SHA256".into(),
            "TLS_RSA_WITH_AES_128_CBC_SHA".into(),
        ],
        curves: &["X25519".into(), "SECP256R1".into()],
        extension_ids: &[0x0000, 0x000a, 0x000b],
        tls_record_version: 771, // TLS 1.2
    };
    let hash = compute_ja3(&input);
    assert_eq!(hash.len(), 32); // MD5 hex length
}
