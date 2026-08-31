//! Response observer hook — single global callback fired once per fully-assembled, non-streaming response.

use std::sync::{Arc, OnceLock};

/// Snapshot of a response handed to the observer.
#[derive(Debug)]
pub struct ResponseSnapshot<'a> {
    /// HTTP method as sent on the final request hop.
    pub method: &'a str,
    /// Originally requested URL (before any redirects).
    pub url: &'a str,
    /// URL after redirect resolution.
    pub final_url: &'a str,
    /// HTTP status code of the final response.
    pub status: u16,
    /// Outbound request headers as they actually went on the wire on the final hop (after preset assembly, identity overlays, brand extras, caller anchors, cookie injection, and identity-level reordering).
    pub request_headers: &'a [(String, String)],
    /// Response headers (crate-private storage; read via the [`response_headers`](ResponseSnapshot::response_headers) accessor so the storage type never leaks into the public API).
    pub(crate) response_headers_raw: &'a [(crate::core::HeaderStr, crate::core::HeaderStr)],
    /// Response body bytes, **after** decompression.
    pub body: &'a [u8],
}

impl<'a> ResponseSnapshot<'a> {
    /// Response headers as received from the wire, in order.
    pub fn response_headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.response_headers_raw
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

/// Type alias for the observer callback.
pub type Observer = Arc<dyn Fn(&ResponseSnapshot<'_>) + Send + Sync + 'static>;

static OBSERVER: OnceLock<Observer> = OnceLock::new();

/// Register the global response observer.
pub fn set_response_observer<F>(f: F)
where
    F: Fn(&ResponseSnapshot<'_>) + Send + Sync + 'static,
{
    let _ = OBSERVER.set(Arc::new(f));
}

/// Internal: fire the observer if one is registered.
pub(crate) fn notify_response(snap: &ResponseSnapshot<'_>) {
    if let Some(obs) = OBSERVER.get() {
        obs(snap);
    }
}

/// Internal: whether a response observer is registered.
pub(crate) fn has_observer() -> bool {
    OBSERVER.get().is_some()
}

/// Snapshot of a request *failure* handed to the error observer.
#[derive(Debug)]
pub struct RequestErrorSnapshot<'a> {
    /// HTTP method as the caller requested it.
    pub method: &'a str,
    /// URL the caller tried to reach (pre-redirect).
    pub url: &'a str,
    /// `Display`-formatted error from `core::error::Error`.
    pub error: &'a str,
}

/// Type alias for the request-error observer callback.
pub type ErrorObserver = Arc<dyn Fn(&RequestErrorSnapshot<'_>) + Send + Sync + 'static>;

static ERROR_OBSERVER: OnceLock<ErrorObserver> = OnceLock::new();

/// Register the global request-error observer.
pub fn set_request_error_observer<F>(f: F)
where
    F: Fn(&RequestErrorSnapshot<'_>) + Send + Sync + 'static,
{
    let _ = ERROR_OBSERVER.set(Arc::new(f));
}

/// Internal: fire the error observer if one is registered.
pub(crate) fn notify_request_error(snap: &RequestErrorSnapshot<'_>) {
    if let Some(obs) = ERROR_OBSERVER.get() {
        obs(snap);
    }
}
