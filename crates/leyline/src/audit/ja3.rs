//! JA3 TLS fingerprint computation (Salesforce).
//!
//! Format: MD5 of `{version},{ciphers},{extensions},{curves},{point_formats}`
//! where values within fields are hyphen-separated decimal integers.

use crate::audit::{non_grease_cipher_ids, non_grease_curve_ids, non_grease_ext_ids};

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
    use md5::Digest as _;
    format!("{:x}", md5::Md5::digest(raw.as_bytes()))
}

/// Compute the raw JA3 string (before hashing).
pub fn compute_ja3_raw(input: &Ja3Input<'_>) -> String {
    // Field 1: TLS version (decimal).
    let version = input.tls_record_version;

    // Field 2: Cipher suites (decimal, hyphen-separated, GREASE excluded).
    let ciphers: String = non_grease_cipher_ids(input.ciphers)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    // Field 3: Extensions (decimal, hyphen-separated, GREASE excluded).
    let extensions: String = non_grease_ext_ids(input.extension_ids)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    // Field 4: Elliptic curves (decimal, hyphen-separated, GREASE excluded).
    let curves: String = non_grease_curve_ids(input.curves)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    // Field 5: EC point formats. Hardcoded, not a profile input: every
    // bundled profile's TLS stack advertises uncompressed(0) only, like
    // every modern browser. If a future profile diverges, this must
    // become a `Ja3Input` field.
    let point_formats = "0";

    format!("{version},{ciphers},{extensions},{curves},{point_formats}")
}

#[cfg(test)]
mod tests;
