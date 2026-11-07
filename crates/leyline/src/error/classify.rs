use std::io;

use super::BodyLimit;
use super::{Cause, Error, Kind};

impl Error {
    pub(crate) fn is_timeout(&self) -> bool {
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

    pub(crate) fn is_status(&self) -> bool {
        self.inner.kind == Kind::Status
    }

    /// Returns true for a timeout, a connect failure, a closed connection, or a transient proxy failure.
    pub fn is_retryable(&self) -> bool {
        self.is_timeout()
            || self.is_connect()
            || self.is_connection_closed()
            || self.is_proxy_transient()
    }

    fn is_proxy_transient(&self) -> bool {
        self.facts().is_some_and(Cause::is_transient)
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
            .or_else(|| self.io().and_then(BodyLimit::of_io))
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

    fn io_cause(&self) -> Option<&io::Error> {
        self.io()
            .or_else(|| self.facts().and_then(Cause::io_source))
    }
}

const GATEWAY_STATUSES: [http::StatusCode; 3] = [
    http::StatusCode::BAD_GATEWAY,
    http::StatusCode::SERVICE_UNAVAILABLE,
    http::StatusCode::GATEWAY_TIMEOUT,
];

pub(crate) fn is_gateway_status(code: u16) -> bool {
    GATEWAY_STATUSES
        .iter()
        .any(|status| status.as_u16() == code)
}
