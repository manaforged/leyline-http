use std::io;

use super::{Error, Kind};
use crate::core::retry::GATEWAY_STATUSES;
use crate::core::session::decompress::BodyLimit;
use crate::h2::H2Error;
use crate::h2::error::ErrorCode;
use crate::tls::TlsError;
use crate::tls::error::ProxyReply;

impl Error {
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
        match self.tls() {
            Some(TlsError::Proxy { status, source, .. }) => {
                source.is_some() || status.is_some_and(|code| GATEWAY_STATUSES.contains(&code))
            }
            Some(TlsError::ProxyTargetUnreachable {
                reply: ProxyReply::HttpStatus(code),
                ..
            }) => GATEWAY_STATUSES.contains(code),
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

    pub fn is_body_limit(&self) -> bool {
        self.body_limit().is_some()
    }

    pub(crate) fn body_limit(&self) -> Option<BodyLimit> {
        self.source_as::<BodyLimit>()
            .copied()
            .or_else(|| self.io().and_then(BodyLimit::of_io))
    }

    pub fn is_proxy(&self) -> bool {
        matches!(self.tls(), Some(TlsError::Proxy { .. }))
    }

    fn io_cause(&self) -> Option<&io::Error> {
        self.io()
            .or_else(|| self.tls().and_then(TlsError::io_source))
    }
}
