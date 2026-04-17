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

use std::sync::{LazyLock, OnceLock};

// Core types
pub use leyline_core::{
    Body, BodyStream, DigestAuth, Error, HeaderList, HttpVersion, ProtocolPolicy, Request,
    RequestBuilder, Response, Result, RetryPolicy, RetryTrigger, Session, SessionBuilder,
};

/// `multipart/form-data` bodies (re-exported from `leyline-core`).
pub mod multipart {
    pub use leyline_core::multipart::{Form, Part};
}

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

/// Lazily-built shared session behind the module-level helpers below.
/// Construction failures are surfaced as a `Result` — the zero-config
/// surface never panics on its own.
fn default_session() -> Result<&'static Session> {
    static DEFAULT: OnceLock<Session> = OnceLock::new();
    if let Some(session) = DEFAULT.get() {
        return Ok(session);
    }
    let built = Session::chrome_latest()?;
    Ok(DEFAULT.get_or_init(|| built))
}

/// GET a URL. Uses the latest bundled Chrome profile.
///
/// All `leyline::get`/`post_json`/`post_form`/`fetch` calls share a single
/// session and cookie jar. For isolation, create your own [`Session`].
///
/// ```rust,ignore
/// let resp = leyline::get("https://example.com").await?;
/// println!("{}", resp.text());
/// ```
pub async fn get(url: &str) -> Result<Response> {
    default_session()?.navigate(url).await
}

/// POST JSON to a URL. Uses the latest bundled Chrome profile.
pub async fn post_json(url: &str, body: &impl serde::Serialize) -> Result<Response> {
    default_session()?.post_json(url, body).await
}

/// POST form data to a URL. Uses the latest bundled Chrome profile.
pub async fn post_form(url: &str, params: &[(&str, &str)]) -> Result<Response> {
    default_session()?.post_form(url, params).await
}

/// Raw GET with no preset headers applied. Prefer [`get`] for the common
/// browser-like GET — `get` installs the Navigate preset (Sec-Fetch-Mode:
/// navigate etc.) the way a browser document-fetch would.
pub async fn fetch(url: &str) -> Result<Response> {
    default_session()?.get(url).send().await
}
