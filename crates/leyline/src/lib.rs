#![doc = include_str!("../README.md")]

#[cfg(doctest)]
#[doc = include_str!("../../../docs/api.md")]
pub struct ApiMap;

#[cfg(doctest)]
pub mod guide {
    #[doc = include_str!("../../../docs/README.md")]
    pub struct Index;
    #[doc = include_str!("../../../docs/guide/quick-start.md")]
    pub struct QuickStart;
    #[doc = include_str!("../../../docs/guide/sessions.md")]
    pub struct Sessions;
    #[doc = include_str!("../../../docs/guide/requests.md")]
    pub struct Requests;
    #[doc = include_str!("../../../docs/guide/responses.md")]
    pub struct Responses;
    #[doc = include_str!("../../../docs/guide/streaming.md")]
    pub struct Streaming;
    #[doc = include_str!("../../../docs/guide/retries-and-timeouts.md")]
    pub struct Retries;
    #[doc = include_str!("../../../docs/guide/proxies.md")]
    pub struct Proxies;
    #[doc = include_str!("../../../docs/guide/cookies.md")]
    pub struct Cookies;
    #[doc = include_str!("../../../docs/guide/websocket.md")]
    pub struct WebSocket;
    #[doc = include_str!("../../../docs/guide/http3.md")]
    pub struct Http3;
    #[doc = include_str!("../../../docs/guide/tls-trust.md")]
    pub struct TlsTrust;
    #[doc = include_str!("../../../docs/guide/network.md")]
    pub struct Network;
    #[doc = include_str!("../../../docs/guide/fingerprints.md")]
    pub struct Fingerprints;
    #[doc = include_str!("../../../docs/guide/features-and-targets.md")]
    pub struct Features;
    #[doc = include_str!("../../../docs/guide/profiles.md")]
    pub struct Profiles;
    #[doc = include_str!("../../../docs/guide/choosing-a-profile.md")]
    pub struct ChoosingAProfile;
    #[doc = include_str!("../../../docs/guide/redirects.md")]
    pub struct Redirects;
    #[doc = include_str!("../../../docs/guide/errors.md")]
    pub struct Errors;
    #[doc = include_str!("../../../docs/guide/logging.md")]
    pub struct Logging;
    #[doc = include_str!("../../../docs/guide/platforms.md")]
    pub struct Platforms;
}

pub mod audit;
pub mod cookie;
pub mod profile;
pub mod tls;
pub mod trace;

pub(crate) mod core;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod h2;
#[cfg(not(feature = "bench-internals"))]
#[allow(dead_code, unused_imports)]
pub(crate) mod h2;
pub(crate) mod header_str;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod pool;
#[cfg(not(feature = "bench-internals"))]
#[allow(dead_code, unused_imports)]
pub(crate) mod pool;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod fuzz {
    pub use crate::cookie::parse::parse_cookie_date;
    pub use crate::pool::h1::parse::{parse_h1_head, read_chunked_body};
    pub use crate::tls::proxy::http::validate_connect_response;

    pub fn parse_set_cookie(header: &str, request_url: &str) -> Option<crate::cookie::Cookie> {
        let parsed = url::Url::parse(request_url).ok()?;
        crate::cookie::parse::parse_set_cookie(header, &parsed)
    }
}
#[cfg(feature = "http3")]
pub(crate) mod quic;
pub(crate) mod tcp;
mod util;

pub use http;

#[cfg(feature = "tower")]
pub mod layer {
    pub use crate::core::layer::{Call, Log, Logged, Pending, Reply, Transport};
}

#[cfg(feature = "tower")]
pub use crate::core::LeylineService;
#[cfg(feature = "websocket")]
pub use crate::core::WebSocketBuilder;
pub use crate::core::{
    Body, BodyStream, CompressionConfig, ContentEncoding, DigestAuth, DnsConfig, Error, HeaderList,
    HttpVersion, Identity, IntoParamPair, Kind, NoProxy, PoolConfig, ProtocolPolicy, ProxyConfig,
    ProxyRule, ProxyUrl, RedirectAction, RedirectAttempt, RedirectPolicy, Request, RequestBuilder,
    Response, ResponseTiming, Result, RetryPolicy, RetryTrigger, Session, SessionBuilder,
    SocketConfig, TimeoutConfig, WebSocketConfig,
};
pub use crate::pool::PoolStats;

#[cfg(feature = "multipart")]
pub mod multipart {
    pub use crate::core::multipart::{Form, Part};
}

use crate::profile::ProfileRegistry;
pub use crate::profile::{Browser, BrowserProfile, ChromiumBrand, Platform, Preset};

pub use crate::tcp::TcpProfile;

pub use crate::h2::{ErrorCode, H2Error};

#[cfg(feature = "http3")]
pub use crate::quic::H3Config;

#[cfg(feature = "websocket")]
pub use crate::core::{CloseFrame, WsConnection, WsMessage, WsSink, WsStream};

pub use crate::tls::{TlsContext, TlsError, TlsMinVersion, TlsTrustConfig};

impl Browser {
    pub fn profile(self) -> &'static BrowserProfile {
        ProfileRegistry::global().get_browser(self).expect(
            "built-in profile missing - registry integrity check in tests would have caught this",
        )
    }
}
