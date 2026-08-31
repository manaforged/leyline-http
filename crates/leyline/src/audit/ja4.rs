//! JA4 TLS fingerprint computation (FoxIO specification).

use crate::audit::cipher_map::sigalg_id;
use crate::audit::{hash12, non_grease_cipher_ids, non_grease_ext_ids};

/// Input data for JA4 computation.
pub struct Ja4Input<'a> {
    /// Cipher suite names from the profile.
    pub ciphers: &'a [String],
    /// Signature algorithm names.
    pub sigalgs: &'a [String],
    /// Named curves / supported groups.
    pub curves: &'a [String],
    /// Extension type IDs that will be in the ClientHello.
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
fn compute_section_a(input: &Ja4Input<'_>) -> String {
    let proto = "t";
    let version = match input.tls_version {
        "1.3" => "13",
        "1.2" => "12",
        "1.1" => "11",
        "1.0" => "10",
        _ => "00",
    };

    let sni = if input.has_sni { "d" } else { "i" };

    let cipher_count = non_grease_cipher_ids(input.ciphers).len().min(99);

    let ext_count = non_grease_ext_ids(input.extension_ids).len().min(99);

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

    let sigalg_str: String = input
        .sigalgs
        .iter()
        .filter_map(|s| sigalg_id(s))
        .map(|id| format!("{id:04x}"))
        .collect::<Vec<_>>()
        .join(",");

    let combined = format!("{ext_str}_{sigalg_str}");
    hash12(&combined)
}

/// Build the list of extension IDs that **Chrome** sends in its ClientHello, derived from the TLS profile configuration.
pub fn chrome_extension_ids(tls: &crate::profile::TlsProfile) -> Vec<u16> {
    let mut exts = Vec::new();

    exts.push(0x0000);
    exts.push(0x0017);
    exts.push(0xff01);
    if !tls.curves.is_empty() {
        exts.push(0x000a);
    }

    exts.push(0x000b);
    exts.push(0x0023);
    exts.push(0x0010);
    if tls.ocsp_stapling {
        exts.push(0x0005);
    }

    if tls.delegated_credentials.is_some() {
        exts.push(0x0022);
    }

    exts.push(0x0033);
    exts.push(0x002b);
    if !tls.sigalgs.is_empty() {
        exts.push(0x000d);
    }

    exts.push(0x002d);
    if tls.record_size_limit.is_some() {
        exts.push(0x001c);
    }

    if !tls.cert_compression.is_empty() {
        exts.push(0x001b);
    }

    if tls.signed_cert_timestamps {
        exts.push(0x0012);
    }

    if tls.alps.is_some() {
        if tls.alps_new_codepoint {
            exts.push(0x4469);
        } else {
            exts.push(0x4411);
        }
    }

    if tls.ech_grease {
        exts.push(0xfe0d);
    }

    exts
}

#[cfg(test)]
mod tests;
