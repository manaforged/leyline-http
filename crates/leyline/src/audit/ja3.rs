//! JA3 TLS fingerprint computation (Salesforce).

use crate::audit::{non_grease_cipher_ids, non_grease_curve_ids, non_grease_ext_ids};

/// Input data for JA3 computation.
pub struct Ja3Input<'a> {
    /// Cipher suite names.
    pub ciphers: &'a [String],
    /// Named curves / supported groups.
    pub curves: &'a [String],
    /// Extension type IDs (in ClientHello order).
    pub extension_ids: &'a [u16],
    /// TLS version (protocol version field).
    pub tls_record_version: u16,
}

/// Compute JA3 fingerprint.
pub fn compute_ja3(input: &Ja3Input<'_>) -> String {
    let raw = compute_ja3_raw(input);
    use md5::Digest as _;
    format!("{:x}", md5::Md5::digest(raw.as_bytes()))
}

/// Compute the raw JA3 string (before hashing).
pub fn compute_ja3_raw(input: &Ja3Input<'_>) -> String {
    let version = input.tls_record_version;

    let ciphers: String = non_grease_cipher_ids(input.ciphers)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    let extensions: String = non_grease_ext_ids(input.extension_ids)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    let curves: String = non_grease_curve_ids(input.curves)
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join("-");

    let point_formats = "0";

    format!("{version},{ciphers},{extensions},{curves},{point_formats}")
}

#[cfg(test)]
mod tests;
