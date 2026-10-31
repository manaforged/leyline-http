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
pub(crate) use identity::Identity;

use std::sync::Arc;

use url::Url;

use crate::cookie::Jar;
#[cfg(feature = "websocket")]
use crate::core::WebSocketOptions;
use crate::core::retry::Retry;
use crate::core::{BodyRules, Proxy, Redirect, Timeout};
use crate::h2::H2Config;
use crate::header_str::HeaderStr;
use crate::pool::Pool;
use crate::profile::{Browser, ChromiumBrand, HeaderStyle, Platform};
use crate::tls::FingerprintConnector;
use crate::tls::Tls;

/// The HTTP version policy a session uses to send requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Protocol {
    /// Negotiates HTTP/2 or HTTP/1.1 through ALPN.
    Auto,
    /// Sends every request over HTTP/1.1.
    Http1,
    /// Sends every request over HTTP/2.
    Http2,
    /// Sends every request over HTTP/3.
    #[cfg(feature = "http3")]
    Http3,
    /// Tries HTTP/3 and falls back to `Auto` when HTTP/3 setup fails.
    #[cfg(feature = "http3")]
    Race,
}

impl Protocol {
    #[cfg(feature = "http3")]
    pub(crate) fn requires_h3(self) -> bool {
        matches!(self, Self::Http3 | Self::Race)
    }
}

#[doc = include_str!("../../../docs/session.md")]
#[derive(Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

#[derive(Clone)]
pub(crate) struct SessionInner {
    browser: Option<Browser>,
    impersonates: bool,
    header_style: HeaderStyle,
    identity: Option<Identity>,
    platform: Platform,
    brand: ChromiumBrand,
    user_agent: HeaderStr,
    sec_ch_ua: HeaderStr,
    accept_language: HeaderStr,
    header_order: Option<Vec<String>>,
    default_headers: Vec<(String, String)>,
    bearer: Option<builder::BearerToken>,
    proxy_config: Proxy,
    proxy_shown: Option<String>,
    timeouts: Timeout,
    redirect_policy: Redirect,
    compression: BodyRules,
    #[cfg(feature = "websocket")]
    websocket_config: WebSocketOptions,
    https_only: bool,
    cookie_jar: Jar,
    connector: FingerprintConnector,
    h2_config: H2Config,
    pool: Arc<Pool>,
    shutdown: tokio_util::sync::CancellationToken,
    writers: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    audit_tls: Option<Arc<crate::audit::AuditTlsCache>>,
    protocol_policy: Protocol,
    default_retry: Retry,
    #[cfg(feature = "http3")]
    h3_config: Option<crate::quic::H3Config>,
    tls_trust: Tls,
    profile: Arc<crate::profile::BrowserProfile>,
    base_url: Option<Url>,
    languages: Option<Vec<String>>,
    host_limits: crate::core::config::HostLimits,
    #[cfg(feature = "html")]
    crawl_limits: Arc<std::sync::OnceLock<crate::core::config::HostLimits>>,
    #[cfg(feature = "html")]
    crawl_pace: Option<crate::core::config::HostLimits>,
    proxy_pool: Option<crate::core::proxy_pool::ProxyPool>,
    block_rules: crate::core::block::Rules,
    inputs: Arc<SessionBuilder>,
    trace: Option<Arc<dyn crate::trace::Trace>>,
}
