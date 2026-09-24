pub(crate) fn cipher_id(name: &str) -> Option<u16> {
    Some(match name {
        "TLS_AES_128_GCM_SHA256" => 0x1301,
        "TLS_AES_256_GCM_SHA384" => 0x1302,
        "TLS_CHACHA20_POLY1305_SHA256" => 0x1303,
        "TLS_AES_128_CCM_SHA256" => 0x1304,
        "TLS_AES_128_CCM_8_SHA256" => 0x1305,

        "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256" => 0xc02b,
        "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384" => 0xc02c,
        "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256" => 0xcca9,
        "TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA" => 0xc009,
        "TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA" => 0xc00a,
        "TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA256" => 0xc023,
        "TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA384" => 0xc024,

        "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256" => 0xc02f,
        "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384" => 0xc030,
        "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256" => 0xcca8,
        "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA" => 0xc013,
        "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA" => 0xc014,
        "TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA256" => 0xc027,
        "TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA384" => 0xc028,

        "TLS_RSA_WITH_AES_128_GCM_SHA256" => 0x009c,
        "TLS_RSA_WITH_AES_256_GCM_SHA384" => 0x009d,
        "TLS_RSA_WITH_AES_128_CBC_SHA" => 0x002f,
        "TLS_RSA_WITH_AES_256_CBC_SHA" => 0x0035,
        "TLS_RSA_WITH_AES_128_CBC_SHA256" => 0x003c,
        "TLS_RSA_WITH_AES_256_CBC_SHA256" => 0x003d,
        "TLS_RSA_WITH_3DES_EDE_CBC_SHA" => 0x000a,
        "TLS_ECDHE_ECDSA_WITH_3DES_EDE_CBC_SHA" => 0xc008,
        "TLS_ECDHE_RSA_WITH_3DES_EDE_CBC_SHA" => 0xc012,

        _ => return None,
    })
}

pub(crate) fn sigalg_id(name: &str) -> Option<u16> {
    Some(match name {
        "ecdsa_secp256r1_sha256" => 0x0403,
        "ecdsa_secp384r1_sha384" => 0x0503,
        "ecdsa_secp521r1_sha512" => 0x0603,
        "ed25519" => 0x0807,
        "ed448" => 0x0808,
        "rsa_pss_pss_sha256" => 0x0809,
        "rsa_pss_pss_sha384" => 0x080a,
        "rsa_pss_pss_sha512" => 0x080b,
        "rsa_pss_rsae_sha256" => 0x0804,
        "rsa_pss_rsae_sha384" => 0x0805,
        "rsa_pss_rsae_sha512" => 0x0806,
        "rsa_pkcs1_sha256" => 0x0401,
        "rsa_pkcs1_sha384" => 0x0501,
        "rsa_pkcs1_sha512" => 0x0601,
        "rsa_pkcs1_sha1" => 0x0201,
        "ecdsa_sha1" => 0x0203,
        "mldsa44" => 0x0904,
        "mldsa65" => 0x0905,
        "mldsa87" => 0x0906,
        _ => return None,
    })
}

pub(crate) fn curve_id(name: &str) -> Option<u16> {
    Some(match name {
        "SECP256R1" | "P-256" => 0x0017,
        "SECP384R1" | "P-384" => 0x0018,
        "SECP521R1" | "P-521" => 0x0019,
        "X25519" => 0x001d,
        "X25519_KYBER768" | "X25519Kyber768Draft00" => 0x6399,
        "X25519_MLKEM768" | "X25519MLKEM768" => 0x11ec,
        "X448" => 0x001e,
        _ => return None,
    })
}

pub(crate) fn is_grease(val: u16) -> bool {
    val & 0x0f0f == 0x0a0a
}

#[cfg(test)]
mod tests;
