#![forbid(unsafe_code)]
mod compress;
mod encode;
mod send;

pub use compress::ContentEncoding;

use http::{HeaderName, HeaderValue, Method};

use crate::profile::{HeaderAnchor, Preset};

use crate::core::body::Body;
use crate::core::config::TimeoutConfig;
use crate::core::digest::DigestAuth;
use crate::core::error::{Error, Kind};
use crate::core::headers::HeaderList;
#[cfg(feature = "multipart")]
use crate::core::multipart::Form;
use crate::core::retry::RetryPolicy;
use crate::core::session::Session;

pub trait IntoParamPair {
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
pub struct RequestBuilder {
    pub(super) session: Session,
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) preset: Option<Preset>,
    pub(super) body: Body,
    pub(super) headers: HeaderList,
    pub(super) query_params: Vec<(String, String)>,
    pub(super) timeouts: Option<TimeoutConfig>,
    pub(super) builder_error: Option<Error>,
    pub(super) stream_response: bool,
    pub(super) compress: Option<ContentEncoding>,
    pub(super) retry_policy: RetryPolicy,
    pub(super) digest_auth: Option<DigestAuth>,
    pub(super) proxy: Option<String>,
    pub(super) header_order: Option<Vec<String>>,
    pub(super) preset_user: bool,
}

fn default_preset(session: &Session, method: &Method) -> Option<Preset> {
    session.browser()?;
    match *method {
        Method::GET | Method::HEAD => Some(Preset::Navigate),
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
        }
    }

    pub(crate) fn invalid(session: &Session, method: Method) -> Self {
        let mut builder = Self::new(session, method, "");
        builder.builder_error = Some(Error::new(Kind::Request).with_message("invalid request URL"));
        builder
    }

    fn fail(&mut self, err: Error) {
        if self.builder_error.is_none() {
            self.builder_error = Some(err);
        }
    }

    fn infer_from_content_type(&mut self) {
        if self.preset_user || self.session.browser().is_none() {
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
            self.preset = Some(Preset::Xhr);
        } else if ct.starts_with("application/x-www-form-urlencoded") {
            self.preset = Some(Preset::Form);
        }
    }

    pub fn timeout(mut self, config: impl Into<TimeoutConfig>) -> Self {
        self.timeouts = Some(config.into());
        self
    }

    pub fn preset(mut self, preset: Preset) -> Self {
        self.preset = Some(preset);
        self.preset_user = true;
        self
    }

    pub fn header_order(mut self, order: &[&str]) -> Self {
        self.header_order = Some(order.iter().map(|s| (*s).to_string()).collect());
        self
    }

    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    pub fn json(mut self, value: &impl serde::Serialize) -> Self {
        match serde_json::to_vec(value) {
            Ok(bytes) => {
                self.put("content-type", "application/json");
                self.body = Body::from(bytes);
            }
            Err(e) => {
                self.builder_error = Some(Error::new(Kind::Json).with_source(e));
            }
        }
        self
    }

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

    pub fn stream(mut self) -> Self {
        self.stream_response = true;
        self
    }

    pub fn compress(mut self, encoding: ContentEncoding) -> Self {
        self.compress = Some(encoding);
        self
    }

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

    pub fn headers<I, P>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        for pair in headers {
            let (k, v) = pair.into_param_pair();
            if let Err(err) = self.headers.append(k, v) {
                self.fail(err);
                return self;
            }
        }
        self
    }

    pub fn anchored(
        mut self,
        anchor: HeaderAnchor,
        name: impl TryInto<HeaderName>,
        value: impl TryInto<HeaderValue>,
    ) -> Self {
        if let Err(err) = self.headers.append_anchored(anchor, name, value) {
            self.fail(err);
        }
        self
    }

    pub fn bearer_auth(mut self, token: &str) -> Self {
        self.put("authorization", format!("Bearer {token}"));
        self
    }

    pub fn basic_auth(mut self, username: &str, password: &str) -> Self {
        let encoded = crate::util::base64_encode(&format!("{username}:{password}"));
        self.put("authorization", format!("Basic {encoded}"));
        self
    }

    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    pub fn digest_auth(mut self, auth: DigestAuth) -> Self {
        self.digest_auth = Some(auth);
        self
    }

    #[cfg(feature = "multipart")]
    pub fn multipart(mut self, form: Form) -> Self {
        self.put("content-type", form.content_type());
        self.body = form.into_stream_body();
        self
    }

    pub fn proxy(mut self, proxy_url: &str) -> Self {
        self.proxy = Some(proxy_url.to_string());
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
