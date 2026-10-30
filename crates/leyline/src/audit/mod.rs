#![forbid(unsafe_code)]
mod compare;
mod ja3;
mod ja4;
mod ja4h;
mod ja4t;

pub(crate) use ja3::{JA3_POINT_FORMATS, JA3_RECORD_VERSION};
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use ja3::{Ja3Input, compute_ja3};
#[cfg(not(feature = "bench-internals"))]
pub(crate) use ja3::{Ja3Input, compute_ja3};
pub(crate) use ja4::JA4_TCP;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use ja4::{Ja4Input, compute_ja4};
#[cfg(not(feature = "bench-internals"))]
pub(crate) use ja4::{Ja4Input, compute_ja4};
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use ja4h::{Ja4hInput, compute_ja4h};
#[cfg(not(feature = "bench-internals"))]
pub(crate) use ja4h::{Ja4hInput, compute_ja4h};
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use ja4t::compute_ja4t;
#[cfg(not(feature = "bench-internals"))]
pub(crate) use ja4t::compute_ja4t;

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

/// Holds the JA3, JA4, JA4H, JA4T, Akamai HTTP/2, and request header fingerprint of one request.
#[derive(Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Fingerprint {
    pub(crate) ja4: String,
    pub(crate) ja3: String,
    pub(crate) h2_fingerprint: String,
    pub(crate) ja4t: String,
    pub(crate) ja4h: String,
    pub(crate) permutes_extensions: bool,
    pub(crate) request_headers: Vec<(String, String)>,
}

impl std::fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers = crate::trace::masked(
            self.request_headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_bytes())),
        );
        f.debug_struct("Fingerprint")
            .field("ja4", &self.ja4)
            .field("ja3", &self.ja3)
            .field("h2_fingerprint", &self.h2_fingerprint)
            .field("ja4t", &self.ja4t)
            .field("ja4h", &self.ja4h)
            .field("permutes_extensions", &self.permutes_extensions)
            .field("request_headers", &headers)
            .finish()
    }
}

impl Fingerprint {
    /// Returns the JA3 hash, or an empty string when it was not reported.
    pub fn ja3(&self) -> &str {
        &self.ja3
    }

    /// Returns the JA4 fingerprint, or an empty string when it was not reported.
    pub fn ja4(&self) -> &str {
        &self.ja4
    }

    /// Returns the JA4H fingerprint, or an empty string when it was not reported.
    pub fn ja4h(&self) -> &str {
        &self.ja4h
    }

    /// Returns the JA4T fingerprint, or an empty string when it was not reported.
    pub fn ja4t(&self) -> &str {
        &self.ja4t
    }

    /// Returns the Akamai HTTP/2 fingerprint, or an empty string when it was not reported.
    pub fn akamai(&self) -> &str {
        &self.h2_fingerprint
    }

    /// Returns the request headers as name and value pairs in send order.
    pub fn headers(&self) -> &[(String, String)] {
        &self.request_headers
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AuditTlsCache {
    pub(crate) ja4: String,
    pub(crate) ja3: String,
    pub(crate) h2_fingerprint: String,
    pub(crate) ja4t: String,
    pub(crate) permutes_extensions: bool,
}
