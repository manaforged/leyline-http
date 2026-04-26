//! Session - the primary Leyline client.

mod builder;
mod decompress;
mod execute;
mod header_merge;
mod helpers;
mod proxy;
mod transport_policy;
mod websocket;

#[cfg(test)]
mod tests;

pub use builder::SessionBuilder;

use std::sync::Arc;

use crate::cookie::Jar as CookieJar;
use crate::h2::H2Config;
use crate::pool::Pool;
use crate::profile::{Browser, ChromiumBrand, Platform};
use crate::tls::FingerprintConnector;

/// Protocol selection policy for requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolPolicy {
    /// Use H1 for `http://`, H2 for `https://`, and fall back to H1 when ALPN
    /// does not negotiate H2.
    Auto,
    /// Force HTTP/1.1. Uses plaintext H1 for `http://` and TLS H1 for `https://`.
    Http1,
    /// Force HTTP/2 over TLS.
    Http2,
    /// Force HTTP/3 over QUIC.
    Http3,
    /// Prefer H3 and fall back to H2/H1. This is currently sequential, not a
    /// true parallel Chrome-style race.
    Race,
}

/// A Leyline session - browser-fingerprinted HTTP client with cookies.
///
/// `Session` is cheaply cloneable: clones share the same connection pool,
/// cookie jar, TLS connector, and BoringSSL session cache.
#[derive(Clone)]
pub struct Session {
    browser: Browser,
    platform: Platform,
    brand: ChromiumBrand,
    user_agent: String,
    sec_ch_ua: String,
    accept_language: String,
    /// Extra per-brand headers (e.g. `dnt: 1` for Edge, `sec-gpc: 1`
    /// for Brave). Appended after the preset's identity block at
    /// request time.
    brand_extra_headers: Vec<(String, String)>,
    /// Brand-specific Navigate `accept` override (Brave drops
    /// signed-exchange). `None` means use the preset's default.
    brand_navigate_accept: Option<String>,
    /// Identity-level extra headers from the active `[identity.*]`
    /// block (e.g. Brave's `sec-gpc: 1` once Brave is a first-class
    /// profile). Same precedence rules as `brand_extra_headers`.
    identity_extra_headers: Vec<(String, String)>,
    /// Identity-level Navigate `accept` override.
    identity_navigate_accept: Option<String>,
    /// Identity-level explicit request-header order. When set,
    /// the assembled header list is reordered to match.
    identity_request_header_order: Option<Vec<String>>,
    proxy: Option<String>,
    timeout: std::time::Duration,
    max_redirects: usize,
    cookie_jar: CookieJar,
    connector: FingerprintConnector,
    h2_config: H2Config,
    pool: Arc<Pool>,
    /// Cached audit data computed from the profile (TLS + TCP parts).
    audit_tls: AuditTlsCache,
    /// Request protocol selection policy.
    protocol_policy: ProtocolPolicy,
    /// H3 config (transport params + QPACK + SETTINGS).
    h3_config: crate::quic::H3Config,
    /// Reference to the static browser profile — passed through to the H3
    /// path so QUIC ClientHello is built from the same factory as H2.
    profile: &'static crate::profile::BrowserProfile,
}

/// Pre-computed TLS/TCP audit data from the profile.
#[derive(Debug, Clone)]
struct AuditTlsCache {
    ja4: String,
    ja3: String,
    h2_fingerprint: String,
    ja4t: String,
}
