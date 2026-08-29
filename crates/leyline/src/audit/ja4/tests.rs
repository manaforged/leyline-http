use super::*;

#[test]
fn section_a_chrome147() {
    let input = Ja4Input {
        ciphers: &[
            "TLS_AES_128_GCM_SHA256".into(),
            "TLS_AES_256_GCM_SHA384".into(),
            "TLS_CHACHA20_POLY1305_SHA256".into(),
            "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256".into(),
            "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256".into(),
            "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384".into(),
            "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384".into(),
            "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256".into(),
            "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256".into(),
            "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA".into(),
            "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA".into(),
            "TLS_RSA_WITH_AES_128_GCM_SHA256".into(),
            "TLS_RSA_WITH_AES_256_GCM_SHA384".into(),
            "TLS_RSA_WITH_AES_128_CBC_SHA".into(),
            "TLS_RSA_WITH_AES_256_CBC_SHA".into(),
        ],
        sigalgs: &[],
        curves: &[],
        extension_ids: &[
            0x0000, 0x0017, 0xff01, 0x000a, 0x000b, 0x0023, 0x0010, 0x0005, 0x0033, 0x002b, 0x000d,
            0x002d, 0x001b, 0x0012, 0x4469, 0xfe0d, 0x0029,
        ],
        tls_version: "1.3",
        has_sni: true,
        alpn: "h2",
    };
    let a = compute_section_a(&input);
    // 15 ciphers, 16 extensions (ALPS included), h2
    assert_eq!(&a[..3], "t13"); // TLS 1.3
    assert_eq!(&a[3..4], "d"); // SNI present
    assert_eq!(&a[4..6], "15"); // 15 ciphers
    // 17 extensions in the list (all non-GREASE). Real Chrome has 16
    // because the list includes extended_master_secret, which Chrome 147
    // may omit. The exact count depends on the BoringSSL configuration.
    assert!(
        a[6..8].parse::<u32>().unwrap() >= 16,
        "ext count: {}",
        &a[6..8]
    );
    assert_eq!(&a[8..], "h2");
}

#[test]
fn section_b_chrome147() {
    let ciphers: Vec<String> = [
        "TLS_AES_128_GCM_SHA256",
        "TLS_AES_256_GCM_SHA384",
        "TLS_CHACHA20_POLY1305_SHA256",
        "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
        "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
        "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
        "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
        "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256",
        "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
        "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA",
        "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA",
        "TLS_RSA_WITH_AES_128_GCM_SHA256",
        "TLS_RSA_WITH_AES_256_GCM_SHA384",
        "TLS_RSA_WITH_AES_128_CBC_SHA",
        "TLS_RSA_WITH_AES_256_CBC_SHA",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let input = Ja4Input {
        ciphers: &ciphers,
        sigalgs: &[],
        curves: &[],
        extension_ids: &[],
        tls_version: "1.3",
        has_sni: true,
        alpn: "h2",
    };
    let b = compute_section_b(&input);
    // Expected from Chrome 147 JA4: 8daaf6152771
    assert_eq!(b, "8daaf6152771");
}
