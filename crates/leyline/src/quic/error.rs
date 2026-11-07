use std::fmt;

use super::H3SendError;
use crate::error::{Cause, Error, Kind};

/// Why an HTTP/3 connection could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QuicError {
    /// The QUIC or HTTP/3 settings could not be applied.
    Config(String),
    /// The host name did not resolve.
    Dns(String),
    /// The proxy cannot carry QUIC or refused the relay.
    Proxy(String),
    /// The UDP socket failed.
    Socket(String),
    /// The QUIC or HTTP/3 handshake failed.
    Protocol(String),
}

impl QuicError {
    pub(crate) const fn kind(&self) -> Kind {
        match self {
            Self::Config(_) => Kind::Config,
            Self::Dns(_) => Kind::Dns,
            Self::Proxy(_) => Kind::Proxy,
            Self::Socket(_) => Kind::Connect,
            Self::Protocol(_) => Kind::Http3,
        }
    }

    fn message(&self) -> &str {
        match self {
            Self::Config(m)
            | Self::Dns(m)
            | Self::Proxy(m)
            | Self::Socket(m)
            | Self::Protocol(m) => m,
        }
    }
}

impl fmt::Display for QuicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl From<QuicError> for Error {
    fn from(error: QuicError) -> Self {
        let kind = error.kind();
        Error::new(kind).with_message(error.to_string())
    }
}

impl From<H3SendError> for Error {
    fn from(e: H3SendError) -> Self {
        match e {
            H3SendError::BodyLimit(limit) => limit.error(),
            H3SendError::RequestBody(io) => Error::from_request_body(io),
            H3SendError::NotSent(_) | H3SendError::Rejected(_) | H3SendError::Closed(_) => {
                let message = e.message().into_owned();
                Error::new(Kind::Http3).with_message(message).with_cause(e)
            }
            other => Error::new(Kind::Http3).with_message(other.message().into_owned()),
        }
    }
}

impl Cause for H3SendError {
    fn is_closed(&self) -> bool {
        matches!(self, H3SendError::Closed(_))
    }

    fn refused(&self) -> bool {
        matches!(self, H3SendError::NotSent(_) | H3SendError::Rejected(_))
    }

    fn duplicate(&self) -> Option<Box<dyn Cause>> {
        match self {
            H3SendError::NotSent(message) => Some(Box::new(H3SendError::NotSent(message.clone()))),
            H3SendError::Rejected(message) => {
                Some(Box::new(H3SendError::Rejected(message.clone())))
            }
            H3SendError::Closed(message) => Some(Box::new(H3SendError::Closed(message.clone()))),
            _ => None,
        }
    }
}
