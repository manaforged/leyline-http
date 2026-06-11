//! Session - the primary Leyline client.

mod builder;
mod decompress;
mod execute;
mod header_merge;
mod helpers;
mod proxy;
mod transport_policy;
#[cfg(feature = "websocket")]
mod websocket;

#[cfg(test)]
mod tests;

pub use builder::SessionBuilder;
#[cfg(feature = "websocket")]
pub use websocket::WebSocketBuilder;

use std::sync::Arc;

use crate::cookie::Jar as CookieJar;
#[cfg(feature = "websocket")]
use crate::core::WebSocketConfig;
use crate::core::{CompressionConfig, ProxyConfig, RedirectPolicy, TimeoutConfig};
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
    #[cfg(feature = "http3")]
    Http3,
    /// Prefer H3 and fall back to H2/H1. This is currently sequential, not a
    /// true parallel Chrome-style race.
    #[cfg(feature = "http3")]
    Race,
}

/// A Leyline session - browser-fingerprinted HTTP client with cookies.
///
/// `Session` is an `Arc` over its inner state, so cloning is a refcount
/// bump (O(1)) — clones share the same connection pool, cookie jar, TLS
/// connector, and BoringSSL session cache. This is what lets every
/// per-request [`RequestBuilder`](crate::RequestBuilder) own its session
/// cheaply instead of borrowing it.
#[derive(Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

impl std::ops::Deref for Session {
    type Target = SessionInner;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Inner session state, held behind an `Arc` by [`Session`]. Field access
/// goes through `Session`'s `Deref`; the only writers are the builder (at
/// construction) and the `with_*` derive methods (via `Arc::make_mut`).
///
/// `pub` only because it is the public `Deref::Target` of [`Session`]; all
/// fields are private and there are no public methods, so it carries no
/// usable surface of its own. Treat it as an implementation detail.
#[doc(hidden)]
#[derive(Clone)]
pub struct SessionInner {
    /// The impersonated browser, or `None` for a bare (non-impersonating)
    /// session.
    browser: Option<Browser>,
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
    /// `true` when `proxy` was discovered from `HTTPS_PROXY`/`HTTP_PROXY`
    /// at build time rather than set explicitly. Env-inherited `NO_PROXY`
    /// patterns only bypass env-discovered proxies.
    proxy_from_env: bool,
    timeout: std::time::Duration,
    max_redirects: usize,
    proxy_config: ProxyConfig,
    timeouts: TimeoutConfig,
    redirect_policy: RedirectPolicy,
    compression: CompressionConfig,
    #[cfg(feature = "websocket")]
    websocket_config: WebSocketConfig,
    https_only: bool,
    cookie_jar: CookieJar,
    connector: FingerprintConnector,
    h2_config: H2Config,
    pool: Arc<Pool>,
    /// Cached connection-level audit data computed from the profile, shared
    /// with every response by `Arc` clone (no per-request recompute).
    audit_tls: Arc<crate::audit::AuditTlsCache>,
    /// When set, responses retain their request headers and expose
    /// `Response::audit()`. Off by default so the hot path skips the
    /// per-request header clone for callers that never introspect.
    audit_enabled: bool,
    /// Request protocol selection policy.
    protocol_policy: ProtocolPolicy,
    /// H3 config (transport params + QPACK + SETTINGS). `None` when the
    /// profile family has no HTTP/3 fingerprint (e.g. okhttp); requesting
    /// HTTP/3 on such a profile errors rather than borrowing another's.
    #[cfg(feature = "http3")]
    h3_config: Option<crate::quic::H3Config>,
    /// Reference to the static browser profile — passed through to the H3
    /// path so QUIC ClientHello is built from the same factory as H2.
    #[cfg(feature = "http3")]
    profile: &'static crate::profile::BrowserProfile,
}
