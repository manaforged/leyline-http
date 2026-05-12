//! Response observer hook — single global callback fired once per
//! fully-assembled, non-streaming response.
//!
//! Designed for development-time debugging tools (e.g. response-body
//! dumps) that want universal coverage without forcing every call site
//! to opt in. The default state is "no observer registered", which makes
//! the path a single load + branch on the hot path.
//!
//! # Usage
//!
//! ```rust,ignore
//! leyline::observe::set_response_observer(|snap| {
//!     println!(
//!         "{} {} -> {} ({} bytes)",
//!         snap.method,
//!         snap.final_url,
//!         snap.status,
//!         snap.body.len()
//!     );
//! });
//! ```
//!
//! Only one observer is honored per process. Subsequent calls to
//! [`set_response_observer`] are silently ignored (the underlying
//! storage is `OnceLock`). This matches the typical "set once at
//! application startup" usage pattern and avoids any concurrent-replace
//! semantics that would complicate the hot path.

use std::sync::{Arc, OnceLock};

/// Snapshot of a response handed to the observer. Borrows from the
/// in-flight response state — observers must not retain the slices
/// across the callback boundary.
#[derive(Debug)]
pub struct ResponseSnapshot<'a> {
    /// HTTP method as sent on the final request hop.
    pub method: &'a str,
    /// Originally requested URL (before any redirects).
    pub url: &'a str,
    /// URL after redirect resolution. Equal to `url` when no redirect
    /// occurred.
    pub final_url: &'a str,
    /// HTTP status code of the final response.
    pub status: u16,
    /// Outbound request headers as they actually went on the wire on
    /// the final hop (after preset assembly, identity overlays, brand
    /// extras, caller anchors, cookie injection, and identity-level
    /// reordering). This is the source of truth for "what did we
    /// actually send" — invaluable when debugging a 4xx/5xx whose body
    /// gives no diagnostic detail.
    pub request_headers: &'a [(String, String)],
    /// Response headers as received from the wire.
    pub response_headers: &'a [(String, String)],
    /// Response body bytes, **after** decompression. Empty when the
    /// caller opted into streaming, which the observer cannot drain
    /// without changing semantics.
    pub body: &'a [u8],
}

/// Type alias for the observer callback. Held in `Arc` so cheap to share
/// across the global slot and any callers that want to keep a handle.
pub type Observer = Arc<dyn Fn(&ResponseSnapshot<'_>) + Send + Sync + 'static>;

static OBSERVER: OnceLock<Observer> = OnceLock::new();

/// Register the global response observer. First write wins; subsequent
/// calls are no-ops. Safe to call from any thread before or during
/// request execution.
pub fn set_response_observer<F>(f: F)
where
    F: Fn(&ResponseSnapshot<'_>) + Send + Sync + 'static,
{
    let _ = OBSERVER.set(Arc::new(f));
}

/// Internal: fire the observer if one is registered. Called from the
/// session execute path right before returning a buffered response.
/// The observer is invoked synchronously on the current task — observers
/// that need to perform expensive work should offload it themselves.
pub(crate) fn notify_response(snap: &ResponseSnapshot<'_>) {
    if let Some(obs) = OBSERVER.get() {
        obs(snap);
    }
}

/// Snapshot of a request *failure* handed to the error observer. Fires
/// for any `Err` returned by `execute_with_timeout` — TLS handshake
/// failure, ALPN mismatch, H2 driver error, DNS, timeout, redirect
/// policy break, body-replay refusal, etc. This is the diagnostic
/// counterpart to [`ResponseSnapshot`]: when the request never reaches
/// the response stage, callers still need to see *which* method+url
/// failed and *with what error string*, not a truncated toast.
#[derive(Debug)]
pub struct RequestErrorSnapshot<'a> {
    /// HTTP method as the caller requested it.
    pub method: &'a str,
    /// URL the caller tried to reach (pre-redirect).
    pub url: &'a str,
    /// `Display`-formatted error from `core::error::Error`.
    pub error: &'a str,
}

/// Type alias for the request-error observer callback. Held in `Arc` so
/// cheap to share across the global slot and any callers that want to
/// keep a handle.
pub type ErrorObserver = Arc<dyn Fn(&RequestErrorSnapshot<'_>) + Send + Sync + 'static>;

static ERROR_OBSERVER: OnceLock<ErrorObserver> = OnceLock::new();

/// Register the global request-error observer. First write wins;
/// subsequent calls are no-ops.
pub fn set_request_error_observer<F>(f: F)
where
    F: Fn(&RequestErrorSnapshot<'_>) + Send + Sync + 'static,
{
    let _ = ERROR_OBSERVER.set(Arc::new(f));
}

/// Internal: fire the error observer if one is registered. Called from
/// the session execute path on every `Err` exit.
pub(crate) fn notify_request_error(snap: &RequestErrorSnapshot<'_>) {
    if let Some(obs) = ERROR_OBSERVER.get() {
        obs(snap);
    }
}
