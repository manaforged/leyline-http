//! # Leyline
//!
//! Browser-profiled TLS fingerprinting for Rust. Makes HTTP requests from
//! explicit browser profiles and exposes verification data for the TLS,
//! HTTP/2, TCP, and header layers. Supports HTTP/3 over QUIC and
//! fingerprinted WebSocket.
//!
//! ## One-liner
//!
//! ```rust,ignore
//! let resp = leyline::get("https://example.com").await?;
//! println!("{}", resp.text());
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
//! ## Fingerprint audit
//!
//! ```rust,ignore
//! let audit = resp.audit().unwrap();
//! println!("{}", audit.ja4);
//! ```

use std::sync::LazyLock;

// Core types
pub use leyline_core::{
    Error, HeaderList, HttpVersion, ProtocolPolicy, RequestBuilder, Response, Result, Session,
    SessionBuilder,
};

// Profile types
pub use leyline_profile::{Browser, BrowserProfile, Platform, Preset, ALL_BROWSERS, PROFILE_COUNT};

// TCP fingerprinting
pub use leyline_tcp::TcpProfile;

// Cookies
pub use leyline_cookies::CookieJar;

// Audit
pub use leyline_audit::AuditData;

/// Fingerprint computation primitives (`compute_ja3`, `compute_ja4`,
/// `compute_ja4h`, `compute_ja4t`, and input types). Re-exported so
/// downstream code can compute fingerprints offline from a
/// [`BrowserProfile`] without depending on `leyline-audit` directly.
pub mod audit {
    pub use leyline_audit::{
        chrome_extension_ids, compute_ja3, compute_ja4, compute_ja4h, compute_ja4t, AuditData,
        Ja3Input, Ja4Input, Ja4hInput,
    };
}

// WebSocket
pub use leyline_core::WsConnection;

// TLS context factory (re-exported so users don't need `leyline-tls` or
// `boring` as separate deps). Start with `tls_context` / `quic_context`
// for the 95% case; drop down to `build_ssl_context` only when you need
// the full `TlsMinVersion` knob against a profile you already hold.
pub use boring::ssl::SslContextBuilder;
pub use leyline_tls::{build_ssl_context, TlsError, TlsMinVersion};

static PROFILES: LazyLock<leyline_profile::ProfileRegistry> =
    LazyLock::new(leyline_profile::ProfileRegistry::builtin);

/// Look up the static built-in profile for a browser variant.
///
/// ```rust,ignore
/// let chrome = leyline::profile(Browser::Chrome147);
/// println!("{}", chrome.tls.ciphers.len());
/// ```
pub fn profile(browser: Browser) -> &'static BrowserProfile {
    PROFILES.get_browser(browser).expect(
        "built-in profile missing — registry integrity check in tests would have caught this",
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
/// per RFC 9001 §4.2.
///
/// ```rust,ignore
/// use leyline::{quic_context, Browser};
/// let ctx = quic_context(Browser::Chrome147)?;
/// let mut cfg = quiche::Config::with_boring_ssl_ctx_builder(
///     quiche::PROTOCOL_VERSION, ctx)?;
/// ```
pub fn quic_context(browser: Browser) -> Result<SslContextBuilder> {
    build_ssl_context(profile(browser), TlsMinVersion::Tls13).map_err(Error::from)
}

// ─── Level 1: Zero-config functions ─────────────────────────────────────

static DEFAULT_SESSION: LazyLock<Session> =
    LazyLock::new(|| Session::chrome().expect("failed to create default leyline session"));

/// GET a URL. Uses Chrome 147 defaults. No setup needed.
///
/// **Note:** All `leyline::get/post_json/post_form` calls share a single
/// session and cookie jar. For isolated requests, create a `Session`.
///
/// ```rust,ignore
/// let resp = leyline::get("https://example.com").await?;
/// println!("{}", resp.text());
/// ```
pub async fn get(url: &str) -> Result<Response> {
    DEFAULT_SESSION.navigate(url).await
}

/// POST JSON to a URL. Uses Chrome 147 defaults.
///
/// ```rust,ignore
/// let resp = leyline::post_json("https://api.example.com", &data).await?;
/// ```
pub async fn post_json(url: &str, body: &impl serde::Serialize) -> Result<Response> {
    DEFAULT_SESSION.post_json(url, body).await
}

/// POST form data to a URL. Uses Chrome 147 defaults.
///
/// ```rust,ignore
/// let resp = leyline::post_form("https://example.com/login", &[("user", "a"), ("pass", "b")]).await?;
/// ```
pub async fn post_form(url: &str, params: &[(&str, &str)]) -> Result<Response> {
    DEFAULT_SESSION.post_form(url, params).await
}

/// Raw GET with no preset headers applied. Uses Chrome 147 defaults.
///
/// Prefer [`get`] for the common browser-like GET — `get` installs the
/// Navigate preset (Sec-Fetch-Mode: navigate etc.) the way a browser
/// document-fetch would, which is what you almost always want.
pub async fn fetch(url: &str) -> Result<Response> {
    DEFAULT_SESSION.get(url).send().await
}
