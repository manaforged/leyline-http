#![doc = include_str!("../docs/overview.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(not(feature = "bench-internals"), deny(missing_docs))]
#![cfg_attr(not(feature = "bench-internals"), deny(missing_debug_implementations))]

#[cfg(doctest)]
#[doc = include_str!("../../../README.md")]
pub struct Readme;

#[cfg(doctest)]
#[doc = include_str!("../../../AGENTS.md")]
pub struct Agents;

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
    pub struct Tls;
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
    #[doc = include_str!("../../../docs/guide/cancellation.md")]
    pub struct Cancellation;
    #[doc = include_str!("../../../docs/guide/testing.md")]
    pub struct Testing;
    #[doc = include_str!("../../../docs/guide/service-integration.md")]
    pub struct ServiceIntegration;
    #[doc = include_str!("../../../docs/guide/throughput.md")]
    pub struct Throughput;
    #[doc = include_str!("../../../docs/guide/crawling.md")]
    pub struct Crawling;
    #[doc = include_str!("../../../docs/guide/accounts.md")]
    pub struct Accounts;
    #[doc = include_str!("../../../docs/guide/api-clients.md")]
    pub struct ApiClients;
}

pub(crate) mod audit;
/// Blocking use: a `Session` whose calls run on the calling thread.
pub mod blocking;
/// Cookie storage and `Set-Cookie` parsing.
pub mod cookie;
pub(crate) mod profile;
pub(crate) mod tls;
/// Trace hooks that observe connection and request events.
pub mod trace;

pub(crate) mod core;
mod deadline;
mod error;
pub(crate) mod h2;
pub(crate) mod header_str;
pub(crate) mod iana;
pub(crate) mod pool;
pub(crate) mod wire;
/// Unsupported internals for the benches, fuzz targets, and integration tests.
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub mod internals {
    use std::sync::Arc;

    use crate::{Browser, Platform, Session};

    pub use crate::core::saved::ConnectionState;
    pub use crate::net::NetError;
    pub use crate::profile::BrowserProfile;
    #[cfg(feature = "http3")]
    pub use crate::quic::H3Config;
    pub use crate::tls::error::TlsError;

    /// Returns the built-in profile of `browser`.
    pub fn browser_profile(browser: Browser) -> &'static BrowserProfile {
        browser.profile()
    }

    /// Returns the profile of `browser` with its brand applied.
    pub fn branded_profile(browser: Browser) -> Arc<BrowserProfile> {
        browser.branded_profile()
    }

    /// Returns the key that names `platform` in profile tables.
    pub fn identity_key(platform: Platform) -> &'static str {
        platform.identity_key()
    }

    /// Returns the TLS sessions, Alt-Svc entries, and HSTS hosts `session` holds.
    pub fn connection_state(session: &Session) -> ConnectionState {
        session.connection_state()
    }

    /// Loads `state` into `session`.
    pub fn restore(session: &Session, state: &ConnectionState) {
        session.restore(state);
    }

    /// TLS and HTTP fingerprint computation.
    pub mod audit {
        pub use crate::audit::{
            ClientHello, Ja3Input, Ja4Input, Ja4hInput, compute_ja3, compute_ja4, compute_ja4h,
            compute_ja4t,
        };
    }

    /// Parsers the fuzz targets drive.
    pub mod fuzz {
        pub use crate::cookie::parse::parse_cookie_date;
        pub use crate::net::proxy::http::validate_connect_response;
        pub use crate::pool::h1::parse::read_chunked_body;

        /// Parses an HTTP/1.1 response head into its status, fields, and minor version.
        pub fn parse_h1_head(head: &[u8]) -> crate::Result<(u16, http::HeaderMap, u8)> {
            crate::wire::h1::parse_h1_head(head)
        }

        /// Parses a `Set-Cookie` value received from `request_url`.
        pub fn parse_set_cookie(header: &str, request_url: &str) -> Option<crate::cookie::Cookie> {
            let parsed = url::Url::parse(request_url).ok()?;
            crate::cookie::parse::parse_set_cookie(header, &parsed)
        }

        /// Returns the TLS extension ids `tls` sends over TCP.
        pub fn extension_ids(tls: &crate::profile::TlsProfile) -> Vec<u16> {
            tls.extension_ids(crate::profile::Carrier::Tcp)
        }

        /// Parses a `Link` header value against `base`.
        pub fn parse_links(header: &str, base: &str) -> Vec<crate::Link> {
            url::Url::parse(base)
                .map(|base| crate::core::parse_links(header, &base))
                .unwrap_or_default()
        }
    }

    /// The HTTP/2 client, codec, and HPACK.
    pub mod h2 {
        pub use crate::h2::client::{H2Client, Head};
        pub use crate::h2::config::H2Limits;
        pub use crate::h2::{
            ErrorCode, H2Config, H2Error, codec, connection, error, frame, hpack, start,
            stream_state,
        };
        pub use crate::header_str::HeaderStr;
        pub use crate::wire::{ErrorBudget, ResponseMode};
    }

    /// The connection pool and the HTTP/1.1 transport.
    pub mod pool {
        pub use crate::net::ProxyUrl;
        pub use crate::pool::h1::{floor, send_request_h1_pooled};
        pub use crate::pool::{
            ConnectionStats, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS,
            DEFAULT_MAX_H1_CONNS_PER_HOST, Pool, checkout_handle,
        };
        pub use crate::wire::{TransportBody, TransportResponse};
    }

    /// Browser profiles and their registry.
    pub mod profile {
        pub use crate::profile::{
            H2Profile, ProfileRegistry, PseudoOrder, SettingId, akamai_fingerprint,
        };
    }

    /// The fingerprinting TLS connector.
    pub mod tls {
        pub use crate::tls::{AlpnPlan, FingerprintConnector};
    }
}
/// HTML parsing for links, forms, and crawl pages.
#[cfg(feature = "html")]
pub mod html;
pub(crate) mod net;
#[cfg(feature = "http3")]
pub(crate) mod quic;
/// A local HTTP and HTTPS server for tests: `Server`, `Response`, `Request`, and `queue`.
#[cfg(feature = "test-util")]
pub mod testing;
mod util;

pub use http;
pub use url::Url;

pub use crate::core::{
    Attempt, Block, Event, Events, HostLimits, IntoUrl, Link, Protocol, Proxy, Purpose, Redirect,
    RequestBuilder, Response, Retry, Robots, RobotsSource, Session, SessionBuilder, Stats, Tab,
    relay_headers,
};
pub use crate::deadline::Timeout;
pub use crate::error::{Error, Kind, Result};
pub use crate::wire::Body;

#[cfg(feature = "html")]
pub use crate::core::{Crawl, Page, Respect, Scope, SkipReason};
pub use crate::util::redact as redact_url;

/// Multipart form bodies built from `Form` and `Part`.
#[cfg(feature = "multipart")]
pub mod multipart {
    pub use crate::core::multipart::{Form, Part};
}

pub use crate::profile::anchor::HeaderPosition;
pub use crate::profile::browser::Browser;
pub use crate::profile::platform::Platform;
pub use crate::profile::preset::RequestKind;

pub use crate::profile::Encoding;
pub use crate::profile::TcpProfile;

#[cfg(feature = "websocket")]
pub use crate::core::{Message, WebSocket};

pub use crate::audit::Fingerprint;
pub use crate::net::Resolver;
#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
pub use crate::profile::TlsMinVersion;
#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
pub use crate::tls::TlsContext;
pub use crate::tls::trust::Tls;
