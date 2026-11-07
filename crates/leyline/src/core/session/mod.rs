#![forbid(unsafe_code)]
mod builder;
mod cache;
mod execute;
mod guard;
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

use crate::h2::H2Config;
use crate::header_str::HeaderStr;
use crate::pool::{Pool, Route};
use crate::profile::preset::HeaderPlan;
use crate::profile::{Browser, Platform};
use crate::tls::FingerprintConnector;

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
        !matches!(self.plan(), Plan::Tcp(_))
    }

    fn plan(self) -> Plan {
        match self {
            Self::Auto => Plan::Tcp(Route::Auto),
            Self::Http1 => Plan::Tcp(Route::Http1),
            Self::Http2 => Plan::Tcp(Route::Http2),
            #[cfg(feature = "http3")]
            Self::Http3 => Plan::H3Only,
            #[cfg(feature = "http3")]
            Self::Race => Plan::H3First,
        }
    }

    fn tcp_route(self) -> Option<Route> {
        match self.plan() {
            Plan::Tcp(route) => Some(route),
            #[cfg(feature = "http3")]
            Plan::H3First => Some(Route::Auto),
            #[cfg(feature = "http3")]
            Plan::H3Only => None,
        }
    }

    fn dials_h2(self) -> bool {
        self.tcp_route().is_some_and(|route| route != Route::Http1)
    }

    fn tcp_offers_h2(self) -> bool {
        self.tcp_route().unwrap_or(Route::Auto) != Route::Http1
    }
}

/// Where a protocol policy sends a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Tcp(Route),
    #[cfg(feature = "http3")]
    H3Only,
    #[cfg(feature = "http3")]
    H3First,
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
    header_plan: Arc<HeaderPlan>,
    identity: Option<Identity>,
    platform: Platform,
    user_agent: HeaderStr,
    sec_ch_ua: HeaderStr,
    accept_language: HeaderStr,
    header_order: Option<Vec<String>>,
    default_headers: Vec<(String, HeaderStr)>,
    cache: Option<Arc<cache::Cache>>,
    proxy_shown: Option<String>,
    connector: FingerprintConnector,
    h2_config: H2Config,
    pool: Arc<Pool>,
    hsts: Arc<std::sync::Mutex<crate::core::hsts::HstsStore>>,
    shutdown: Arc<crate::core::shutdown::Shutdown>,
    writers: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    audit_tls: Option<Arc<crate::audit::AuditTlsCache>>,
    #[cfg(feature = "http3")]
    h3_config: Option<crate::quic::H3Config>,
    profile: Arc<crate::profile::BrowserProfile>,
    host_limits: crate::core::config::HostGates,
    extra_gates: Option<crate::core::config::HostGates>,
    inputs: SessionBuilder,
    header_id: crate::core::request_headers::HeaderId,
}
