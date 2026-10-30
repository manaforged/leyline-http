use std::io;

use super::{Cause, Error, Kind};
use crate::core::retry::is_gateway_status;
use crate::core::session::decompress::BodyLimit;
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;
use crate::tls::TlsError;
use crate::tls::error::ProxyReply;

impl Error {
    pub(crate) fn is_timeout(&self) -> bool {
        self.inner.kind == Cause::Timeout
            || self
                .io_cause()
                .is_some_and(|e| e.kind() == io::ErrorKind::TimedOut)
    }

    /// Returns true for a DNS or connect failure, a TLS failure while connecting, or a timeout while connecting.
    pub fn is_connect(&self) -> bool {
        match self.kind() {
            Kind::Connect | Kind::Dns => true,
            Kind::Tls => self.tls().is_some_and(TlsError::is_connect_phase),
            Kind::Timeout => {
                self.inner.kind == Cause::Connect
                    || self.tls().is_some_and(TlsError::is_connect_phase)
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
        self.inner.kind == Cause::Status
    }

    /// Returns true for a timeout, a connect failure, a closed connection, or a transient proxy failure.
    pub fn is_retryable(&self) -> bool {
        self.is_timeout()
            || self.is_connect()
            || self.is_connection_closed()
            || self.is_proxy_transient()
    }

    fn is_proxy_transient(&self) -> bool {
        match self.tls() {
            Some(TlsError::Proxy { status, source, .. }) => {
                source.is_some() || status.is_some_and(is_gateway_status)
            }
            Some(TlsError::ProxyTargetUnreachable {
                reply: ProxyReply::HttpStatus(code),
                ..
            }) => is_gateway_status(*code),
            _ => false,
        }
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
            )
        ) || self.is_refused_stream()
        {
            return true;
        }
        self.tls().is_some_and(TlsError::is_handshake)
    }

    pub(crate) fn is_refused_stream(&self) -> bool {
        matches!(
            self.h2(),
            Some(H2Error::Stream {
                code: ErrorCode::RefusedStream,
                ..
            })
        ) || self.not_processed()
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
        matches!(self.tls(), Some(TlsError::Proxy { .. }))
    }

    /// Returns the proxy's HTTP status or SOCKS5 reply code.
    #[must_use]
    pub fn proxy_status(&self) -> Option<u16> {
        match self.tls()? {
            TlsError::Proxy { status, .. } => *status,
            TlsError::ProxyTargetUnreachable { reply, .. } => Some(match reply {
                ProxyReply::HttpStatus(code) => *code,
                ProxyReply::Socks5(code) => u16::from(*code),
            }),
            _ => None,
        }
    }

    /// Returns the HTTP/2 error code of a connection or stream error.
    #[must_use]
    pub fn h2_code(&self) -> Option<u32> {
        match self.h2()? {
            H2Error::Connection { code, .. } | H2Error::Stream { code, .. } => Some(*code as u32),
            _ => None,
        }
    }

    pub(crate) fn is_unsent(&self) -> bool {
        !self.follows_response()
            && (self.is_connect()
                || matches!(self.kind(), Kind::Proxy | Kind::ProxyTarget)
                || self.is_refused_stream())
    }

    fn io_cause(&self) -> Option<&io::Error> {
        self.io()
            .or_else(|| self.tls().and_then(TlsError::io_source))
    }
}
