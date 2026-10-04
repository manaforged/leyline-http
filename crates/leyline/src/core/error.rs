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

pub use category::ErrorCategory;

pub type Result<T> = std::result::Result<T, Error>;

type Source = Box<dyn StdError + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    Request,
    Redirect,
    Status,
    Body,
    Decode,
    Timeout,
    Connect,
    Tls,
    Http2,
    Http3,
    Proxy,
    Io,
    Config,
    Url,
    Json,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Request => "request",
            Kind::Redirect => "redirect",
            Kind::Status => "status",
            Kind::Body => "body",
            Kind::Decode => "decode",
            Kind::Timeout => "timeout",
            Kind::Connect => "connect",
            Kind::Tls => "tls",
            Kind::Http2 => "http2",
            Kind::Http3 => "http3",
            Kind::Proxy => "proxy",
            Kind::Io => "io",
            Kind::Config => "config",
            Kind::Url => "url",
            Kind::Json => "json",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

struct Inner {
    kind: Kind,
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

pub struct Error {
    inner: Box<Inner>,
}

impl Error {
    pub(crate) fn new(kind: Kind) -> Self {
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

    pub(crate) fn after_response(mut self) -> Self {
        self.inner.after_response = true;
        self
    }

    pub(crate) fn follows_response(&self) -> bool {
        self.inner.after_response
    }

    pub fn headers(&self) -> Option<&HeaderMap> {
        self.inner.headers.as_ref()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers()?.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn attempts(&self) -> u32 {
        self.inner.attempts
    }

    pub(crate) fn set_attempts(&mut self, attempts: u32) {
        self.inner.attempts = attempts.max(1);
    }

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

    pub fn body(&self) -> Option<&[u8]> {
        self.inner.body.as_deref()
    }

    pub fn body_text(&self) -> Option<Cow<'_, str>> {
        self.body().map(String::from_utf8_lossy)
    }

    pub fn retry_after(&self) -> Option<std::time::Duration> {
        self.status()?;
        if self.inner.policy_wait.is_some() {
            return self.inner.policy_wait;
        }
        self.header(http::header::RETRY_AFTER.as_str())
            .and_then(crate::core::retry::parse_retry_after)
    }

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

    pub fn kind(&self) -> Kind {
        self.inner.kind
    }

    pub(crate) fn message(&self) -> Option<&str> {
        self.inner.message.as_deref()
    }

    pub fn status(&self) -> Option<StatusCode> {
        self.inner.status
    }

    pub fn url(&self) -> Option<&Url> {
        self.inner.url.as_ref()
    }

    pub fn io(&self) -> Option<&io::Error> {
        self.source_as()
    }

    pub fn tls(&self) -> Option<&TlsError> {
        self.source_as()
    }

    pub fn h2(&self) -> Option<&H2Error> {
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
        f.write_str(self.inner.kind.as_str())?;
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
        out.field("kind", &self.inner.kind);
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
