//! Error types for Leyline.

use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;
use std::io;

use http::StatusCode;
use http::Uri;

use crate::h2::H2Error;
use crate::h2::error::ErrorCode;
use crate::tls::TlsError;

/// Leyline result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Boxed source error carried by [`Error`].
type Source = Box<dyn StdError + Send + Sync>;

/// The layer that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// A client or request could not be built from the given inputs.
    Builder,
    /// The request could not be sent or the exchange failed at the HTTP layer.
    Request,
    /// Redirect handling failed.
    Redirect,
    /// The response carried a 4xx or 5xx status.
    Status,
    /// Request or response body streaming or buffering failed.
    Body,
    /// Response decompression or decoding failed.
    Decode,
    /// The operation timed out.
    Timeout,
    /// The connection could not be established.
    Connect,
    /// TLS configuration, handshake, or verification failed.
    Tls,
    /// HTTP/2 protocol or transport error.
    Http2,
    /// HTTP/3 or QUIC protocol or transport error.
    Http3,
    /// Proxy configuration or tunnel failure.
    Proxy,
    /// Low-level IO error.
    Io,
    /// Invalid configuration.
    Config,
    /// URL parsing failed.
    Url,
    /// JSON serialization or deserialization failed.
    Json,
}

impl Kind {
    /// A stable lowercase token for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Builder => "builder",
            Kind::Request => "http",
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

/// Everything an [`Error`] carries, kept behind one allocation.
struct Inner {
    kind: Kind,
    source: Option<Source>,
    url: Option<Uri>,
    status: Option<StatusCode>,
    message: Option<Cow<'static, str>>,
    body: Option<Vec<u8>>,
    alpn: Option<String>,
}

/// The error returned by every fallible Leyline operation.
pub struct Error {
    inner: Box<Inner>,
}

impl Error {
    /// A new error of `kind` with no context attached.
    pub fn new(kind: Kind) -> Self {
        Self {
            inner: Box::new(Inner {
                kind,
                source: None,
                url: None,
                status: None,
                message: None,
                body: None,
                alpn: None,
            }),
        }
    }

    /// Attach the underlying error.
    pub fn with_source(mut self, source: impl Into<Source>) -> Self {
        self.inner.source = Some(source.into());
        self
    }

    /// Attach the request URL.
    pub fn with_url(mut self, url: Uri) -> Self {
        self.inner.url = Some(url);
        self
    }

    /// Attach the response status.
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.inner.status = Some(status);
        self
    }

    /// Attach a human-readable message.
    pub fn with_message(mut self, message: impl Into<Cow<'static, str>>) -> Self {
        self.inner.message = Some(message.into());
        self
    }

    /// Attach a response body prefix, as captured by [`Response::error_for_status`](crate::Response::error_for_status).
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.inner.body = Some(body);
        self
    }

    /// Attach the ALPN protocol the peer negotiated when it was not the required one.
    pub fn with_alpn(mut self, negotiated: impl Into<String>) -> Self {
        self.inner.alpn = Some(negotiated.into());
        self
    }

    /// The layer that failed.
    pub fn kind(&self) -> Kind {
        self.inner.kind
    }

    /// The attached message, when the error carries one.
    pub fn message(&self) -> Option<&str> {
        self.inner.message.as_deref()
    }

    /// The captured response body prefix, when the error carries one.
    pub fn body_prefix(&self) -> Option<&[u8]> {
        self.inner.body.as_deref()
    }

    /// The HTTP status code, when this error carries one.
    pub fn status(&self) -> Option<StatusCode> {
        self.inner.status
    }

    /// The request URL, when this error carries one.
    pub fn url(&self) -> Option<&Uri> {
        self.inner.url.as_ref()
    }

    /// The same error without its URL, for callers that must not leak it.
    pub fn without_url(mut self) -> Self {
        self.inner.url = None;
        self
    }

    /// True if this error is a timeout.
    pub fn is_timeout(&self) -> bool {
        if self.inner.kind == Kind::Timeout {
            return true;
        }
        if self
            .io()
            .is_some_and(|e| e.kind() == io::ErrorKind::TimedOut)
        {
            return true;
        }
        self.tls()
            .and_then(io_kind)
            .is_some_and(|k| k == io::ErrorKind::TimedOut)
    }

    /// True if this is a connection-establishment failure (TCP, DNS, TLS handshake, or proxy tunnel), not body or file I/O.
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
                    | TlsError::SslConnect(_)
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

    /// True if this error carries an HTTP status (from [`Response::error_for_status`](crate::Response::error_for_status)).
    pub fn is_status(&self) -> bool {
        self.inner.kind == Kind::Status
    }

    /// True if redirect handling failed.
    pub fn is_redirect(&self) -> bool {
        self.inner.kind == Kind::Redirect
    }

    /// True if a request or response body failed.
    pub fn is_body(&self) -> bool {
        self.inner.kind == Kind::Body
    }

    /// True if response decoding or decompression failed.
    pub fn is_decode(&self) -> bool {
        self.inner.kind == Kind::Decode
    }

    /// True if the connection went away (peer closed, graceful GOAWAY, or a transport-level EOF or reset) and the request can be retried on a fresh connection.
    pub fn is_connection_closed(&self) -> bool {
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
            Some(TlsError::Handshake(_) | TlsError::HandshakeIo(_) | TlsError::SslConnect(_))
        )
    }

    /// The wrapped IO error, when the source is one.
    pub fn io(&self) -> Option<&io::Error> {
        self.source_as()
    }

    /// The wrapped TLS error, when the source is one.
    pub fn tls(&self) -> Option<&TlsError> {
        self.source_as()
    }

    /// The wrapped HTTP/2 error, when the source is one.
    pub fn h2(&self) -> Option<&H2Error> {
        self.source_as()
    }

    /// The ALPN protocol the peer negotiated, when this error is an ALPN mismatch.
    pub(crate) fn alpn(&self) -> Option<&str> {
        self.inner.alpn.as_deref()
    }

    fn source_as<T: StdError + 'static>(&self) -> Option<&T> {
        self.inner.source.as_ref()?.downcast_ref::<T>()
    }

    fn detail(&self) -> Option<String> {
        if let Some(message) = &self.inner.message {
            return Some(message.to_string());
        }
        self.inner.source.as_ref().map(ToString::to_string)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.inner.kind.as_str())?;
        if let Some(status) = self.inner.status {
            write!(f, " {}", status.as_u16())?;
        }
        if let Some(detail) = self.detail() {
            write!(f, ": {detail}")?;
        }
        if let Some(url) = &self.inner.url {
            write!(f, " for {}", redact(url))?;
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
            out.field("url", &redact(url));
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

/// A URL with any userinfo replaced, so credentials never reach a log.
fn redact(url: &Uri) -> String {
    let Some(authority) = url.authority() else {
        return url.to_string();
    };
    let text = authority.as_str();
    let Some(at) = text.rfind('@') else {
        return url.to_string();
    };
    let scheme = url
        .scheme_str()
        .map(|s| format!("{s}://"))
        .unwrap_or_default();
    let path = url
        .path_and_query()
        .map(ToString::to_string)
        .unwrap_or_default();
    let host = text.get(at + 1..).unwrap_or_default();
    format!("{scheme}***@{host}{path}")
}

fn io_kind(err: &TlsError) -> Option<io::ErrorKind> {
    match err {
        TlsError::TcpConnect(e) | TlsError::Dns(e) | TlsError::HandshakeIo(e) => Some(e.kind()),
        _ => None,
    }
}

impl From<url::ParseError> for Error {
    fn from(e: url::ParseError) -> Self {
        Error::new(Kind::Url).with_source(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::new(Kind::Json).with_source(e)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::new(Kind::Io).with_source(e)
    }
}

impl From<TlsError> for Error {
    fn from(e: TlsError) -> Self {
        Error::new(Kind::Tls).with_source(e)
    }
}

impl From<H2Error> for Error {
    fn from(e: H2Error) -> Self {
        Error::new(Kind::Http2).with_source(e)
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
