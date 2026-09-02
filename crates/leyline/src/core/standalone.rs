//! Standalone [`Request`] — a value type that carries everything needed to dispatch an HTTP request against a [`crate::Session`].

use std::time::Duration;

use http::{HeaderName, HeaderValue, Method, Uri};

use crate::core::body::Body;
use crate::core::digest::DigestAuth;
use crate::core::headers::HeaderList;
use crate::core::retry::RetryPolicy;
use crate::profile::Preset;

/// Owned HTTP request. `Session::execute` infers Xhr/Form from `content-type` unless `preset` is set.
#[non_exhaustive]
pub struct Request {
    /// Method (`GET`, `POST`, ...).
    pub method: Method,
    /// Absolute request URL.
    pub url: Uri,
    /// Extra headers to merge with the session's preset headers.
    pub headers: HeaderList,
    /// Request body.
    pub body: Body,
    /// Optional per-request timeout override.
    pub timeout: Option<Duration>,
    /// Opt-in retry policy (default: no retry).
    pub retry_policy: Option<RetryPolicy>,
    /// Opt-in digest auth challenge handler.
    pub digest_auth: Option<DigestAuth>,
    /// When `true`, retry even non-idempotent methods (`POST`, `PATCH`).
    pub allow_non_idempotent_retry: bool,
    /// When `true`, the response body is delivered as a stream rather than buffered into `Vec<u8>`.
    pub stream_response: bool,
    /// Request preset. `None` lets `Session::execute` infer from `content-type`.
    pub preset: Option<Preset>,
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
    /// Build a new request with the given method and URL. An unparsable URL surfaces when the session executes the request.
    pub fn new(method: Method, url: impl TryInto<Uri>) -> Self {
        Self {
            method,
            url: url.try_into().unwrap_or_default(),
            headers: HeaderList::new(),
            body: Body::Empty,
            timeout: None,
            retry_policy: None,
            digest_auth: None,
            allow_non_idempotent_retry: false,
            stream_response: false,
            preset: None,
        }
    }

    /// Attach a retry policy.
    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
        self
    }

    /// Opt into retrying non-idempotent methods (`POST`, `PATCH`).
    pub fn allow_non_idempotent_retry(mut self, v: bool) -> Self {
        self.allow_non_idempotent_retry = v;
        self
    }

    /// Attach digest credentials for RFC 7616 challenge-response auth.
    pub fn digest_auth(mut self, auth: DigestAuth) -> Self {
        self.digest_auth = Some(auth);
        self
    }

    /// Opt into streaming response delivery.
    pub fn stream(mut self) -> Self {
        self.stream_response = true;
        self
    }

    /// Set a request header, replacing any prior value with the same name. An invalid name or value is dropped.
    pub fn header(
        mut self,
        name: impl TryInto<HeaderName>,
        value: impl TryInto<HeaderValue>,
    ) -> Self {
        drop(self.headers.set(name, value));
        self
    }

    /// Set the request body.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// Pin the request preset instead of inferring it from `content-type`.
    pub fn preset(mut self, preset: Preset) -> Self {
        self.preset = Some(preset);
        self
    }

    /// Set a per-request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}
