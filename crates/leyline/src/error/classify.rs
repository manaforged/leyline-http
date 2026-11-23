use std::io;

use super::BodyLimit;
use super::{Cause, Error, Kind};

pub(crate) fn is_peer_closed(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    )
}

impl Error {
    /// Returns true when a connect, read, or total timeout ended the request.
    pub fn is_timeout(&self) -> bool {
        self.inner.kind == Kind::Timeout
            || self
                .io_cause()
                .is_some_and(|e| e.kind() == io::ErrorKind::TimedOut)
    }

    /// Returns true for a DNS or connect failure, a TLS failure while connecting, or a timeout while connecting.
    pub fn is_connect(&self) -> bool {
        match self.kind() {
            Kind::Connect | Kind::Dns => true,
            Kind::Tls => self.facts().is_some_and(Cause::is_connect_phase),
            Kind::Timeout => {
                self.inner.kind == Kind::Connect
                    || self.facts().is_some_and(Cause::is_connect_phase)
            }
            _ => false,
        }
    }

    pub(super) fn is_socket_unreachable(&self) -> bool {
        self.transport_io().is_some_and(|e| {
            matches!(
                e.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::AddrNotAvailable
                    | io::ErrorKind::NotConnected
                    | io::ErrorKind::NetworkUnreachable
            )
        })
    }

    pub(crate) fn is_status(&self) -> bool {
        self.inner.kind == Kind::Status
    }

    /// Returns true for a timeout, a connect failure, a closed connection, a transient proxy failure, or a 429, 502, 503 or 504 status error.
    pub fn is_retryable(&self) -> bool {
        self.is_transient_status()
            || self.is_timeout()
            || self.is_connect()
            || self.is_connection_closed()
            || self.is_proxy_transient()
    }

    fn is_transient_status(&self) -> bool {
        let status = self.inner.status.filter(|_| self.is_status());
        status.is_some_and(is_transient_status)
    }

    fn is_proxy_transient(&self) -> bool {
        self.facts().is_some_and(Cause::is_transient)
    }

    pub(crate) fn is_connection_closed(&self) -> bool {
        if self
            .transport_io()
            .is_some_and(|e| is_peer_closed(e.kind()))
        {
            return true;
        }
        if self.facts().is_some_and(Cause::is_closed) || self.is_refused_stream() {
            return true;
        }
        self.facts().is_some_and(Cause::is_handshake)
    }

    pub(crate) fn is_refused_stream(&self) -> bool {
        self.facts().is_some_and(Cause::refused)
    }

    pub(crate) fn is_body_limit(&self) -> bool {
        self.body_limit().is_some()
    }

    pub(crate) fn body_limit(&self) -> Option<BodyLimit> {
        self.source_as::<BodyLimit>()
            .copied()
            .or_else(|| self.transport_io().and_then(BodyLimit::of_io))
    }

    /// Returns true when the proxy itself failed.
    pub fn is_proxy(&self) -> bool {
        self.facts().is_some_and(Cause::is_proxy)
    }

    /// Returns the proxy's HTTP status or SOCKS5 reply code.
    #[must_use]
    pub fn proxy_status(&self) -> Option<u16> {
        self.facts()?.proxy_status()
    }

    /// Returns the HTTP/2 error code of a connection or stream error.
    #[must_use]
    pub fn h2_code(&self) -> Option<u32> {
        self.facts()?.h2_code()
    }

    pub(crate) fn is_unsent(&self) -> bool {
        !self.inner.after_response
            && (self.is_connect()
                || matches!(self.kind(), Kind::Proxy | Kind::ProxyTarget)
                || self.is_refused_stream())
    }

    pub(super) fn io_kind(&self) -> io::ErrorKind {
        let cause = self.io_cause().map(io::Error::kind);
        if let Some(kind) = cause.filter(|kind| *kind != io::ErrorKind::Other) {
            return kind;
        }
        match self.kind() {
            Kind::Timeout => io::ErrorKind::TimedOut,
            _ if self.is_connection_closed() => io::ErrorKind::ConnectionAborted,
            Kind::Connect | Kind::Dns => io::ErrorKind::NotConnected,
            Kind::Url | Kind::Config | Kind::Request => io::ErrorKind::InvalidInput,
            Kind::Tls | Kind::Decode | Kind::Json | Kind::Http2 | Kind::Http3 => {
                io::ErrorKind::InvalidData
            }
            _ => io::ErrorKind::Other,
        }
    }

    fn io_cause(&self) -> Option<&io::Error> {
        self.transport_io()
            .or_else(|| self.facts().and_then(Cause::io_source))
    }

    fn transport_io(&self) -> Option<&io::Error> {
        if matches!(self.inner.kind, Kind::Request | Kind::Config | Kind::Url) {
            return None;
        }
        self.io()
    }
}

const GATEWAY_STATUSES: [http::StatusCode; 3] = [
    http::StatusCode::BAD_GATEWAY,
    http::StatusCode::SERVICE_UNAVAILABLE,
    http::StatusCode::GATEWAY_TIMEOUT,
];

pub(crate) fn is_transient_status(status: http::StatusCode) -> bool {
    status == http::StatusCode::TOO_MANY_REQUESTS || is_gateway_status(status.as_u16())
}

pub(crate) fn is_gateway_status(code: u16) -> bool {
    GATEWAY_STATUSES
        .iter()
        .any(|status| status.as_u16() == code)
}
