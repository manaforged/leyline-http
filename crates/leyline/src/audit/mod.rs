//! JA3/JA4/JA4H/JA4T fingerprint computation and export.

#![forbid(unsafe_code)]
mod cipher_map;
mod ja3;
mod ja4;
mod ja4h;
mod ja4t;

pub use ja3::{Ja3Input, compute_ja3};
pub use ja4::{Ja4Input, compute_ja4, extension_ids};
pub use ja4h::{Ja4hInput, compute_ja4h};
pub use ja4t::compute_ja4t;

use sha2::{Digest, Sha256};

pub(crate) use cipher_map::sigalg_id;
use cipher_map::{cipher_id, curve_id, is_grease};

/// Map cipher-suite names to their IANA IDs, dropping unrecognized names and GREASE values.
fn non_grease_cipher_ids(ciphers: &[String]) -> Vec<u16> {
    ciphers
        .iter()
        .filter_map(|c| cipher_id(c))
        .filter(|id| !is_grease(*id))
        .collect()
}

/// Filter a list of extension IDs to the non-GREASE ones, preserving order.
fn non_grease_ext_ids(extension_ids: &[u16]) -> Vec<u16> {
    extension_ids
        .iter()
        .copied()
        .filter(|id| !is_grease(*id))
        .collect()
}

/// Map named-curve / supported-group names to their IANA IDs, dropping unrecognized names and GREASE values.
fn non_grease_curve_ids(curves: &[String]) -> Vec<u16> {
    curves
        .iter()
        .filter_map(|c| curve_id(c))
        .filter(|id| !is_grease(*id))
        .collect()
}

/// SHA-256, take first 12 hex chars.
fn hash12(s: &str) -> String {
    if s.is_empty() {
        "000000000000".to_owned()
    } else {
        let digest = Sha256::digest(s.as_bytes());
        hex::encode(digest)[..12].to_owned()
    }
}

/// Per-response fingerprint data.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AuditData {
    /// JA4 TLS fingerprint.
    pub ja4: String,
    /// JA3 TLS fingerprint (MD5 hash).
    pub ja3: String,
    /// Akamai-style H2 fingerprint.
    pub h2_fingerprint: String,
    /// JA4T computed from the TCP profile applied via `setsockopt`, not captured from the SYN.
    pub ja4t: String,
    /// JA4H HTTP request fingerprint.
    pub ja4h: String,
}

/// Connection-level fingerprints, precomputed once per [`Session`] from the profile.
#[derive(Debug, Clone)]
pub(crate) struct AuditTlsCache {
    pub(crate) ja4: String,
    pub(crate) ja3: String,
    pub(crate) h2_fingerprint: String,
    pub(crate) ja4t: String,
}
