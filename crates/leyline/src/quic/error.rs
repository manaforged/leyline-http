use std::fmt;

use super::H3SendError;
use crate::error::{Cause, Error, Kind};

/// Why an HTTP/3 connection could not be opened.
#[derive(Debug)]
pub(crate) enum QuicError {
    /// The QUIC or HTTP/3 settings could not be applied.
    Config(String),
    /// The host name did not resolve.
    Dns(String),
    /// Name resolution timed out.
    Timeout(String),
    /// The proxy cannot carry QUIC or refused the relay.
    Proxy(String),
    /// A network or SOCKS5 proxy error while opening the relay.
    #[cfg(feature = "socks")]
    ProxyNet(Box<crate::net::NetError>),
    /// The UDP socket failed.
    Socket(String),
    /// The TLS handshake or certificate check failed.
    Tls(String),
    /// The QUIC or HTTP/3 handshake failed.
    Protocol(String),
}

impl Clone for QuicError {
    fn clone(&self) -> Self {
        match self {
            Self::Config(m) => Self::Config(m.clone()),
            Self::Dns(m) => Self::Dns(m.clone()),
            Self::Timeout(m) => Self::Timeout(m.clone()),
            Self::Proxy(m) => Self::Proxy(m.clone()),
            #[cfg(feature = "socks")]
            Self::ProxyNet(e) => Self::ProxyNet(Box::new(e.copy())),
            Self::Socket(m) => Self::Socket(m.clone()),
            Self::Tls(m) => Self::Tls(m.clone()),
            Self::Protocol(m) => Self::Protocol(m.clone()),
        }
    }
}

impl QuicError {
    pub(crate) fn from_net_dns(err: crate::net::NetError) -> Self {
        let msg = format!("dns: {err}");
        match err.kind() {
            Kind::Timeout => Self::Timeout(msg),
            Kind::Config => Self::Config(msg),
            _ => Self::Dns(msg),
        }
    }

    pub(crate) fn kind(&self) -> Kind {
        match self {
            Self::Config(_) => Kind::Config,
            Self::Dns(_) => Kind::Dns,
            Self::Timeout(_) => Kind::Timeout,
            Self::Proxy(_) => Kind::Proxy,
            #[cfg(feature = "socks")]
            Self::ProxyNet(err) => err.kind(),
            Self::Socket(_) => Kind::Connect,
            Self::Tls(_) => Kind::Tls,
            Self::Protocol(_) => Kind::Http3,
        }
    }
}

impl fmt::Display for QuicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(m)
            | Self::Dns(m)
            | Self::Timeout(m)
            | Self::Proxy(m)
            | Self::Socket(m)
            | Self::Tls(m)
            | Self::Protocol(m) => f.write_str(m),
            #[cfg(feature = "socks")]
            Self::ProxyNet(err) => write!(f, "h3 proxy: {err}"),
        }
    }
}

impl From<QuicError> for Error {
    fn from(error: QuicError) -> Self {
        match error {
            #[cfg(feature = "socks")]
            QuicError::ProxyNet(err) => {
                let kind = err.kind();
                let message = format!("h3 proxy: {err}");
                Error::new(kind).with_message(message).with_cause(*err)
            }
            other => {
                let kind = other.kind();
                Error::new(kind).with_message(other.to_string())
            }
        }
    }
}

impl From<H3SendError> for Error {
    fn from(e: H3SendError) -> Self {
        match e {
            H3SendError::BodyLimit(limit) => limit.error(),
            H3SendError::RequestBody(io) => Error::from_request_body(io),
            H3SendError::Tls(message) => Error::new(Kind::Tls).with_message(message),
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
