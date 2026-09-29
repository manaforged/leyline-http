use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;
use std::io;

use http::StatusCode;
use url::Url;

use crate::core::retry::GATEWAY_STATUSES;
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;
#[cfg(feature = "http3")]
use crate::quic::H3SendError;
use crate::tls::TlsError;

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

    pub fn is_timeout(&self) -> bool {
        self.inner.kind == Kind::Timeout
            || self
                .io_cause()
                .is_some_and(|e| e.kind() == io::ErrorKind::TimedOut)
    }

    pub fn is_connect(&self) -> bool {
        if self.inner.kind == Kind::Connect {
            return true;
        }
        if matches!(
            self.tls(),
            Some(
                TlsError::TcpConnect(_)
                    | TlsError::Dns(_)
                    | TlsError::Handshake(_)
                    | TlsError::HandshakeIo(_)
                    | TlsError::Rejected(_)
            )
        ) {
            return true;
        }
        self.io().is_some_and(|e| {
            matches!(
                e.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::AddrNotAvailable
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::NetworkUnreachable
            )
        })
    }

    pub fn is_status(&self) -> bool {
        self.inner.kind == Kind::Status
    }

    pub fn is_retryable(&self) -> bool {
        self.is_timeout()
            || self.is_connect()
            || self.is_connection_closed()
            || self.is_proxy_transient()
    }

    fn is_proxy_transient(&self) -> bool {
        let Some(TlsError::Proxy { status, source, .. }) = self.tls() else {
            return false;
        };
        source.is_some() || status.is_some_and(|code| GATEWAY_STATUSES.contains(&code))
    }

    pub(crate) fn is_connection_closed(&self) -> bool {
        if self.io().is_some_and(|e| {
            matches!(
                e.kind(),
                io::ErrorKind::UnexpectedEof
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::BrokenPipe
            )
        }) {
            return true;
        }
        if matches!(
            self.h2(),
            Some(
                H2Error::Io(_)
                    | H2Error::Connection {
                        code: ErrorCode::NoError,
                        ..
                    }
                    | H2Error::Stream {
                        code: ErrorCode::RefusedStream,
                        ..
                    }
            )
        ) {
            return true;
        }
        matches!(
            self.tls(),
            Some(TlsError::Handshake(_) | TlsError::HandshakeIo(_) | TlsError::Rejected(_))
        )
    }

    fn io_cause(&self) -> Option<&io::Error> {
        self.io()
            .or_else(|| self.tls().and_then(TlsError::io_source))
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
        out.finish()
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner.source.as_ref().map(|e| &**e as &dyn StdError)
    }
}

impl Error {
    pub(crate) fn from_url_parse(e: url::ParseError) -> Self {
        Error::new(Kind::Url).with_source(e)
    }

    pub(crate) fn from_json(e: serde_json::Error) -> Self {
        Error::new(Kind::Json).with_source(e)
    }

    pub(crate) fn from_request_body(e: io::Error) -> Self {
        Error::new(Kind::Body)
            .with_message("request body stream failed")
            .with_source(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::new(Kind::Io).with_source(e)
    }
}

impl From<TlsError> for Error {
    fn from(e: TlsError) -> Self {
        let kind = match &e {
            TlsError::Dns(_) | TlsError::TcpConnect(_) => Kind::Connect,
            TlsError::Proxy { .. } => Kind::Proxy,
            TlsError::SslConfig(_) | TlsError::Profile(_) | TlsError::TrustStore(_) => Kind::Config,
            _ => Kind::Tls,
        };
        Error::new(kind).with_source(e)
    }
}

impl From<H2Error> for Error {
    fn from(e: H2Error) -> Self {
        if let Some(limit) = e.body_limit() {
            return limit.error();
        }
        match e {
            H2Error::RequestBody(io) => Error::from_request_body(io),
            other => Error::new(Kind::Http2).with_source(other),
        }
    }
}

#[cfg(feature = "http3")]
impl From<H3SendError> for Error {
    fn from(e: H3SendError) -> Self {
        match e {
            H3SendError::BodyLimit(limit) => limit.error(),
            H3SendError::RequestBody(io) => Error::from_request_body(io),
            other => Error::new(Kind::Http3).with_message(other.message().into_owned()),
        }
    }
}

impl From<http::Error> for Error {
    fn from(e: http::Error) -> Self {
        Error::new(Kind::Request).with_source(e)
    }
}

impl From<http::header::InvalidHeaderName> for Error {
    fn from(e: http::header::InvalidHeaderName) -> Self {
        Error::new(Kind::Request).with_source(e)
    }
}

impl From<http::header::InvalidHeaderValue> for Error {
    fn from(e: http::header::InvalidHeaderValue) -> Self {
        Error::new(Kind::Request).with_source(e)
    }
}

impl From<http::uri::InvalidUri> for Error {
    fn from(e: http::uri::InvalidUri) -> Self {
        Error::new(Kind::Url).with_source(e)
    }
}

#[cfg(test)]
#[path = "error/tests.rs"]
mod tests;
