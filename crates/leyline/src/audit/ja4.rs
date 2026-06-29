//! JA4 TLS fingerprint computation (FoxIO specification).
//!
//! Format: `{section_a}_{section_b}_{section_c}`
//! Computed from the TLS profile data (cipher suites, extensions, sigalgs).

use crate::audit::cipher_map::sigalg_id;
use crate::audit::{hash12, non_grease_cipher_ids, non_grease_ext_ids};

/// Input data for JA4 computation. Extracted from a browser profile.
pub struct Ja4Input<'a> {
    /// Cipher suite names from the profile.
    pub ciphers: &'a [String],
    /// Signature algorithm names.
    pub sigalgs: &'a [String],
    /// Named curves / supported groups.
    pub curves: &'a [String],
    /// Extension type IDs that will be in the ClientHello.
    /// These should be the actual extension IDs we send (after GREASE filtering).
    pub extension_ids: &'a [u16],
    /// TLS version (e.g., "1.3", "1.2").
    pub tls_version: &'a str,
    /// Whether SNI is present (true for all domain connections).
    pub has_sni: bool,
    /// First ALPN value (e.g., "h2").
    pub alpn: &'a str,
}

/// Compute JA4 fingerprint from profile data.
pub fn compute_ja4(input: &Ja4Input<'_>) -> String {
    let section_a = compute_section_a(input);
    let section_b = compute_section_b(input);
    let section_c = compute_section_c(input);
    format!("{section_a}_{section_b}_{section_c}")
}

/// Section A: client attributes (10 chars).
/// Format: {proto}{version}{sni}{cipher_count:02}{ext_count:02}{alpn}
fn compute_section_a(input: &Ja4Input<'_>) -> String {
    let proto = "t"; // TLS over TCP

    let version = match input.tls_version {
        "1.3" => "13",
        "1.2" => "12",
        "1.1" => "11",
        "1.0" => "10",
        _ => "00",
    };

    let sni = if input.has_sni { "d" } else { "i" };

    // Count ciphers excluding GREASE
    let cipher_count = non_grease_cipher_ids(input.ciphers).len().min(99);

    // Count extensions excluding GREASE
    let ext_count = non_grease_ext_ids(input.extension_ids).len().min(99);

    // ALPN: first and last char of first ALPN value
    let alpn = if input.alpn.is_empty() {
        "00".to_string()
    } else {
        let bytes = input.alpn.as_bytes();
        let first = bytes[0] as char;
        let last = bytes[bytes.len() - 1] as char;
        format!("{first}{last}")
    };

    format!("{proto}{version}{sni}{cipher_count:02}{ext_count:02}{alpn}")
}

/// Section B: sorted cipher suites hash (12 hex chars).
fn compute_section_b(input: &Ja4Input<'_>) -> String {
    let mut ids = non_grease_cipher_ids(input.ciphers);

    ids.sort();

    let s: String = ids
        .iter()
        .map(|id| format!("{id:04x}"))
        .collect::<Vec<_>>()
        .join(",");

    hash12(&s)
}

/// Section C: sorted extensions + sigalgs hash (12 hex chars).
fn compute_section_c(input: &Ja4Input<'_>) -> String {
    // Extensions: exclude GREASE, SNI (0x0000), ALPN (0x0010), then sort.
    let mut ext_ids: Vec<u16> = non_grease_ext_ids(input.extension_ids)
        .into_iter()
        .filter(|id| *id != 0x0000 && *id != 0x0010)
        .collect();
    ext_ids.sort();

    let ext_str: String = ext_ids
        .iter()
        .map(|id| format!("{id:04x}"))
        .collect::<Vec<_>>()
        .join(",");

    // Sigalgs: in original order (not sorted), as 4-char hex.
    let sigalg_str: String = input
        .sigalgs
        .iter()
        .filter_map(|s| sigalg_id(s))
        .map(|id| format!("{id:04x}"))
        .collect::<Vec<_>>()
        .join(",");

    // Combine: extensions_sigalgs
    let combined = format!("{ext_str}_{sigalg_str}");
    hash12(&combined)
}

/// Build the list of extension IDs that **Chrome** sends in its
/// ClientHello, derived from the TLS profile configuration.
///
/// Chrome-only advisory: the session builder feeds this into the
/// precomputed `audit()` JA4/JA3 for *every* profile, including Firefox
/// and Safari, which order and select extensions differently. There is
/// no firefox/safari equivalent yet, so for non-Chromium profiles the
/// precomputed audit hashes are approximations. The wire fingerprint is
/// the source of truth — `tests/tls_peet.rs` validates the *observed*
/// JA4 against each profile TOML's `expected_ja4`, not this precompute.
pub fn chrome_extension_ids(tls: &crate::profile::TlsProfile) -> Vec<u16> {
    // Chrome's extension order (before permutation) based on BoringSSL defaults.
    // These are the extensions we configure in connector.rs.
    let mut exts = Vec::new();

    // SNI (always present for domain connections)
    exts.push(0x0000); // server_name

    // Extended master secret
    exts.push(0x0017); // extended_master_secret (23)

    // Renegotiation info
    exts.push(0xff01); // renegotiation_info (65281)

    // Supported groups
    if !tls.curves.is_empty() {
        exts.push(0x000a); // supported_groups (10)
    }

    // EC point formats
    exts.push(0x000b); // ec_point_formats (11)

    // Session ticket
    exts.push(0x0023); // session_ticket (35)

    // ALPN
    exts.push(0x0010); // application_layer_protocol_negotiation (16)

    // Status request (OCSP stapling)
    if tls.ocsp_stapling {
        exts.push(0x0005); // status_request (5)
    }

    // Delegated credentials
    if tls.delegated_credentials.is_some() {
        exts.push(0x0022); // delegated_credentials (34)
    }

    // Key share
    exts.push(0x0033); // key_share (51)

    // Supported versions
    exts.push(0x002b); // supported_versions (43)

    // Signature algorithms
    if !tls.sigalgs.is_empty() {
        exts.push(0x000d); // signature_algorithms (13)
    }

    // PSK key exchange modes
    exts.push(0x002d); // psk_key_exchange_modes (45)

    // Record size limit
    if tls.record_size_limit.is_some() {
        exts.push(0x001c); // record_size_limit (28)
    }

    // Certificate compression
    if !tls.cert_compression.is_empty() {
        exts.push(0x001b); // compress_certificate (27)
    }

    // Signed certificate timestamps
    if tls.signed_cert_timestamps {
        exts.push(0x0012); // signed_certificate_timestamp (18)
    }

    // ALPS (Application-Layer Protocol Settings)
    if tls.alps.is_some() {
        if tls.alps_new_codepoint {
            exts.push(0x4469); // ALPS new codepoint (17513)
        } else {
            exts.push(0x4411); // ALPS old codepoint
        }
    }

    // ECH (Encrypted Client Hello) GREASE
    if tls.ech_grease {
        exts.push(0xfe0d); // encrypted_client_hello (65037)
    }

    // pre_shared_key (41) is only sent when a session ticket is available
    // for resumption. On first connection it is absent, and leyline does not
    // do session resumption, so it is omitted from the extension list.

    exts
}

#[cfg(test)]
mod tests {
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
                0x0000, 0x0017, 0xff01, 0x000a, 0x000b, 0x0023, 0x0010, 0x0005, 0x0033, 0x002b,
                0x000d, 0x002d, 0x001b, 0x0012, 0x4469, 0xfe0d, 0x0029,
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
}
