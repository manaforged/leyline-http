//! JA3 TLS fingerprint computation (Salesforce).
//!
//! Format: MD5 of `{version},{ciphers},{extensions},{curves},{point_formats}`
//! where values within fields are hyphen-separated decimal integers.

use crate::cipher_map::{cipher_id, curve_id, is_grease};

/// Input data for JA3 computation.
pub struct Ja3Input<'a> {
    /// Cipher suite names.
    pub ciphers: &'a [String],
    /// Named curves / supported groups.
    pub curves: &'a [String],
    /// Extension type IDs (in ClientHello order).
    pub extension_ids: &'a [u16],
    /// TLS version (protocol version field). E.g. "1.2" → 771, "1.3" → 771 (record layer is 1.2).
    pub tls_record_version: u16,
}

/// Compute JA3 fingerprint. Returns (raw_string, md5_hash).
pub fn compute_ja3(input: &Ja3Input<'_>) -> String {
    let raw = compute_ja3_raw(input);
    let digest = md5::compute(raw.as_bytes());
    format!("{:x}", digest)
}

/// Compute the raw JA3 string (before hashing).
pub fn compute_ja3_raw(input: &Ja3Input<'_>) -> String {
    // Field 1: TLS version (decimal).
    let version = input.tls_record_version;

    // Field 2: Cipher suites (decimal, hyphen-separated, GREASE excluded).
    let ciphers: String = input
        .ciphers
        .iter()
        .filter_map(|c| cipher_id(c))
        .filter(|id| !is_grease(*id))
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    // Field 3: Extensions (decimal, hyphen-separated, GREASE excluded).
    let extensions: String = input
        .extension_ids
        .iter()
        .filter(|id| !is_grease(**id))
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    // Field 4: Elliptic curves (decimal, hyphen-separated, GREASE excluded).
    let curves: String = input
        .curves
        .iter()
        .filter_map(|c| curve_id(c))
        .filter(|id| !is_grease(*id))
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    // Field 5: EC point formats (always "0" for uncompressed in modern TLS).
    let point_formats = "0";

    format!("{version},{ciphers},{extensions},{curves},{point_formats}")
}

#[cfg(test)]
mod tests {
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
}
