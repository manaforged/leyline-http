#![forbid(unsafe_code)]
mod compare;
mod ja3;
mod ja4;
mod ja4h;
mod ja4t;

pub use compare::{FieldOutcome, FingerprintReport, HeaderOutcome, Observed};
pub use ja3::{Ja3Input, compute_ja3};
pub use ja4::{Ja4Input, compute_ja4};
pub use ja4h::{Ja4hInput, compute_ja4h};
pub use ja4t::compute_ja4t;

use sha2::{Digest, Sha256};

use crate::iana::{cipher_id, curve_id, is_grease};

fn non_grease_cipher_ids(ciphers: &[String]) -> Vec<u16> {
    ciphers
        .iter()
        .filter_map(|c| cipher_id(c))
        .filter(|id| !is_grease(*id))
        .collect()
}

fn non_grease_ext_ids(extension_ids: &[u16]) -> Vec<u16> {
    extension_ids
        .iter()
        .copied()
        .filter(|id| !is_grease(*id))
        .collect()
}

fn non_grease_curve_ids(curves: &[String]) -> Vec<u16> {
    curves
        .iter()
        .filter_map(|c| curve_id(c))
        .filter(|id| !is_grease(*id))
        .collect()
}

fn hash12(s: &str) -> String {
    if s.is_empty() {
        "000000000000".to_owned()
    } else {
        let digest = Sha256::digest(s.as_bytes());
        hex::encode(digest)[..12].to_owned()
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AuditData {
    pub ja4: String,
    pub ja3: String,
    pub h2_fingerprint: String,
    pub ja4t: String,
    pub ja4h: String,
    pub permutes_extensions: bool,
    pub request_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub(crate) struct AuditTlsCache {
    pub(crate) ja4: String,
    pub(crate) ja3: String,
    pub(crate) h2_fingerprint: String,
    pub(crate) ja4t: String,
    pub(crate) permutes_extensions: bool,
}
