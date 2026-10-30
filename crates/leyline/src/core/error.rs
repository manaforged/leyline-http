use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;
use std::io;

use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use url::Url;

use crate::h2::H2Error;
use crate::tls::TlsError;

mod category;
mod classify;
mod from;
mod marker;
mod profile;

pub(crate) use profile::ProfileChanged;

pub use category::Kind;

/// A result whose error type is the crate's `Error`.
pub type Result<T> = std::result::Result<T, Error>;

type Source = Box<dyn StdError + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Cause {
    Request,
    Redirect,
    Status,
    Body,
    Decode,
    Timeout,
    Connect,
    Tls,
    Http2,
    #[cfg(feature = "http3")]
    Http3,
    Proxy,
    Io,
    Config,
    Url,
    Json,
}

struct Inner {
    kind: Cause,
    source: Option<Source>,
    url: Option<Url>,
    status: Option<StatusCode>,
    message: Option<Cow<'static, str>>,
    alpn: Option<String>,
    body: Option<Bytes>,
    headers: Option<HeaderMap>,
    attempts: u32,
    proxy: Option<String>,
    policy_wait: Option<std::time::Duration>,
    retries_exhausted: bool,
    after_response: bool,
}

#[doc = include_str!("../../docs/error.md")]
pub struct Error {
    inner: Box<Inner>,
}

impl Error {
    pub(crate) fn new(kind: Cause) -> Self {
        Self {
            inner: Box::new(Inner {
                kind,
                source: None,
                url: None,
                status: None,
                message: None,
                alpn: None,
                body: None,
                headers: None,
                attempts: 0,
                proxy: None,
                policy_wait: None,
                retries_exhausted: false,
                after_response: false,
            }),
        }
    }

    pub(crate) fn with_source(mut self, source: impl Into<Source>) -> Self {
        self.inner.source = Some(source.into());
        self
    }

    pub(crate) fn with_url(mut self, url: Url) -> Self {
        self.inner.url = Some(url);
        self
    }

    pub(crate) fn with_status(mut self, status: StatusCode) -> Self {
        self.inner.status = Some(status);
        self
    }

    pub(crate) fn with_message(mut self, message: impl Into<Cow<'static, str>>) -> Self {
        self.inner.message = Some(message.into());
        self
    }

    pub(crate) fn with_alpn(mut self, negotiated: impl Into<String>) -> Self {
        self.inner.alpn = Some(negotiated.into());
        self
    }

    pub(crate) fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.inner.body = Some(body.into());
        self
    }

    pub(crate) fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.inner.headers = Some(headers);
        self
    }

    pub(crate) fn without_content_coding(mut self) -> Self {
        if let Some(headers) = self.inner.headers.as_mut() {
            headers.remove(http::header::CONTENT_ENCODING);
        }
        self.without_content_length()
    }

    pub(crate) fn without_content_length(mut self) -> Self {
        if let Some(headers) = self.inner.headers.as_mut() {
            headers.remove(http::header::CONTENT_LENGTH);
        }
        self
    }

    pub(crate) fn after_response(mut self) -> Self {
        self.inner.after_response = true;
        self
    }

    pub(crate) fn follows_response(&self) -> bool {
        self.inner.after_response
    }

    /// Returns the response headers when the error came after a response.
    pub fn headers(&self) -> Option<&HeaderMap> {
        self.inner.headers.as_ref()
    }

    /// Returns how many attempts were made before the error.
    pub fn attempts(&self) -> u32 {
        self.inner.attempts
    }

    pub(crate) fn set_attempts(&mut self, attempts: u32) {
        self.inner.attempts = attempts.max(1);
    }

    /// Returns the redacted URL of the proxy the last attempt used.
    pub fn proxy(&self) -> Option<&str> {
        self.inner.proxy.as_deref()
    }

    pub(crate) fn set_proxy(&mut self, proxy: Option<&str>) {
        self.inner.proxy = proxy.map(crate::util::redact);
    }

    pub(crate) fn with_trail(mut self, attempts: u32, proxy: Option<String>) -> Self {
        self.inner.attempts = attempts;
        self.inner.proxy = proxy;
        self
    }

    /// Returns true when the retry policy ran out of attempts.
    pub fn retries_exhausted(&self) -> bool {
        self.inner.retries_exhausted
    }

    pub(crate) fn set_retry_outcome(
        &mut self,
        policy_wait: Option<std::time::Duration>,
        retries_exhausted: bool,
    ) {
        self.inner.policy_wait = policy_wait;
        self.inner.retries_exhausted = retries_exhausted;
    }

    /// Returns the error response body when one was read.
    pub fn body(&self) -> Option<&[u8]> {
        self.inner.body.as_deref()
    }

    /// Returns the server's requested wait for a status error, from the retry policy or the `Retry-After` header.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        self.status()?;
        if self.inner.policy_wait.is_some() {
            return self.inner.policy_wait;
        }
        self.headers()?
            .get(http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(crate::core::retry::parse_retry_after)
    }

    /// Returns the first `Error` in the source chain of `error`, starting with `error` itself.
    pub fn find<'a>(error: &'a (dyn StdError + 'static)) -> Option<&'a Error> {
        let mut current = Some(error);
        while let Some(err) = current {
            if let Some(found) = err.downcast_ref::<Error>() {
                return Some(found);
            }
            current = err.source();
        }
        None
    }

    pub(crate) fn into_io(self) -> io::Error {
        match self.body_limit() {
            Some(limit) => limit.into_io(),
            None => io::Error::other(self),
        }
    }

    pub(crate) fn cause(&self) -> Cause {
        self.inner.kind
    }

    pub(crate) fn message(&self) -> Option<&str> {
        self.inner.message.as_deref()
    }

    /// Returns the HTTP status for a status error.
    pub fn status(&self) -> Option<StatusCode> {
        self.inner.status
    }

    /// Returns the request URL when it is known.
    pub fn url(&self) -> Option<&Url> {
        self.inner.url.as_ref()
    }

    pub(crate) fn io(&self) -> Option<&io::Error> {
        self.source_as()
    }

    pub(crate) fn tls(&self) -> Option<&TlsError> {
        self.source_as()
    }

    pub(crate) fn h2(&self) -> Option<&H2Error> {
        self.source_as()
    }

    pub(crate) fn alpn(&self) -> Option<&str> {
        self.inner.alpn.as_deref()
    }

    fn source_as<T: StdError + 'static>(&self) -> Option<&T> {
        self.inner.source.as_ref()?.downcast_ref::<T>()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind().as_str())?;
        if let Some(status) = self.inner.status {
            write!(f, " {}", status.as_u16())?;
        }
        if let Some(message) = &self.inner.message {
            write!(f, ": {message}")?;
        }
        if let Some(url) = &self.inner.url {
            write!(f, " for {}", crate::util::redact(url.as_str()))?;
        }
        Ok(())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("leyline::Error");
        out.field("kind", &self.kind());
        if let Some(status) = self.inner.status {
            out.field("status", &status.as_u16());
        }
        if let Some(url) = &self.inner.url {
            out.field("url", &crate::util::redact(url.as_str()));
        }
        if let Some(message) = &self.inner.message {
            out.field("message", message);
        }
        if let Some(source) = &self.inner.source {
            out.field("source", source);
        }
        if let Some(body) = &self.inner.body {
            out.field("body_len", &body.len());
        }
        if let Some(headers) = &self.inner.headers {
            out.field("header_count", &headers.len());
        }
        if self.inner.attempts > 0 {
            out.field("attempts", &self.inner.attempts);
        }
        if let Some(proxy) = &self.inner.proxy {
            out.field("proxy", proxy);
        }
        if self.inner.retries_exhausted {
            out.field("retries_exhausted", &true);
        }
        out.finish()
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner.source.as_ref().map(|e| &**e as &dyn StdError)
    }
}

#[cfg(test)]
#[path = "error/tests.rs"]
mod tests;
