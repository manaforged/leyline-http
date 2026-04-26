//! Fluent request builder.

mod encode;
mod send;

use std::time::Duration;

use crate::profile::{HeaderAnchor, Preset};

use crate::core::body::Body;
use crate::core::digest::DigestAuth;
use crate::core::error::Error;
use crate::core::headers::HeaderList;
use crate::core::multipart::Form;
use crate::core::retry::RetryPolicy;
use crate::core::session::Session;

/// Fluent builder for constructing and sending HTTP requests.
///
/// ```rust,ignore
/// let resp = session.post("https://api.example.com/items")
///     .preset(Preset::Xhr)
///     .json(&payload)
///     .bearer_auth("token123")
///     .header("x-request-id", "abc")
///     .send()
///     .await?;
/// ```
pub struct RequestBuilder<'a> {
    pub(super) session: &'a Session,
    pub(super) method: String,
    pub(super) url: String,
    pub(super) preset: Option<Preset>,
    pub(super) body: Body,
    pub(super) headers: HeaderList,
    pub(super) query_params: Vec<(String, String)>,
    pub(super) timeout: Option<Duration>,
    pub(super) builder_error: Option<Error>,
    pub(super) stream_response: bool,
    pub(super) retry_policy: RetryPolicy,
    pub(super) allow_non_idempotent_retry: bool,
    pub(super) digest_auth: Option<DigestAuth>,
    /// Per-request proxy override. When `Some`, this proxy is used
    /// instead of the session's default proxy for this request only.
    /// Other requests on the same session are unaffected — the pool
    /// keys connections by `(host, port, proxy)` so the session can
    /// multiplex traffic across multiple proxies.
    pub(super) proxy: Option<String>,
}

impl<'a> RequestBuilder<'a> {
    pub(crate) fn new(session: &'a Session, method: &str, url: &str) -> Self {
        Self {
            session,
            method: method.to_string(),
            url: url.to_string(),
            preset: None,
            body: Body::Empty,
            headers: HeaderList::new(),
            query_params: Vec::new(),
            timeout: None,
            builder_error: None,
            stream_response: false,
            retry_policy: RetryPolicy::none(),
            allow_non_idempotent_retry: false,
            digest_auth: None,
            proxy: None,
        }
    }

    /// Override the session's default timeout for this one request.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set a request preset (Navigate, Script, Xhr, Form, CrossOrigin, SameSite).
    pub fn preset(mut self, preset: Preset) -> Self {
        self.preset = Some(preset);
        self
    }

    /// Set the request body.
    ///
    /// Accepts any `Into<Body>` — `Vec<u8>`, `&'static [u8]`, `String`,
    /// `&'static str`, `bytes::Bytes`, or a prebuilt [`Body`] (including
    /// `Body::stream(...)` for a streaming upload that will be pumped
    /// to the wire without materialising the full payload).
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// Set the request body as JSON. Sets `content-type: application/json`.
    pub fn json(mut self, value: &impl serde::Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(bytes) => {
                self.headers.set("content-type", "application/json");
                self.body = Body::from(bytes);
            }
            Err(e) => {
                self.builder_error = Some(Error::Json(e));
            }
        }
        self
    }

    /// Set the request body as URL-encoded form data. Sets content-type automatically.
    pub fn form(mut self, params: &[(&str, &str)]) -> Self {
        let encoded = encode::url_encode_pairs(params);
        self.headers
            .set("content-type", "application/x-www-form-urlencoded");
        self.body = Body::from(encoded.into_bytes());
        self
    }

    /// Set the request body as a pre-encoded form string. Sets content-type automatically.
    pub fn form_str(mut self, encoded: &str) -> Self {
        self.headers
            .set("content-type", "application/x-www-form-urlencoded");
        self.body = Body::from(encoded.as_bytes().to_vec());
        self
    }

    /// Opt into streaming response delivery.
    pub fn stream(mut self) -> Self {
        self.stream_response = true;
        self
    }

    /// Add URL query parameters. Can be called multiple times.
    pub fn query(mut self, params: &[(&str, &str)]) -> Self {
        for &(k, v) in params {
            self.query_params.push((k.to_string(), v.to_string()));
        }
        self
    }

    /// Set a request header.
    ///
    /// For headers with a well-known Chrome slot (`origin`,
    /// `authorization`, `x-csrf-token`, `x-requested-with`, etc.) the
    /// profile picks the anchor automatically. For site-specific
    /// headers with no universal rule (for example `x-extra-*`),
    /// use [`anchored`](Self::anchored) and name the slot explicitly.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.set(name, value);
        self
    }

    /// Append a request header without replacing existing values with the same name.
    pub fn append_header(mut self, name: &str, value: &str) -> Self {
        self.headers.append(name, value);
        self
    }

    /// Set multiple headers at once.
    pub fn headers(mut self, headers: &[(&str, &str)]) -> Self {
        for &(k, v) in headers {
            self.headers.set(k, v);
        }
        self
    }

    /// Append multiple headers, preserving duplicate names and order.
    pub fn append_headers(mut self, headers: &[(&str, &str)]) -> Self {
        for &(k, v) in headers {
            self.headers.append(k, v);
        }
        self
    }

    /// Append a header at a caller-specified anchor slot.
    ///
    /// Use this when the header has no universal Chrome rule — the
    /// profile cannot infer where it goes, so the caller declares
    /// the slot explicitly. Typical case: WAF-emitted headers whose
    /// position depends on the target site's capture.
    ///
    /// ```rust,ignore
    /// use crate::profile::HeaderAnchor;
    /// session.post(url)
    ///     .anchored(HeaderAnchor::AfterUserAgent, "x-extra-6", c_val)
    ///     .anchored(HeaderAnchor::AfterContentType, "x-extra-7", d_val)
    ///     .body(payload)
    ///     .send().await?;
    /// ```
    pub fn anchored(mut self, anchor: HeaderAnchor, name: &str, value: &str) -> Self {
        self.headers.append_anchored(anchor, name, value);
        self
    }

    /// Set a Bearer token for the Authorization header.
    pub fn bearer_auth(mut self, token: &str) -> Self {
        self.headers.set("authorization", format!("Bearer {token}"));
        self
    }

    /// Set Basic auth for the Authorization header.
    pub fn basic_auth(mut self, username: &str, password: &str) -> Self {
        let encoded = encode::base64_encode(&format!("{username}:{password}"));
        self.headers
            .set("authorization", format!("Basic {encoded}"));
        self
    }

    /// Attach a [`RetryPolicy`] to this request.
    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Opt into retrying non-idempotent methods (POST / PATCH).
    pub fn allow_non_idempotent_retry(mut self, allow: bool) -> Self {
        self.allow_non_idempotent_retry = allow;
        self
    }

    /// Enable HTTP Digest authentication for this request.
    pub fn digest_auth(mut self, auth: DigestAuth) -> Self {
        self.digest_auth = Some(auth);
        self
    }

    /// Send the request as `multipart/form-data`.
    pub fn multipart(mut self, form: Form) -> Self {
        self.headers.set("content-type", form.content_type());
        self.body = form.into_stream_body();
        self
    }

    /// Override the session's proxy for this single request.
    ///
    /// The session's connection pool keys connections by
    /// `(host, port, proxy)`, so a session can multiplex requests
    /// across multiple proxies cheaply — the first request through a
    /// new proxy pays one TLS handshake, subsequent requests through
    /// the same proxy reuse the cached connection.
    ///
    /// Pass `http://user:pass@host:port` for HTTP proxies or
    /// `socks5://user:pass@host:port` for SOCKS5. The session's
    /// `NO_PROXY` rules still apply — if the URL host matches a
    /// `NO_PROXY` pattern, the override is ignored just like the
    /// session-default proxy would be.
    pub fn proxy(mut self, proxy_url: &str) -> Self {
        self.proxy = Some(proxy_url.to_string());
        self
    }
}
