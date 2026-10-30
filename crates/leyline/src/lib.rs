#![doc = include_str!("../docs/overview.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
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
    #[doc = include_str!("../../../docs/guide/crawling.md")]
    pub struct Crawling;
    #[doc = include_str!("../../../docs/guide/accounts.md")]
    pub struct Accounts;
    #[doc = include_str!("../../../docs/guide/api-clients.md")]
    pub struct ApiClients;
}

macro_rules! bench_pub {
    ($(#[$meta:meta])* fn $($rest:tt)*) => {
        #[cfg(feature = "bench-internals")]
        #[allow(missing_docs, reason = "bench-internals exposes unsupported internals for benches and fuzzing")]
        $(#[$meta])*
        pub fn $($rest)*

        #[cfg(not(feature = "bench-internals"))]
        $(#[$meta])*
        pub(crate) fn $($rest)*
    };
}

#[cfg(feature = "bench-internals")]
#[doc(hidden)]
#[allow(
    missing_docs,
    reason = "bench-internals exposes unsupported internals for benches and fuzzing"
)]
pub mod audit;
#[cfg(not(feature = "bench-internals"))]
pub(crate) mod audit;
/// Cookie storage and `Set-Cookie` parsing.
pub mod cookie;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
#[allow(
    missing_docs,
    reason = "bench-internals exposes unsupported internals for benches and fuzzing"
)]
pub mod profile;
#[cfg(not(feature = "bench-internals"))]
pub(crate) mod profile;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
#[allow(
    missing_docs,
    reason = "bench-internals exposes unsupported internals for benches and fuzzing"
)]
pub mod tls;
#[cfg(not(feature = "bench-internals"))]
pub(crate) mod tls;
/// Trace hooks that observe connection and request events.
pub mod trace;

pub(crate) mod core;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
#[allow(
    missing_docs,
    reason = "bench-internals exposes unsupported internals for benches and fuzzing"
)]
pub mod h2;
#[cfg(not(feature = "bench-internals"))]
pub(crate) mod h2;
pub(crate) mod header_str;
pub(crate) mod iana;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
#[allow(
    missing_docs,
    reason = "bench-internals exposes unsupported internals for benches and fuzzing"
)]
pub mod pool;
#[cfg(not(feature = "bench-internals"))]
pub(crate) mod pool;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
#[allow(
    missing_docs,
    reason = "bench-internals exposes unsupported internals for benches and fuzzing"
)]
pub mod fuzz {
    pub use crate::cookie::parse::parse_cookie_date;
    pub use crate::pool::h1::parse::{parse_h1_head, read_chunked_body};
    pub use crate::tls::proxy::http::validate_connect_response;

    pub fn parse_set_cookie(header: &str, request_url: &str) -> Option<crate::cookie::Cookie> {
        let parsed = url::Url::parse(request_url).ok()?;
        crate::cookie::parse::parse_set_cookie(header, &parsed)
    }

    pub fn extension_ids(tls: &crate::profile::TlsProfile) -> Vec<u16> {
        tls.extension_ids()
    }

    pub fn parse_links(header: &str, base: &str) -> Vec<crate::Link> {
        url::Url::parse(base)
            .map(|base| crate::core::parse_links(header, &base))
            .unwrap_or_default()
    }
}
/// HTML parsing for links, forms, and crawl pages.
#[cfg(feature = "html")]
pub mod html;
#[cfg(feature = "http3")]
pub(crate) mod quic;
pub(crate) mod tcp;
/// A local HTTP and HTTPS server for tests: `Server`, `Response`, `Request`, and `queue`.
#[cfg(feature = "test-util")]
pub mod testing;
mod util;

pub use http;
pub use url::Url;

pub(crate) use crate::core::Cause;
pub(crate) use crate::core::HttpVersion;
pub use crate::core::{
    Attempt, Block, Body, Encoding, Error, HostLimits, IntoParamPair, IntoUrl, Kind, Link,
    Protocol, Proxy, Purpose, Redirect, RequestBuilder, Response, Result, Retry, Robots,
    RobotsSource, Session, SessionBuilder, Stats, Tab, Timeout, relay_headers,
};
pub(crate) use crate::core::{BodyStream, FetchSite, ResponseTiming};
#[cfg(feature = "html")]
pub use crate::core::{Crawl, Page, Respect, Scope, SkipReason};
pub use crate::util::redact as redact_url;

/// Multipart form bodies built from `Form` and `Part`.
#[cfg(feature = "multipart")]
pub mod multipart {
    pub use crate::core::multipart::{Form, Part};
}

use crate::profile::ProfileRegistry;
pub use crate::profile::anchor::HeaderPosition;
pub use crate::profile::browser::Browser;
pub use crate::profile::platform::Platform;
pub use crate::profile::preset::RequestKind;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use crate::profile::types::BrowserProfile;
#[cfg(not(feature = "bench-internals"))]
pub(crate) use crate::profile::types::BrowserProfile;

pub use crate::tcp::TcpProfile;

#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use crate::h2::{ErrorCode, H2Error};

#[cfg(all(feature = "http3", feature = "bench-internals"))]
#[doc(hidden)]
pub use crate::quic::H3Config;

#[cfg(feature = "websocket")]
pub use crate::core::{Message, WebSocket};

pub use crate::audit::Fingerprint;
pub use crate::tls::Resolver;
#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
pub use crate::tls::TlsContext;
#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
pub use crate::tls::builder::TlsMinVersion;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use crate::tls::error::{ProxyReply, TlsError};
pub use crate::tls::trust::Tls;

impl Browser {
    bench_pub! {
        fn profile(self) -> &'static BrowserProfile {
            ProfileRegistry::global().get_browser(self).expect(
                "built-in profile missing - registry integrity check in tests would have caught this",
            )
        }
    }

    bench_pub! {
        #[cfg_attr(
            not(any(test, feature = "bench-internals")),
            expect(dead_code, reason = "tests and bench-internals read the branded profile")
        )]
        fn branded_profile(self) -> std::sync::Arc<BrowserProfile> {
            let base = self.shared_profile();
            match self.brand() {
                Some(brand) => brand.tls_profile(base),
                None => base,
            }
        }
    }

    pub(crate) fn shared_profile(self) -> std::sync::Arc<BrowserProfile> {
        ProfileRegistry::global().shared(self).expect(
            "built-in profile missing - registry integrity check in tests would have caught this",
        )
    }
}
