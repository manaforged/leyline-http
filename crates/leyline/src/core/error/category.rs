use http::StatusCode;

use super::{Cause, Error};
use crate::tls::TlsError;

const LABELS: [&str; 18] = [
    "timeout",
    "dns",
    "connect",
    "tls",
    "proxy",
    "proxy_target",
    "status",
    "body_limit",
    "body",
    "decode",
    "json",
    "http2",
    "http3",
    "redirect",
    "url",
    "config",
    "request",
    "io",
];

/// The category of an `Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// A time limit expired: `total`, `response_header`, `connect` (DNS, TCP, proxy, TLS, or QUIC handshake), or `read` or `body` during a body read.
    Timeout,
    /// Name resolution failed.
    Dns,
    /// The TCP connect failed, or the socket was refused or unreachable.
    Connect,
    /// The TLS handshake or certificate verification failed.
    Tls,
    /// The proxy dial, handshake, authentication, or `CONNECT` failed, or the session rejected an invalid environment proxy.
    Proxy,
    /// The proxy could not reach the origin.
    ProxyTarget,
    /// `error_for_status` rejected the status.
    Status,
    /// A response body passed `max_body_size` or a caller limit.
    BodyLimit,
    /// A body cannot be used: a response body already taken, a request body that cannot be compressed or resent, or a WebSocket protocol violation or oversized message.
    Body,
    /// Decompression failed: bad data, a truncated stream, or more than 4 codings.
    Decode,
    /// JSON serialization or deserialization failed.
    Json,
    /// An HTTP/2 protocol failure occurred.
    Http2,
    /// An HTTP/3 protocol or handshake failure occurred.
    Http3,
    /// A redirect could not be followed: a custom policy passed 32 hops, a `Location` scheme other than http or https, or a redirect that must resend a streamed request body.
    Redirect,
    /// The URL did not parse.
    Url,
    /// The session, request, TLS, trust store, or proxy configuration is invalid, including an invalid environment proxy at `build()`.
    Config,
    /// The request is malformed at the HTTP level, such as a bad header.
    Request,
    /// Socket I/O failed after the connect, or an HTTP/1.1 response broke the wire format.
    Io,
}

const _: () = assert!(LABELS.len() == Kind::Io as usize + 1);

impl Kind {
    pub(crate) const ALL: [Kind; 18] = [
        Self::Timeout,
        Self::Dns,
        Self::Connect,
        Self::Tls,
        Self::Proxy,
        Self::ProxyTarget,
        Self::Status,
        Self::BodyLimit,
        Self::Body,
        Self::Decode,
        Self::Json,
        Self::Http2,
        Self::Http3,
        Self::Redirect,
        Self::Url,
        Self::Config,
        Self::Request,
        Self::Io,
    ];

    /// Returns the lowercase label for the kind, such as `"timeout"`.
    pub fn as_str(self) -> &'static str {
        LABELS[self as usize]
    }

    /// Returns the status a gateway should answer with: 504 for a timeout, 500 for a local fault, 502 otherwise, and `None` for `Status`.
    pub fn gateway_status(self) -> Option<StatusCode> {
        match self {
            Self::Status => None,
            Self::Timeout => Some(StatusCode::GATEWAY_TIMEOUT),
            Self::Url | Self::Config | Self::Request => Some(StatusCode::INTERNAL_SERVER_ERROR),
            _ => Some(StatusCode::BAD_GATEWAY),
        }
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Error {
    /// Returns the error's category.
    pub fn kind(&self) -> Kind {
        if self.is_timeout() {
            return Kind::Timeout;
        }
        if self.is_status() {
            return Kind::Status;
        }
        if self.is_body_limit() {
            return Kind::BodyLimit;
        }
        match self.tls() {
            Some(tls) => tls_category(tls),
            None => self.kind_category(),
        }
    }

    fn kind_category(&self) -> Kind {
        match self.inner.kind {
            Cause::Proxy => Kind::Proxy,
            Cause::Tls => Kind::Tls,
            Cause::Connect => Kind::Connect,
            _ if self.is_socket_unreachable() => Kind::Connect,
            Cause::Status => Kind::Status,
            Cause::Body => Kind::Body,
            Cause::Decode => Kind::Decode,
            Cause::Json => Kind::Json,
            Cause::Http2 => Kind::Http2,
            #[cfg(feature = "http3")]
            Cause::Http3 => Kind::Http3,
            Cause::Redirect => Kind::Redirect,
            Cause::Url => Kind::Url,
            Cause::Config => Kind::Config,
            Cause::Request => Kind::Request,
            _ => Kind::Io,
        }
    }
}

fn tls_category(err: &TlsError) -> Kind {
    match err {
        TlsError::Dns(_) => Kind::Dns,
        TlsError::TcpConnect(_) => Kind::Connect,
        TlsError::ProxyTargetUnreachable { .. } => Kind::ProxyTarget,
        TlsError::Proxy { .. } => Kind::Proxy,
        TlsError::SslConfig(_) | TlsError::Profile(_) | TlsError::TrustStore(_) => Kind::Config,
        TlsError::Handshake(_)
        | TlsError::Rejected(_)
        | TlsError::HandshakeIo(_)
        | TlsError::Certificate { .. }
        | TlsError::Hostname(_)
        | TlsError::Pinning(_) => Kind::Tls,
    }
}
