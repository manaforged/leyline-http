//! # Leyline
//!
//! An easy, full-spectrum HTTP client for Rust — from a plain one-line GET to
//! byte-exact browser TLS/HTTP/2/HTTP/3 parity, with verification data for the
//! TLS, HTTP/2, TCP, and header layers.
//!
//! **The default is bare.** A `Session` does *not* impersonate a browser
//! unless you ask: `leyline::get(...)` and `Session::new()` give a plain,
//! honest `leyline/<version>` client on the host OS — ideal for internal and
//! third-party API calls. To look like a real browser,
//! opt in explicitly with [`Session::chrome`] / [`Session::builder`]`.browser(...)`.
//!
//! ## One-liner (bare)
//!
//! ```rust,ignore
//! let resp = leyline::get("https://api.example.com/v1").await?;
//! println!("{}", resp.text());
//! ```
//!
//! ## Browser parity (opt in)
//!
//! ```rust,ignore
//! let session = Session::chrome();              // infallible
//! let resp = session.navigate("https://example.com").await?;
//! ```
//!
//! ## Session with full control
//!
//! ```rust,ignore
//! let session = Session::builder()
//!     .browser(Browser::Chrome147)
//!     .platform(Platform::Linux)
//!     .proxy("socks5://user:pass@host:port")
//!     .timeout(Duration::from_secs(15))
//!     .build()?;
//!
//! let resp = session.post("https://api.example.com/items")
//!     .json(&payload)
//!     .bearer_auth("token")
//!     .send()
//!     .await?
//!     .error_for_status()?;
//!
//! let data: Vec<Item> = resp.json()?;
//! ```
//!
//! ## HTTP/3
//!
//! ```rust,ignore
//! let session = Session::builder().http3().build()?;
//! ```
//!
//! ## WebSocket
//!
//! ```rust,ignore
//! let mut ws = session.websocket("wss://example.com/ws").await?;
//! ws.send("hello").await?;
//! ```
//!
//! ## Custom headers
//!
//! ```rust,ignore
//! let resp = session.get("https://example.com")
//!     .header("x-request-id", "abc")
//!     .bearer_auth("token")
//!     .send().await?;
//! ```
//!
//! ## Cookies
//!
//! ```rust,ignore
//! use leyline::CookieJar;
//! let session = Session::builder()
//!     .cookie_jar(CookieJar::new())
//!     .build()?;
//! // Set-Cookie headers update the jar; later requests send them back.
//! ```
//!
//! ## Fingerprint audit
//!
//! Auditing is opt-in (`SessionBuilder::audit(true)`); without it `audit()`
//! returns `None`.
//!
//! ```rust,ignore
//! let session = Session::builder().chrome().audit(true).build()?;
//! let resp = session.navigate("https://example.com").await?;
//! if let Some(audit) = resp.audit() {
//!     println!("{}", audit.ja4);
//! }
//! ```

use std::sync::{LazyLock, OnceLock};

// Compile-checks the crate README's `rust` code blocks as doctests, so an
// example that stops matching the real API (e.g. `?` on an infallible
// constructor) fails `cargo test`. `cfg(doctest)` means this item exists
// only during doc-testing — it is not part of normal builds or rendered docs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;

// Internal modules.
pub mod audit;
pub mod cookie;
pub mod core;
pub mod h2;
pub mod observe;
pub mod pool;
pub mod profile;
#[cfg(feature = "http3")]
pub mod quic;
pub mod tcp;
pub mod tls;
pub mod tls_selftest;
mod util;

// Public API re-exports.

// Core types
#[cfg(feature = "tower")]
pub use crate::core::LeylineService;
#[cfg(feature = "websocket")]
pub use crate::core::WebSocketBuilder;
pub use crate::core::{
    Body, BodyStream, CompressionConfig, DigestAuth, DnsConfig, Error, HeaderList, HttpVersion,
    IntoParamPair, NoProxy, PoolConfig, ProtocolPolicy, ProxyConfig, ProxyRule, ProxyUrl,
    RedirectAction, RedirectAttempt, RedirectPolicy, Request, RequestBuilder, Response, Result,
    RetryPolicy, RetryTrigger, Session, SessionBuilder, SocketConfig, TimeoutConfig,
    WebSocketConfig,
};

/// Short alias for the primary Leyline session type.
pub type Client = Session;

/// `multipart/form-data` bodies.
#[cfg(feature = "multipart")]
pub mod multipart {
    pub use crate::core::multipart::{Form, Part};
}

// Profile types
pub use crate::profile::{
    BrandOverlay, BrandOverlayError, Browser, BrowserProfile, ChromiumBrand, Platform, Preset,
    ALL_BROWSERS, PROFILE_COUNT,
};

// TCP fingerprinting
pub use crate::tcp::TcpProfile;

// Cookie jar - re-export at crate root for the high-traffic case.
// Prefer `leyline::cookie::Jar` in module signatures; `leyline::CookieJar`
// is kept as a convenience alias.
pub use crate::cookie::Jar as CookieJar;

// Audit data type (top-level re-export)
pub use crate::audit::AuditData;

// WebSocket
#[cfg(feature = "websocket")]
pub use crate::core::WsConnection;

// TLS context factory (re-exported so users don't need `leyline-tls` or
// `boring` as separate deps). Start with `tls_context` / `quic_context`
// for the 95% case; drop down to `build_ssl_context` only when you need
// the full `TlsMinVersion` knob against a profile you already hold.
pub use crate::tls::{
    build_ssl_context, ClientIdentity, HappyEyeballsConfig, ResolveFuture, Resolver,
    SystemResolver, TlsError, TlsMinVersion, TlsTrustConfig,
};
pub use btls::ssl::SslContextBuilder;

static PROFILES: LazyLock<crate::profile::ProfileRegistry> =
    LazyLock::new(crate::profile::ProfileRegistry::builtin);

/// Look up the static built-in profile for a browser variant.
///
/// # Panics
/// The built-in registry contains a profile for every [`Browser`] variant
/// and the integrity test `every_browser_variant_has_a_profile` fails the
/// build if that invariant is ever broken. A panic here would indicate a
/// bug in Leyline itself, not in caller input.
///
/// ```rust,ignore
/// let chrome = leyline::profile(Browser::Chrome147);
/// println!("{}", chrome.tls.ciphers.len());
/// ```
pub fn profile(browser: Browser) -> &'static BrowserProfile {
    PROFILES.get_browser(browser).expect(
        "built-in profile missing - registry integrity check in tests would have caught this",
    )
}

/// Build a BoringSSL `SslContextBuilder` that produces a ClientHello
/// matching the given browser over TCP+TLS (h2 / http/1.1 / WebSocket).
/// TLS 1.2 is allowed so real-browser behaviour against legacy servers
/// is preserved.
///
/// ```rust,ignore
/// use leyline::{tls_context, Browser};
/// let mut ctx = tls_context(Browser::Chrome147)?;
/// ctx.set_alpn_protos(b"\x02h2")?;
/// let ctx = ctx.build();
/// ```
pub fn tls_context(browser: Browser) -> Result<SslContextBuilder> {
    build_ssl_context(profile(browser), TlsMinVersion::Tls12).map_err(Error::from)
}

/// Build a BoringSSL `SslContextBuilder` that produces a ClientHello
/// matching the given browser over QUIC (HTTP/3). TLS 1.3 is pinned
/// per RFC 9001 section 4.2.
///
/// ```rust,ignore
/// use leyline::{quic_context, Browser};
/// let ctx = quic_context(Browser::Chrome147)?;
/// let mut cfg = leyline_quiche::Config::with_boring_ssl_ctx_builder(
///     leyline_quiche::PROTOCOL_VERSION, ctx)?;
/// ```
#[cfg(feature = "http3")]
pub fn quic_context(browser: Browser) -> Result<SslContextBuilder> {
    build_ssl_context(profile(browser), TlsMinVersion::Tls13).map_err(Error::from)
}

// Zero-config functions.

/// Lazily-built shared **bare** session behind the module-level helpers
/// below. Bare = no browser impersonation (a plain `leyline/<version>`
/// client). Built infallibly (the bare profile is statically valid), so the
/// zero-config surface only ever returns a `Result` for the network call.
fn default_session() -> &'static Session {
    static DEFAULT: OnceLock<Session> = OnceLock::new();
    DEFAULT.get_or_init(Session::new)
}

/// A ready-to-use **bare** session (no impersonation). Hold and reuse it —
/// clones are cheap and share its pool and cookie jar. For an isolated
/// cookie scope build another with [`Session::new`]. To impersonate a
/// browser use [`Session::chrome`] / [`Session::builder`]`.browser(...)`.
///
/// ```rust,ignore
/// let client = leyline::client();
/// let resp = client.get("https://api.example.com/v1").send().await?;
/// ```
pub fn client() -> Session {
    Session::new()
}

/// GET a URL with the shared bare default session — a plain, honest request
/// (no browser fingerprint).
///
/// All `leyline::get`/`post_json`/`post_form`/`post`/`fetch` calls share one
/// session and cookie jar. For an isolated jar, hold your own
/// [`client`]/[`Session::new`]. To look like a browser, build a
/// [`Session::chrome`] and call its methods.
///
/// ```rust,ignore
/// let resp = leyline::get("https://example.com").await?;
/// println!("{}", resp.text());
/// ```
pub async fn get(url: &str) -> Result<Response> {
    default_session().get(url).send().await
}

/// POST JSON with the shared bare default session. Sets `content-type:
/// application/json`.
pub async fn post_json(url: &str, body: &impl serde::Serialize) -> Result<Response> {
    default_session().post(url).json(body).send().await
}

/// POST a raw body with the shared bare default session. For JSON use
/// [`post_json`]; for form data [`post_form`].
pub async fn post(url: &str, body: impl Into<crate::core::Body>) -> Result<Response> {
    default_session().post(url).body(body).send().await
}

/// POST URL-encoded form data with the shared bare default session.
pub async fn post_form<I, P>(url: &str, params: I) -> Result<Response>
where
    I: IntoIterator<Item = P>,
    P: IntoParamPair,
{
    default_session().post(url).form(params).send().await
}

/// Alias for [`get`] — a plain GET on the shared bare default session.
pub async fn fetch(url: &str) -> Result<Response> {
    default_session().get(url).send().await
}
