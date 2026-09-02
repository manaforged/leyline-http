//! Session - the primary Leyline client.

#![forbid(unsafe_code)]
mod builder;
pub(crate) mod decompress;
pub(crate) mod execute;
mod header_merge;
mod helpers;
mod identity;
mod proxy;
mod transport_policy;
#[cfg(feature = "websocket")]
mod websocket;

#[cfg(test)]
mod tests;

pub use builder::SessionBuilder;
pub use identity::Identity;
#[cfg(feature = "websocket")]
pub use websocket::WebSocketBuilder;

use std::sync::Arc;

use crate::cookie::Jar;
#[cfg(feature = "websocket")]
use crate::core::WebSocketConfig;
#[cfg(feature = "tower")]
use crate::core::layer::Stack;
use crate::core::retry::RetryPolicy;
use crate::core::{CompressionConfig, ProxyConfig, RedirectPolicy, TimeoutConfig};
use crate::h2::H2Config;
use crate::pool::Pool;
use crate::profile::{Browser, ChromiumBrand, Platform};
use crate::tls::FingerprintConnector;
use crate::tls::TlsTrustConfig;

/// Protocol selection policy for requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolPolicy {
    /// Use H1 for `http://`, H2 for `https://`, and fall back to H1 when ALPN does not negotiate H2.
    Auto,
    /// Force HTTP/1.1.
    Http1,
    /// Force HTTP/2 over TLS.
    Http2,
    /// Force HTTP/3 over QUIC.
    #[cfg(feature = "http3")]
    Http3,
    /// Race QUIC (H3) against TCP+TLS (H2) for origins that advertised `h3` in `Alt-Svc` or already completed a QUIC handshake; every other origin behaves like `Auto`.
    #[cfg(feature = "http3")]
    Race,
}

/// A Leyline session - browser-fingerprinted HTTP client with cookies.
#[derive(Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

/// Inner session state, held behind an `Arc` by [`Session`].
#[derive(Clone)]
pub(crate) struct SessionInner {
    /// The impersonated browser, or `None` for a bare (non-impersonating) session.
    browser: Option<Browser>,
    /// Locked presentation.
    identity: Option<Identity>,
    platform: Platform,
    brand: ChromiumBrand,
    user_agent: String,
    sec_ch_ua: String,
    accept_language: String,
    /// Extra per-brand headers (e.g. `dnt: 1` for Edge, `sec-gpc: 1` for Brave).
    brand_extra_headers: Vec<(String, String)>,
    /// Brand-specific Navigate `accept` override (Brave drops signed-exchange).
    brand_navigate_accept: Option<String>,
    /// Identity-level extra headers from the active `[identity.*]` block (e.g. Brave's `sec-gpc: 1` once Brave is a first-class profile).
    identity_extra_headers: Vec<(String, String)>,
    /// Identity-level Navigate `accept` override.
    identity_navigate_accept: Option<String>,
    /// Identity-level explicit request-header order.
    identity_request_header_order: Option<Vec<String>>,
    proxy_config: ProxyConfig,
    /// Single source of truth for all timeouts; the total request timeout is `timeouts.total`.
    timeouts: TimeoutConfig,
    redirect_policy: RedirectPolicy,
    compression: CompressionConfig,
    #[cfg(feature = "websocket")]
    websocket_config: WebSocketConfig,
    https_only: bool,
    cookie_jar: Jar,
    connector: FingerprintConnector,
    h2_config: H2Config,
    pool: Arc<Pool>,
    /// Cached connection-level audit data computed from the profile, shared with every response by `Arc` clone (no per-request recompute).
    audit_tls: Arc<crate::audit::AuditTlsCache>,
    /// When set, responses retain their request headers and expose `Response::audit()`.
    audit_enabled: bool,
    /// Request protocol selection policy.
    protocol_policy: ProtocolPolicy,
    /// Session-wide default retry policy, inherited by every request that does not set its own via [`crate::RequestBuilder::retry`].
    default_retry: RetryPolicy,
    /// H3 config (transport params + QPACK + SETTINGS).
    #[cfg(feature = "http3")]
    h3_config: Option<crate::quic::H3Config>,
    /// Trust configuration for the h3 path — the tcp connector bakes its own copy at build time; h3 builds its context per connection.
    tls_trust: TlsTrustConfig,
    /// Reference to the static browser profile — passed through to the H3 path so QUIC ClientHello is built from the same factory as H2.
    #[cfg(feature = "http3")]
    profile: &'static crate::profile::BrowserProfile,
    /// Composed middleware stack, or `None` when the session was built without `SessionBuilder::layer`.
    #[cfg(feature = "tower")]
    layer: Option<Arc<dyn Stack>>,
    /// Lifecycle listener, or `None` when the session was built without `SessionBuilder::trace`.
    trace: Option<Arc<dyn crate::trace::Trace>>,
    /// Last-parsed request URL cache: sequential calls with the same URL string skip the parse.
    url_cache: std::sync::Arc<std::sync::Mutex<Option<(String, url::Url)>>>,
}
