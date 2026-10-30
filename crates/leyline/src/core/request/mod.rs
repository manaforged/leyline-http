#![forbid(unsafe_code)]
mod compress;
mod context;
mod debug;
mod encode;
mod route;
mod send;
#[cfg(feature = "websocket")]
mod websocket;

pub use compress::Encoding;

use http::{HeaderName, HeaderValue, Method};

use crate::profile::{HeaderPosition, RequestKind};

use crate::core::body::Body;
use crate::core::config::{BodyRules, Proxy, Redirect, Timeout};
use crate::core::digest::DigestAuth;
use crate::core::error::{Cause, Error};
use crate::core::headers::HeaderList;
use crate::core::into_url::IntoUrl;
#[cfg(feature = "multipart")]
use crate::core::multipart::Form;
use crate::core::retry::Retry;
use crate::core::session::Session;

mod sealed {
    pub trait Sealed {}
}

/// Converts a value into a name and value string pair.
pub trait IntoParamPair: sealed::Sealed {
    /// Returns the pair as owned strings.
    fn into_param_pair(self) -> (String, String);
}

impl<K: AsRef<str>, V: AsRef<str>> sealed::Sealed for (K, V) {}
impl<K: AsRef<str>, V: AsRef<str>> sealed::Sealed for &(K, V) {}

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

#[doc = include_str!("../../../docs/request.md")]
#[must_use = "builders are lazy: nothing happens until `.send()` / `.build()`"]
pub struct RequestBuilder {
    pub(super) session: Session,
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) preset: Option<RequestKind>,
    pub(super) body: Body,
    pub(super) headers: HeaderList,
    pub(super) query_params: Vec<(String, String)>,
    pub(super) timeouts: Option<Timeout>,
    pub(super) builder_error: Option<Error>,
    pub(super) stream_response: bool,
    pub(super) compress: Option<Encoding>,
    pub(super) retry_policy: Retry,
    pub(super) digest_auth: Option<DigestAuth>,
    pub(super) proxy: Option<Proxy>,
    pub(super) header_order: Option<Vec<String>>,
    pub(super) preset_user: bool,
    pub(super) redirect: Option<Redirect>,
    pub(super) initiator: Option<url::Url>,
    pub(crate) trusted_origin: Option<url::Url>,
    pub(super) label: Option<String>,
    pub(super) status_errors: bool,
    pub(super) body_rules: BodyRules,
}

fn default_preset(session: &Session, method: &Method) -> Option<RequestKind> {
    if !session.impersonates() {
        return None;
    }
    match *method {
        Method::GET | Method::HEAD => Some(RequestKind::Navigate),
        _ => None,
    }
}

impl RequestBuilder {
    pub(crate) fn new(session: &Session, method: Method, url: &str) -> Self {
        let preset = default_preset(session, &method);
        Self {
            session: session.clone(),
            method,
            url: url.to_string(),
            preset,
            body: Body::default(),
            headers: HeaderList::new(),
            query_params: Vec::new(),
            timeouts: None,
            builder_error: None,
            stream_response: false,
            compress: None,
            retry_policy: session.default_retry().clone(),
            digest_auth: None,
            proxy: None,
            header_order: None,
            preset_user: false,
            redirect: None,
            initiator: None,
            trusted_origin: None,
            label: None,
            status_errors: false,
            body_rules: session.body_rules(),
        }
    }

    pub(crate) fn from_url(session: &Session, method: Method, url: impl IntoUrl) -> Self {
        match url.into_url_with_base(session.base_url()) {
            Ok(url) => Self::new(session, method, url.as_str()),
            Err(err) => {
                let mut builder = Self::new(session, method, "");
                builder.builder_error = Some(err);
                builder
            }
        }
    }

    pub(crate) fn fail(&mut self, err: Error) {
        if self.builder_error.is_none() {
            self.builder_error = Some(err);
        }
    }

    fn infer_from_content_type(&mut self) {
        if self.preset_user || !self.session.impersonates() {
            return;
        }
        if !matches!(self.method, Method::POST | Method::PUT | Method::PATCH) {
            return;
        }
        let Some(ct) = self.headers.get("content-type") else {
            return;
        };
        let ct = String::from_utf8_lossy(ct.as_bytes()).to_ascii_lowercase();
        if ct.starts_with("application/json") {
            self.preset = Some(RequestKind::Xhr);
        } else if ct.starts_with("application/x-www-form-urlencoded") {
            self.preset = Some(RequestKind::Form);
        }
    }

    /// Sets the timeouts for this request, overriding the session timeouts.
    pub fn timeout(mut self, config: impl Into<Timeout>) -> Self {
        self.timeouts = Some(config.into());
        self
    }

    /// Sets the browser request kind that selects the header preset.
    pub fn kind(mut self, kind: RequestKind) -> Self {
        self.preset = Some(kind);
        self.preset_user = true;
        self
    }

    /// Sets the order in which the named headers are sent.
    pub fn header_order<I, S>(mut self, order: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.header_order = Some(order.into_iter().map(|s| s.as_ref().to_owned()).collect());
        self
    }

    /// Sets the request body.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// Serializes `value` as the JSON body and sets `content-type` to `application/json`.
    pub fn json(mut self, value: &impl serde::Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(bytes) => {
                self.put("content-type", "application/json");
                self.body = Body::from(bytes);
            }
            Err(e) => {
                self.builder_error = Some(Error::new(Cause::Json).with_source(e));
            }
        }
        self
    }

    /// Encodes `params` as a URL-encoded form body and sets the matching `content-type`.
    pub fn form<I, P>(mut self, params: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        let pairs = collect_pairs(params);
        let encoded = encode::url_encode_pairs(&pairs);
        self.put("content-type", "application/x-www-form-urlencoded");
        self.body = Body::from(encoded.into_bytes());
        self
    }

    /// Sets whether the response body is decompressed, overriding the session setting.
    pub fn decode(mut self, on: bool) -> Self {
        self.body_rules.decode = on;
        self
    }

    /// Sets the maximum response body size in bytes, overriding the session setting.
    pub fn max_body_size(mut self, bytes: usize) -> Self {
        self.body_rules.max_body_size = bytes;
        self
    }

    /// Returns the response as soon as the headers arrive and leaves the body to stream.
    pub fn stream(mut self) -> Self {
        self.stream_response = true;
        self
    }

    /// Compresses the request body with the given encoding before sending.
    pub fn compress_body(mut self, encoding: Encoding) -> Self {
        self.compress = Some(encoding);
        self
    }

    /// Appends `params` to the URL query string.
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

    /// Appends a header and records an error for `send` when the name or value is invalid.
    pub fn header(
        mut self,
        name: impl TryInto<HeaderName>,
        value: impl TryInto<HeaderValue>,
    ) -> Self {
        if let Err(err) = self.headers.append(name, value) {
            self.fail(err);
        }
        self
    }

    fn put(&mut self, name: impl TryInto<HeaderName>, value: impl TryInto<HeaderValue>) {
        if let Err(err) = self.headers.set(name, value) {
            self.fail(err);
        }
    }

    /// Appends each name and value pair as a header.
    pub fn headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: TryInto<HeaderName>,
        V: TryInto<HeaderValue>,
    {
        for (name, value) in headers {
            if let Err(err) = self.headers.append(name, value) {
                self.fail(err);
                return self;
            }
        }
        self
    }

    /// Inserts a header at the given position in the header order.
    pub fn header_at(
        mut self,
        anchor: HeaderPosition,
        name: impl TryInto<HeaderName>,
        value: impl TryInto<HeaderValue>,
    ) -> Self {
        if let Err(err) = self.headers.append_anchored(anchor, name, value) {
            self.fail(err);
        }
        self
    }

    /// Sets the `authorization` header to a bearer token.
    pub fn bearer_auth(mut self, token: &str) -> Self {
        self.put("authorization", format!("Bearer {token}"));
        self
    }

    /// Sets the `authorization` header to HTTP Basic credentials.
    pub fn basic_auth(mut self, username: &str, password: &str) -> Self {
        let encoded = crate::util::base64_encode(&format!("{username}:{password}"));
        self.put("authorization", format!("Basic {encoded}"));
        self
    }

    /// Sets the retry policy for this request, overriding the session default.
    pub fn retry(mut self, retry: impl Into<Retry>) -> Self {
        self.retry_policy = retry.into();
        self
    }

    /// Sets the redirect policy for this request, overriding the session policy.
    pub fn redirect(mut self, policy: impl Into<Redirect>) -> Self {
        self.redirect = Some(policy.into());
        self
    }

    /// Answers an HTTP Digest challenge with the given credentials.
    pub fn digest_auth(mut self, username: &str, password: &str) -> Self {
        self.digest_auth = Some(DigestAuth::new(username, password));
        self
    }

    /// Sets a multipart form as the streamed body and sets the matching `content-type`.
    #[cfg(feature = "multipart")]
    pub fn multipart(mut self, form: Form) -> Self {
        self.put("content-type", form.content_type());
        self.body = form.into_stream_body();
        self
    }

    /// Sends this request through the given proxy, and fails at send when the proxy is a pool.
    pub fn proxy(mut self, config: impl Into<Proxy>) -> Self {
        let config = config.into();
        if config.pool.is_some() {
            self.fail(
                Error::new(Cause::Config)
                    .with_message("a request takes one proxy; set a proxy list on the session"),
            );
        }
        self.proxy = Some(config);
        self
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

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
