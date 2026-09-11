use std::time::Duration;

use http::{HeaderName, HeaderValue, Method, Uri};

use crate::core::body::Body;
use crate::core::digest::DigestAuth;
use crate::core::headers::HeaderList;
use crate::core::retry::RetryPolicy;
use crate::profile::Preset;

#[non_exhaustive]
pub struct Request {
    pub method: Method,
    pub url: Uri,
    pub headers: HeaderList,
    pub body: Body,
    pub timeout: Option<Duration>,
    pub retry_policy: Option<RetryPolicy>,
    pub digest_auth: Option<DigestAuth>,
    pub allow_non_idempotent_retry: bool,
    pub stream_response: bool,
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

    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
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
        self.stream_response = true;
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
}
