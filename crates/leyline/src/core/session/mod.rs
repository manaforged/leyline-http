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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolPolicy {
    Auto,
    Http1,
    Http2,
    #[cfg(feature = "http3")]
    Http3,
    #[cfg(feature = "http3")]
    Race,
}

#[derive(Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

#[derive(Clone)]
pub(crate) struct SessionInner {
    browser: Option<Browser>,
    identity: Option<Identity>,
    platform: Platform,
    brand: ChromiumBrand,
    user_agent: String,
    sec_ch_ua: String,
    accept_language: String,
    brand_extra_headers: Vec<(String, String)>,
    brand_navigate_accept: Option<String>,
    identity_extra_headers: Vec<(String, String)>,
    identity_navigate_accept: Option<String>,
    identity_request_header_order: Option<Vec<String>>,
    proxy_config: ProxyConfig,
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
    audit_tls: Arc<crate::audit::AuditTlsCache>,
    audit_enabled: bool,
    protocol_policy: ProtocolPolicy,
    default_retry: RetryPolicy,
    #[cfg(feature = "http3")]
    h3_config: Option<crate::quic::H3Config>,
    tls_trust: TlsTrustConfig,
    #[cfg(feature = "http3")]
    profile: &'static crate::profile::BrowserProfile,
    #[cfg(feature = "tower")]
    layer: Option<Arc<dyn Stack>>,
    trace: Option<Arc<dyn crate::trace::Trace>>,
    url_cache: std::sync::Arc<std::sync::Mutex<Option<(String, url::Url)>>>,
}
