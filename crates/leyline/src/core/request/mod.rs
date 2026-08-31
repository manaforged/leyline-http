//! Fluent request builder.

#![forbid(unsafe_code)]
mod compress;
mod encode;
mod send;

pub use compress::ContentEncoding;

use std::time::Duration;

use crate::profile::{HeaderAnchor, Preset};

use crate::core::body::Body;
use crate::core::digest::DigestAuth;
use crate::core::error::Error;
use crate::core::headers::HeaderList;
#[cfg(feature = "multipart")]
use crate::core::multipart::Form;
use crate::core::retry::RetryPolicy;
use crate::core::session::Session;

/// A key-value pair that can be used by request helper methods.
pub trait IntoParamPair {
    /// Convert into owned `(name, value)` strings.
    fn into_param_pair(self) -> (String, String);
}

impl<K, V> IntoParamPair for (K, V)
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    fn into_param_pair(self) -> (String, String) {
        (self.0.as_ref().to_string(), self.1.as_ref().to_string())
    }
}

impl<K, V> IntoParamPair for &(K, V)
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    fn into_param_pair(self) -> (String, String) {
        (self.0.as_ref().to_string(), self.1.as_ref().to_string())
    }
}

#[must_use = "builders are lazy: nothing happens until `.send()` / `.build()`"]
/// Fluent builder for constructing and sending HTTP requests.
pub struct RequestBuilder {
    pub(super) session: Session,
    pub(super) method: String,
    pub(super) url: String,
    pub(super) preset: Option<Preset>,
    pub(super) body: Body,
    pub(super) headers: HeaderList,
    pub(super) query_params: Vec<(String, String)>,
    pub(super) timeout: Option<Duration>,
    pub(super) builder_error: Option<Error>,
    pub(super) stream_response: bool,
    /// When `Some`, the buffered body is compressed with this codec and a matching `Content-Encoding` header is set at send time.
    pub(super) compress: Option<ContentEncoding>,
    pub(super) retry_policy: RetryPolicy,
    pub(super) allow_non_idempotent_retry: bool,
    pub(super) digest_auth: Option<DigestAuth>,
    /// Per-request proxy override.
    pub(super) proxy: Option<String>,
    /// Per-request wire header order (H2/H3 only).
    pub(super) header_order: Option<Vec<String>>,
    pub(super) preset_user: bool,
}

fn default_preset(session: &Session, method: &str) -> Option<Preset> {
    session.browser()?;
    match method {
        "GET" | "HEAD" => Some(Preset::Navigate),
        _ => None,
    }
}

impl RequestBuilder {
    pub(crate) fn new(session: &Session, method: &str, url: &str) -> Self {
        Self {
            session: session.clone(),
            method: method.to_string(),
            url: url.to_string(),
            preset: default_preset(session, method),
            body: Body::Empty,
            headers: HeaderList::new(),
            query_params: Vec::new(),
            timeout: None,
            builder_error: None,
            stream_response: false,
            compress: None,
            retry_policy: session.default_retry().clone(),
            allow_non_idempotent_retry: false,
            digest_auth: None,
            proxy: None,
            header_order: None,
            preset_user: false,
        }
    }

    fn infer_body_preset(&mut self, preset: Preset) {
        if self.preset_user || self.session.browser().is_none() {
            return;
        }
        if matches!(self.method.as_str(), "POST" | "PUT" | "PATCH") {
            self.preset = Some(preset);
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
        self.preset_user = true;
        self
    }

    /// Pin the exact H2/H3 wire order of the regular headers for this request.
    pub fn header_order(mut self, order: &[&str]) -> Self {
        self.header_order = Some(order.iter().map(|s| (*s).to_string()).collect());
        self
    }

    /// Set the request body.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// Set the request body as JSON.
    pub fn json(mut self, value: &impl serde::Serialize) -> Self {
        self.infer_body_preset(Preset::Xhr);
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

    /// Set the request body as URL-encoded form data.
    pub fn form<I, P>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        self.infer_body_preset(Preset::Form);
        let pairs = collect_pairs(params);
        let encoded = encode::url_encode_pairs(&pairs);
        self.headers
            .set("content-type", "application/x-www-form-urlencoded");
        self.body = Body::from(encoded.into_bytes());
        self
    }

    /// Set the request body as a pre-encoded form string.
    pub fn form_str(mut self, encoded: &str) -> Self {
        self.infer_body_preset(Preset::Form);
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

    /// Compress the request body with `encoding` and set the matching `Content-Encoding` header.
    pub fn compress(mut self, encoding: ContentEncoding) -> Self {
        self.compress = Some(encoding);
        self
    }

    /// Add URL query parameters.
    pub fn query<I, P>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        for pair in params {
            self.query_params.push(pair.into_param_pair());
        }
        self
    }

    /// Set a request header.
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
    pub fn headers<I, P>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        for pair in headers {
            let (k, v) = pair.into_param_pair();
            self.headers.set(k, v);
        }
        self
    }

    /// Append multiple headers, preserving duplicate names and order.
    pub fn append_headers<I, P>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        for pair in headers {
            let (k, v) = pair.into_param_pair();
            self.headers.append(k, v);
        }
        self
    }

    /// Set `accept`.
    pub fn accept(self, value: &str) -> Self {
        self.header("accept", value)
    }

    /// Set `accept-language`.
    pub fn accept_language(self, value: &str) -> Self {
        self.header("accept-language", value)
    }

    /// Set `user-agent`.
    pub fn user_agent(self, value: &str) -> Self {
        self.header("user-agent", value)
    }

    /// Set `referer`.
    pub fn referer(self, value: &str) -> Self {
        self.header("referer", value)
    }

    /// Set `origin`.
    pub fn origin(self, value: &str) -> Self {
        self.header("origin", value)
    }

    /// Set `content-type`.
    pub fn content_type(self, value: &str) -> Self {
        self.header("content-type", value)
    }

    /// Append a header at a caller-specified anchor slot.
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
        let encoded = crate::util::base64_encode(&format!("{username}:{password}"));
        self.headers
            .set("authorization", format!("Basic {encoded}"));
        self
    }

    /// Attach a [`RetryPolicy`] to this request, overriding the session policy.
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
    #[cfg(feature = "multipart")]
    pub fn multipart(mut self, form: Form) -> Self {
        self.headers.set("content-type", form.content_type());
        self.body = form.into_stream_body();
        self
    }

    /// Override the session's proxy for this single request.
    pub fn proxy(mut self, proxy_url: &str) -> Self {
        self.proxy = Some(proxy_url.to_string());
        self
    }

    /// Override the session proxy with a validated [`ProxyUrl`](crate::ProxyUrl).
    pub fn proxy_url(mut self, proxy_url: crate::ProxyUrl) -> Self {
        self.proxy = Some(proxy_url.into_string());
        self
    }
}

fn collect_pairs<I, P>(params: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = P>,
    P: IntoParamPair,
{
    params
        .into_iter()
        .map(IntoParamPair::into_param_pair)
        .collect()
}
