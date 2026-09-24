use std::time::Duration;

use http::{HeaderName, HeaderValue, Method, Uri};

use crate::core::body::Body;
use crate::core::config::TimeoutConfig;
use crate::core::digest::DigestAuth;
use crate::core::headers::HeaderList;
use crate::core::retry::RetryPolicy;
use crate::profile::Preset;

pub struct Request {
    pub(crate) method: Method,
    pub(crate) url: Uri,
    pub(crate) headers: HeaderList,
    pub(crate) body: Body,
    pub(crate) timeout: Option<Duration>,
    pub(crate) timeouts: Option<TimeoutConfig>,
    pub(crate) retry: Option<RetryPolicy>,
    pub(crate) digest_auth: Option<DigestAuth>,
    pub(crate) allow_non_idempotent_retry: bool,
    pub(crate) stream: bool,
    pub(crate) preset: Option<Preset>,
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
    pub fn new(method: Method, url: impl TryInto<Uri>) -> Self {
        Self {
            method,
            url: url.try_into().unwrap_or_default(),
            headers: HeaderList::new(),
            body: Body::default(),
            timeout: None,
            timeouts: None,
            retry: None,
            digest_auth: None,
            allow_non_idempotent_retry: false,
            stream: false,
            preset: None,
        }
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn url(&self) -> &Uri {
        &self.url
    }

    pub fn headers(&self) -> &HeaderList {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderList {
        &mut self.headers
    }

    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry = Some(policy);
        self
    }

    pub fn allow_non_idempotent_retry(mut self, v: bool) -> Self {
        self.allow_non_idempotent_retry = v;
        self
    }

    pub fn digest_auth(mut self, auth: DigestAuth) -> Self {
        self.digest_auth = Some(auth);
        self
    }

    pub fn stream(mut self) -> Self {
        self.stream = true;
        self
    }

    pub fn header(
        mut self,
        name: impl TryInto<HeaderName>,
        value: impl TryInto<HeaderValue>,
    ) -> Self {
        drop(self.headers.set(name, value));
        self
    }

    pub fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    pub fn preset(mut self, preset: Preset) -> Self {
        self.preset = Some(preset);
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn timeouts(mut self, timeouts: TimeoutConfig) -> Self {
        self.timeout = Some(timeouts.total);
        self.timeouts = Some(timeouts);
        self
    }
}
