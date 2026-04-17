//! Standalone [`Request`] — a value type that carries everything
//! needed to dispatch an HTTP request against a [`crate::Session`].
//!
//! [`crate::RequestBuilder`] is the canonical fluent API and borrows
//! the session for its lifetime. Some ecosystems (tower, axum
//! middleware, rate-limiter stacks) can only work with an *owned*
//! value type — they pump `Request`s through a pipeline and call
//! `Service::call(req)` at the end. [`Request`] is the value those
//! adapters pass around.
//!
//! You don't normally need to build a `Request` yourself — the
//! `leyline-tower` crate's `LeylineService` adapts
//! `Service<Request>` to a session.

use std::time::Duration;

use crate::body::Body;
use crate::headers::HeaderList;

/// A standalone, owned HTTP request.
///
/// Fields are all public so middleware layers can rewrite any of them
/// before dispatch without having to round-trip through setters.
pub struct Request {
    /// Method (`GET`, `POST`, ...).
    pub method: String,
    /// Absolute request URL.
    pub url: String,
    /// Extra headers to merge with the session's preset headers.
    pub headers: HeaderList,
    /// Request body.
    pub body: Body,
    /// Optional per-request timeout override.
    pub timeout: Option<Duration>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .field("body", &self.body)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl Request {
    /// Build a new request with the given method and URL. Headers and
    /// body are empty.
    pub fn new(method: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            headers: HeaderList::new(),
            body: Body::Empty,
            timeout: None,
        }
    }

    /// Convenience: build a `GET` request.
    pub fn get(url: impl Into<String>) -> Self {
        Self::new("GET", url)
    }

    /// Convenience: build a `POST` request.
    pub fn post(url: impl Into<String>) -> Self {
        Self::new("POST", url)
    }

    /// Set a request header, replacing any prior value with the same name.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.set(name, value);
        self
    }

    /// Set the request body.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// Set a per-request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}
