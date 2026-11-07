#![forbid(unsafe_code)]
mod compress;
mod context;
mod debug;
mod download;
mod encode;
mod events;
pub use events::{Event, Events};
mod pages;
mod route;
mod send;
#[cfg(feature = "websocket")]
mod websocket;

use std::borrow::Borrow;

use http::{HeaderName, HeaderValue, Method};

use crate::profile::{Encoding, HeaderPosition, RequestKind};

use crate::core::config::Proxy;

use crate::core::BodyRules;

use crate::core::Redirect;
use crate::core::digest::DigestAuth;
use crate::core::headers::HeaderList;
use crate::core::into_url::IntoUrl;
#[cfg(feature = "multipart")]
use crate::core::multipart::Form;
use crate::core::retry::Retry;
use crate::core::session::Session;
use crate::deadline::Deadline;
use crate::deadline::Timeout;
use crate::error::{Error, Kind};
use crate::wire::Body;
use crate::wire::ResponseMode;

#[doc = include_str!("../../../docs/request.md")]
#[must_use = "builders are lazy: nothing happens until `.send()` / `.build()`"]
pub struct RequestBuilder {
    session: Session,
    draft: Box<Draft>,
}

pub(crate) struct Draft {
    pub(crate) method: Method,
    pub(crate) url: Option<std::sync::Arc<url::Url>>,
    pub(crate) body: Body,
    pub(crate) headers: HeaderList,
    query_params: Vec<(String, String)>,
    builder_error: Option<Error>,
    compress: Option<Encoding>,
    pub(crate) trusted_origin: Option<url::Url>,
    pub(crate) deadline: Deadline,
    pub(crate) response: ResponseMode,
    download: Option<Box<download::Download>>,
    pub(crate) carry: Carry,
}

#[derive(Clone)]
pub(crate) struct Carry {
    pub(crate) preset: Option<RequestKind>,
    pub(crate) preset_user: bool,
    pub(crate) timeouts: Option<Timeout>,
    pub(crate) stream_response: bool,
    pub(crate) retry_policy: Retry,
    pub(crate) digest_auth: Option<DigestAuth>,
    pub(crate) proxy: Option<Proxy>,
    pub(crate) header_order: Option<Vec<String>>,
    pub(crate) redirect: Option<Redirect>,
    pub(crate) initiator: Option<url::Url>,
    pub(crate) label: Option<String>,
    pub(crate) status_errors: bool,
    pub(crate) body_rules: BodyRules,
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

impl Draft {
    fn new(session: &Session, method: Method, url: Option<std::sync::Arc<url::Url>>) -> Self {
        let preset = default_preset(session, &method);
        Self {
            method,
            url,
            body: Body::default(),
            headers: HeaderList::new(),
            query_params: Vec::new(),
            builder_error: None,
            compress: None,
            trusted_origin: None,
            deadline: Deadline::default(),
            response: ResponseMode::Buffered,
            download: None,
            carry: Carry {
                preset,
                preset_user: false,
                timeouts: None,
                stream_response: false,
                retry_policy: session.default_retry().clone(),
                digest_auth: None,
                proxy: None,
                header_order: None,
                redirect: None,
                initiator: None,
                label: None,
                status_errors: false,
                body_rules: session.body_rules(),
            },
        }
    }

    fn again(&self, body: Body, headers: HeaderList) -> Box<Draft> {
        Box::new(Draft {
            method: self.method.clone(),
            url: self.url.clone(),
            body,
            headers,
            query_params: Vec::new(),
            builder_error: None,
            compress: None,
            trusted_origin: self.trusted_origin.clone(),
            deadline: self.deadline,
            response: self.response,
            download: None,
            carry: self.carry.clone(),
        })
    }

    pub(crate) fn method(&self) -> &Method {
        &self.method
    }

    pub(crate) fn url(&self) -> Option<&url::Url> {
        self.url.as_deref()
    }

    pub(crate) fn deadline(&self) -> &Deadline {
        &self.deadline
    }
}

impl RequestBuilder {
    pub(crate) fn new(
        session: &Session,
        method: Method,
        url: Option<std::sync::Arc<url::Url>>,
    ) -> Self {
        Self {
            session: session.clone(),
            draft: Box::new(Draft::new(session, method, url)),
        }
    }

    pub(crate) fn trusting(mut self, origin: Option<url::Url>) -> Self {
        self.draft.trusted_origin = origin;
        self
    }

    pub(crate) fn from_url(session: &Session, method: Method, url: impl IntoUrl) -> Self {
        match crate::core::into_url::sealed::Sealed::shared_url(url, session.base_url()) {
            Ok(url) => Self::new(session, method, Some(url)),
            Err(err) => {
                let mut builder = Self::new(session, method, None);
                builder.draft.builder_error = Some(err);
                builder
            }
        }
    }

    pub(crate) fn fail(&mut self, err: Error) {
        if self.draft.builder_error.is_none() {
            self.draft.builder_error = Some(err);
        }
    }

    fn infer_from_content_type(&mut self) {
        if self.draft.carry.preset_user || !self.session.impersonates() {
            return;
        }
        if !crate::wire::has_body_by_default(&self.draft.method) {
            return;
        }
        let Some(ct) = self.draft.headers.get("content-type") else {
            return;
        };
        let ct = String::from_utf8_lossy(ct.as_bytes()).to_ascii_lowercase();
        if ct.starts_with("application/json") {
            self.draft.carry.preset = Some(RequestKind::Xhr);
        } else if ct.starts_with("application/x-www-form-urlencoded") {
            self.draft.carry.preset = Some(RequestKind::Form);
        }
    }

    /// Sets the timeouts for this request, overriding the session timeouts.
    pub fn timeout(mut self, config: impl Into<Timeout>) -> Self {
        self.draft.carry.timeouts = Some(config.into());
        self
    }

    /// Sets the browser request kind that selects the header preset.
    pub fn kind(mut self, kind: RequestKind) -> Self {
        self.draft.carry.preset = Some(kind);
        self.draft.carry.preset_user = true;
        self
    }

    /// Sets the order in which the named headers are sent.
    pub fn header_order<I, S>(mut self, order: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.draft.carry.header_order =
            Some(order.into_iter().map(|s| s.as_ref().to_owned()).collect());
        self
    }

    /// Sets the request body.
    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.draft.body = body.into();
        self
    }

    /// Serializes `value` as the JSON body and sets `content-type` to `application/json`.
    pub fn json(mut self, value: &impl serde::Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(bytes) => {
                self.put("content-type", "application/json");
                self.draft.body = Body::from(bytes);
            }
            Err(e) => {
                self.draft.builder_error = Some(Error::new(Kind::Json).with_source(e));
            }
        }
        self
    }

    /// Encodes `params` as a URL-encoded form body and sets the matching `content-type`.
    pub fn form<I, K, V>(mut self, params: I) -> Self
    where
        I: IntoIterator,
        I::Item: Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let pairs = collect_pairs(params);
        let encoded = encode::url_encode_pairs(&pairs);
        self.put("content-type", "application/x-www-form-urlencoded");
        self.draft.body = Body::from(encoded.into_bytes());
        self
    }

    /// Sets whether the response body is decompressed, overriding the session setting.
    pub fn decode(mut self, on: bool) -> Self {
        self.draft.carry.body_rules.decode = on;
        self
    }

    /// Sets the maximum response body size in bytes, overriding the session setting.
    pub fn max_body_size(mut self, bytes: usize) -> Self {
        self.draft.carry.body_rules.max_body_size = bytes;
        self
    }

    /// Returns the response as soon as the headers arrive and leaves the body to stream.
    pub fn stream(mut self) -> Self {
        self.draft.carry.stream_response = true;
        self
    }

    /// Compresses the request body with the given encoding before sending.
    pub fn compress_body(mut self, encoding: Encoding) -> Self {
        self.draft.compress = Some(encoding);
        self
    }

    /// Appends `params` to the URL query string.
    pub fn query<I, K, V>(mut self, params: I) -> Self
    where
        I: IntoIterator,
        I::Item: Borrow<(K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for pair in params {
            self.draft.query_params.push(param_pair(pair));
        }
        self
    }

    /// Appends a header and records an error for `send` when the name or value is invalid.
    pub fn header(
        mut self,
        name: impl TryInto<HeaderName>,
        value: impl TryInto<HeaderValue>,
    ) -> Self {
        if let Err(err) = self.draft.headers.append(name, value) {
            self.fail(err);
        }
        self
    }

    fn put(&mut self, name: impl TryInto<HeaderName>, value: impl TryInto<HeaderValue>) {
        if let Err(err) = self.draft.headers.set(name, value) {
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
            if let Err(err) = self.draft.headers.append(name, value) {
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
        if let Err(err) = self.draft.headers.append_anchored(anchor, name, value) {
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
        self.put(
            "authorization",
            crate::util::basic_credentials(username, password),
        );
        self
    }

    /// Sets the retry policy for this request, overriding the session default.
    pub fn retry(mut self, retry: impl Into<Retry>) -> Self {
        self.draft.carry.retry_policy = retry.into();
        self
    }

    /// Sets the redirect policy for this request, overriding the session policy.
    pub fn redirect(mut self, policy: impl Into<Redirect>) -> Self {
        self.draft.carry.redirect = Some(policy.into());
        self
    }

    /// Answers an HTTP Digest challenge with the given credentials.
    pub fn digest_auth(mut self, username: &str, password: &str) -> Self {
        self.draft.carry.digest_auth = Some(DigestAuth::new(username, password));
        self
    }

    /// Sets a multipart form as the streamed body and sets the matching `content-type`.
    #[cfg(feature = "multipart")]
    pub fn multipart(mut self, form: Form) -> Self {
        self.put("content-type", form.content_type());
        self.draft.body = form.into_stream_body();
        self
    }

    /// Sends this request through the given proxy, and fails at send when the proxy is a pool.
    pub fn proxy(mut self, config: impl Into<Proxy>) -> Self {
        let config = config.into();
        if config.pool.is_some() {
            self.fail(Error::config(
                "a request takes one proxy; set a proxy list on the session",
            ));
        }
        self.draft.carry.proxy = Some(config);
        self
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

fn collect_pairs<I, K, V>(params: I) -> Vec<(String, String)>
where
    I: IntoIterator,
    I::Item: Borrow<(K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    params.into_iter().map(param_pair).collect()
}

fn param_pair<K: AsRef<str>, V: AsRef<str>>(pair: impl Borrow<(K, V)>) -> (String, String) {
    let (key, value) = pair.borrow();
    (key.as_ref().to_owned(), value.as_ref().to_owned())
}
