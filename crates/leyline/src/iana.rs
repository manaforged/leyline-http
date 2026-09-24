const CIPHERS: &[(&str, u16)] = &[
    ("TLS_AES_128_GCM_SHA256", 0x1301),
    ("TLS_AES_256_GCM_SHA384", 0x1302),
    ("TLS_CHACHA20_POLY1305_SHA256", 0x1303),
    ("TLS_AES_128_CCM_SHA256", 0x1304),
    ("TLS_AES_128_CCM_8_SHA256", 0x1305),
    ("TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256", 0xc02b),
    ("TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384", 0xc02c),
    ("TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256", 0xcca9),
    ("TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA", 0xc009),
    ("TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA", 0xc00a),
    ("TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA256", 0xc023),
    ("TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA384", 0xc024),
    ("TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256", 0xc02f),
    ("TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384", 0xc030),
    ("TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256", 0xcca8),
    ("TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA", 0xc013),
    ("TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA", 0xc014),
    ("TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA256", 0xc027),
    ("TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA384", 0xc028),
    ("TLS_RSA_WITH_AES_128_GCM_SHA256", 0x009c),
    ("TLS_RSA_WITH_AES_256_GCM_SHA384", 0x009d),
    ("TLS_RSA_WITH_AES_128_CBC_SHA", 0x002f),
    ("TLS_RSA_WITH_AES_256_CBC_SHA", 0x0035),
    ("TLS_RSA_WITH_AES_128_CBC_SHA256", 0x003c),
    ("TLS_RSA_WITH_AES_256_CBC_SHA256", 0x003d),
    ("TLS_RSA_WITH_3DES_EDE_CBC_SHA", 0x000a),
    ("TLS_ECDHE_ECDSA_WITH_3DES_EDE_CBC_SHA", 0xc008),
    ("TLS_ECDHE_RSA_WITH_3DES_EDE_CBC_SHA", 0xc012),
];

const TLS13_CIPHERS: std::ops::RangeInclusive<u16> = 0x1301..=0x1303;

const SIGALGS: &[(&str, u16)] = &[
    ("ecdsa_secp256r1_sha256", 0x0403),
    ("ecdsa_secp384r1_sha384", 0x0503),
    ("ecdsa_secp521r1_sha512", 0x0603),
    ("ed25519", 0x0807),
    ("ed448", 0x0808),
    ("rsa_pss_pss_sha256", 0x0809),
    ("rsa_pss_pss_sha384", 0x080a),
    ("rsa_pss_pss_sha512", 0x080b),
    ("rsa_pss_rsae_sha256", 0x0804),
    ("rsa_pss_rsae_sha384", 0x0805),
    ("rsa_pss_rsae_sha512", 0x0806),
    ("rsa_pkcs1_sha256", 0x0401),
    ("rsa_pkcs1_sha384", 0x0501),
    ("rsa_pkcs1_sha512", 0x0601),
    ("rsa_pkcs1_sha1", 0x0201),
    ("ecdsa_sha1", 0x0203),
    ("mldsa44", 0x0904),
    ("mldsa65", 0x0905),
    ("mldsa87", 0x0906),
];

struct Curve {
    name: &'static str,
    boring: &'static str,
    id: u16,
}

const CURVES: &[Curve] = &[
    Curve {
        name: "SECP256R1",
        boring: "P-256",
        id: 0x0017,
    },
    Curve {
        name: "SECP384R1",
        boring: "P-384",
        id: 0x0018,
    },
    Curve {
        name: "SECP521R1",
        boring: "P-521",
        id: 0x0019,
    },
    Curve {
        name: "X25519",
        boring: "X25519",
        id: 0x001d,
    },
    Curve {
        name: "X448",
        boring: "X448",
        id: 0x001e,
    },
    Curve {
        name: "X25519_KYBER768",
        boring: "X25519Kyber768Draft00",
        id: 0x6399,
    },
    Curve {
        name: "X25519_MLKEM768",
        boring: "X25519MLKEM768",
        id: 0x11ec,
    },
];

fn lookup(table: &[(&str, u16)], name: &str) -> Option<u16> {
    table.iter().find(|(n, _)| *n == name).map(|&(_, id)| id)
}

fn curve(name: &str) -> Option<&'static Curve> {
    CURVES.iter().find(|c| c.name == name || c.boring == name)
}

pub(crate) fn cipher_id(name: &str) -> Option<u16> {
    lookup(CIPHERS, name)
}

pub(crate) fn is_tls13_cipher(id: u16) -> bool {
    TLS13_CIPHERS.contains(&id)
}

pub(crate) fn sigalg_id(name: &str) -> Option<u16> {
    lookup(SIGALGS, name)
}

pub(crate) fn curve_id(name: &str) -> Option<u16> {
    curve(name).map(|c| c.id)
}

pub(crate) fn boring_curve_name(name: &str) -> &str {
    curve(name).map_or(name, |c| c.boring)
}

pub(crate) fn is_grease(val: u16) -> bool {
    val & 0x0f0f == 0x0a0a
}

#[cfg(test)]
mod tests;
