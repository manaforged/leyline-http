#![forbid(unsafe_code)]
mod compress;
mod context;
mod debug;
mod download;
mod encode;
mod events;
pub use events::{Event, Events};
mod interop;
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
use crate::core::session::{Protocol, Session};
use crate::deadline::Deadline;
use crate::deadline::Timeout;
use crate::error::{Error, Kind};
use crate::wire::Body;
use crate::wire::ResponseMode;

#[doc = include_str!("../../../docs/request.md")]
#[must_use = "builders are lazy: nothing happens until `.await` or `.send()`"]
pub struct RequestBuilder {
    session: Session,
    draft: Box<Draft>,
}

#[derive(Default)]
struct BuildExtras {
    query_params: Vec<(String, String)>,
    builder_error: Option<Error>,
    compress: Option<Encoding>,
    download: Option<Box<download::Download>>,
}

pub(crate) struct Draft {
    pub(crate) method: Method,
    pub(crate) url: Option<std::sync::Arc<url::Url>>,
    pub(crate) body: Body,
    pub(crate) headers: HeaderList,
    pub(crate) deadline: Deadline,
    pub(crate) response: ResponseMode,
    pub(crate) carry: Carry,
    build: Option<Box<BuildExtras>>,
}

#[derive(Clone)]
pub(crate) struct Carry {
    pub(crate) retrying: bool,
    pub(crate) preset: Option<RequestKind>,
    pub(crate) preset_user: bool,
    pub(crate) stream_response: bool,
    pub(crate) status_errors: bool,
    pub(crate) protocol: Option<Protocol>,
    pub(crate) body_rules: BodyRules,
    pub(crate) overrides: Option<Box<Overrides>>,
}

#[derive(Clone, Default)]
pub(crate) struct Overrides {
    pub(crate) timeouts: Option<Timeout>,
    pub(crate) retry_policy: Option<Retry>,
    pub(crate) digest_auth: Option<DigestAuth>,
    pub(crate) proxy: Option<Proxy>,
    pub(crate) first_hop: Option<Option<crate::net::ProxyUrl>>,
    pub(crate) header_order: Option<Vec<String>>,
    pub(crate) redirect: Option<Redirect>,
    pub(crate) initiator: Option<url::Url>,
    pub(crate) trusted_origin: Option<url::Url>,
    pub(crate) label: Option<String>,
}

impl Carry {
    pub(crate) fn overrides(&self) -> Option<&Overrides> {
        self.overrides.as_deref()
    }

    pub(crate) fn overrides_mut(&mut self) -> &mut Overrides {
        self.overrides.get_or_insert_with(Box::default)
    }

    pub(crate) fn timeouts(&self) -> Option<&Timeout> {
        self.overrides().and_then(|o| o.timeouts.as_ref())
    }

    pub(crate) fn proxy(&self) -> Option<&Proxy> {
        self.overrides().and_then(|o| o.proxy.as_ref())
    }

    pub(crate) fn header_order(&self) -> Option<&[String]> {
        self.overrides().and_then(|o| o.header_order.as_deref())
    }

    pub(crate) fn redirect(&self) -> Option<&Redirect> {
        self.overrides().and_then(|o| o.redirect.as_ref())
    }

    pub(crate) fn retry_policy<'a>(&'a self, session: &'a Session) -> &'a Retry {
        self.overrides()
            .and_then(|o| o.retry_policy.as_ref())
            .unwrap_or_else(|| session.default_retry())
    }

    pub(crate) fn take_retry_policy(&mut self, session: &Session) -> Retry {
        self.overrides
            .as_deref_mut()
            .and_then(|o| o.retry_policy.take())
            .unwrap_or_else(|| session.default_retry().clone())
    }
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
            deadline: Deadline::default(),
            response: ResponseMode::Buffered,
            carry: Carry {
                retrying: false,
                preset,
                preset_user: false,
                stream_response: false,
                status_errors: false,
                protocol: None,
                body_rules: session.body_rules(),
                overrides: None,
            },
            build: None,
        }
    }

    fn build_extras(&self) -> Option<&BuildExtras> {
        self.build.as_deref()
    }

    fn build_extras_mut(&mut self) -> &mut BuildExtras {
        self.build.get_or_insert_with(Box::default)
    }

    fn again(&self, body: Body, headers: HeaderList) -> Box<Draft> {
        Box::new(Draft {
            method: self.method.clone(),
            url: self.url.clone(),
            body,
            headers,
            deadline: self.deadline,
            response: self.response,
            carry: self.carry.clone(),
            build: None,
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
    pub(super) fn refresh_inferred_preset(&mut self, session: &Session) {
        if self.carry.preset_user || !session.impersonates() {
            return;
        }
        self.carry.preset = default_preset(session, &self.method);
        if !crate::wire::has_body_by_default(&self.method) {
            return;
        }
        let Some(ct) = self.headers.get("content-type") else {
            return;
        };
        let ct = String::from_utf8_lossy(ct.as_bytes()).to_ascii_lowercase();
        if ct.starts_with("application/json") {
            self.carry.preset = Some(RequestKind::Xhr);
        } else if ct.starts_with("application/x-www-form-urlencoded") {
            self.carry.preset = Some(RequestKind::Form);
        }
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
        if origin.is_some() {
            self.draft.carry.overrides_mut().trusted_origin = origin;
        }
        self
    }

    pub(crate) fn from_url(session: &Session, method: Method, url: impl IntoUrl) -> Self {
        match crate::core::into_url::sealed::Sealed::shared_url(url, session.base_url()) {
            Ok(url) => Self::new(session, method, Some(url)),
            Err(err) => {
                let mut builder = Self::new(session, method, None);
                builder.fail(err);
                builder
            }
        }
    }

    pub(crate) fn fail(&mut self, err: Error) {
        let build = self.draft.build_extras_mut();
        if build.builder_error.is_none() {
            build.builder_error = Some(err);
        }
    }

    /// Returns a copy of this request with every option, or `None` when the body is a stream or a setter recorded an error.
    pub fn try_clone(&self) -> Option<Self> {
        let draft = &self.draft;
        if draft
            .build_extras()
            .is_some_and(|b| b.builder_error.is_some())
        {
            return None;
        }
        let mut copy = draft.again(draft.body.try_clone()?, draft.headers.clone());
        if let Some(build) = draft.build_extras() {
            let copy_build = copy.build_extras_mut();
            copy_build.query_params.clone_from(&build.query_params);
            copy_build.compress = build.compress;
            copy_build.download.clone_from(&build.download);
        }
        Some(Self {
            session: self.session.clone(),
            draft: copy,
        })
    }

    /// Converts this request into an `http::Request` with its absolute URL, the headers set on it and its options in the extensions, or returns the error a setter recorded; `Session::request_from` converts it back.
    pub fn build(mut self) -> crate::error::Result<http::Request<Body>> {
        self.prepare_in_place()?;
        self.draft.into_http(self.session.default_retry())
    }

    fn infer_from_content_type(&mut self) {
        self.draft.refresh_inferred_preset(&self.session);
    }

    /// Sets the timeouts for this request, overriding the session timeouts.
    pub fn timeout(mut self, config: impl Into<Timeout>) -> Self {
        self.draft.carry.overrides_mut().timeouts = Some(config.into());
        self
    }

    /// Sets the HTTP version policy for this request and the redirects it follows, overriding the session policy and still using the session's connection pool.
    pub fn protocol(mut self, protocol: Protocol) -> Self {
        self.draft.carry.protocol = Some(protocol);
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
        self.draft.carry.overrides_mut().header_order =
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
                self.draft.build_extras_mut().builder_error =
                    Some(Error::new(Kind::Json).with_source(e));
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

    /// Sets the maximum response body size in bytes, overriding the session setting; `None` removes the cap.
    pub fn max_body_size(mut self, bytes: impl Into<Option<usize>>) -> Self {
        self.draft.carry.body_rules.max_body_size =
            bytes.into().unwrap_or(crate::wire::NO_SIZE_CAP);
        self
    }

    /// Returns the response as soon as the headers arrive and leaves the body to stream; without it `send` reads the whole body into memory, up to `max_body_size`, before it returns.
    pub fn stream(mut self) -> Self {
        self.draft.carry.stream_response = true;
        self
    }

    /// Compresses the request body with the given encoding before sending.
    pub fn compress_body(mut self, encoding: Encoding) -> Self {
        self.draft.build_extras_mut().compress = Some(encoding);
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
        let query_params = &mut self.draft.build_extras_mut().query_params;
        for pair in params {
            query_params.push(param_pair(pair));
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
    pub fn bearer_auth(mut self, token: impl AsRef<str>) -> Self {
        self.put(
            http::header::AUTHORIZATION,
            crate::util::bearer_credentials(token.as_ref()),
        );
        self
    }

    /// Sets the `authorization` header to HTTP Basic credentials.
    pub fn basic_auth(mut self, username: impl AsRef<str>, password: impl AsRef<str>) -> Self {
        self.put(
            http::header::AUTHORIZATION,
            crate::util::basic_credentials(username.as_ref(), password.as_ref()),
        );
        self
    }

    /// Sets the retry policy for this request, overriding the session default.
    pub fn retry(mut self, retry: impl Into<Retry>) -> Self {
        self.draft.carry.overrides_mut().retry_policy = Some(retry.into());
        self
    }

    /// Sets the redirect policy for this request, overriding the session policy.
    pub fn redirect(mut self, policy: impl Into<Redirect>) -> Self {
        self.draft.carry.overrides_mut().redirect = Some(policy.into());
        self
    }

    /// Answers an HTTP Digest challenge with the given credentials.
    pub fn digest_auth(mut self, username: impl AsRef<str>, password: impl AsRef<str>) -> Self {
        self.draft.carry.overrides_mut().digest_auth =
            Some(DigestAuth::new(username.as_ref(), password.as_ref()));
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
        if let Err(err) = config.check_headers() {
            self.fail(err);
        }
        if config.is_pool() {
            self.fail(Error::config(
                "a request takes one proxy; set a proxy list on the session",
            ));
        }
        self.draft.carry.overrides_mut().proxy = Some(config);
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
