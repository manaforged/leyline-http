//! An HTTP client that mimics browsers on the wire.

use std::sync::LazyLock;

#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
pub struct ReadmeDoctests;

pub mod audit;
pub mod cookie;
pub mod profile;
pub mod tls;
pub(crate) mod tls_selftest;

#[doc(hidden)]
pub mod observe;

#[doc(hidden)]
pub mod core;
#[doc(hidden)]
pub mod h2;
#[doc(hidden)]
pub mod pool;
#[cfg(feature = "http3")]
pub(crate) mod quic;
pub(crate) mod tcp;
mod util;

#[cfg(feature = "tower")]
pub use crate::core::LeylineService;
#[cfg(feature = "websocket")]
pub use crate::core::WebSocketBuilder;
pub use crate::core::{
    Body, BodyStream, CompressionConfig, ContentEncoding, DigestAuth, DnsConfig, Error, HeaderList,
    HttpVersion, Identity, IntoParamPair, NoProxy, PoolConfig, ProtocolPolicy, ProxyConfig,
    ProxyRule, ProxyUrl, RedirectAction, RedirectAttempt, RedirectPolicy, Request, RequestBuilder,
    Response, ResponseTiming, Result, RetryPolicy, RetryTrigger, Session, SessionBuilder,
    SocketConfig, TimeoutConfig, WebSocketConfig,
};
pub use crate::pool::PoolStats;

/// `multipart/form-data` bodies.
#[cfg(feature = "multipart")]
pub mod multipart {
    pub use crate::core::multipart::{Form, Part};
}

pub use crate::profile::{Browser, BrowserProfile, ChromiumBrand, Platform, Preset};

pub use crate::tcp::TcpProfile;

pub use crate::h2::{ErrorCode, H2Error};

#[cfg(feature = "http3")]
pub use crate::quic::H3Config;

#[cfg(feature = "websocket")]
pub use crate::core::{WsConnection, WsMessage, WsSink, WsStream};

pub use crate::tls::{TlsContext, TlsError, TlsMinVersion, TlsTrustConfig};

static PROFILES: LazyLock<crate::profile::ProfileRegistry> =
    LazyLock::new(crate::profile::ProfileRegistry::builtin);

impl Browser {
    /// Built-in static profile for this variant.
    pub fn profile(self) -> &'static BrowserProfile {
        PROFILES.get_browser(self).expect(
            "built-in profile missing - registry integrity check in tests would have caught this",
        )
    }
}
