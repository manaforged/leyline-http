use std::io;

use crate::error::{Cause, Kind, is_gateway_status};

/// The code a proxy answered a tunnel request with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProxyReply {
    /// The HTTP status of a `CONNECT` reply.
    HttpStatus(u16),
    /// The SOCKS5 reply code.
    Socks5(u8),
}

/// A DNS, TCP, or proxy failure while opening a connection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NetError {
    /// The TCP connect failed.
    #[error("tcp connect")]
    TcpConnect(#[source] io::Error),

    /// Name resolution failed.
    #[error("dns resolution")]
    Dns(#[source] io::Error),

    /// The proxy dial, handshake, authentication, or `CONNECT` failed.
    #[error("proxy: {detail}")]
    Proxy {
        /// The proxy's HTTP status, when it answered with one.
        status: Option<u16>,
        /// What failed.
        detail: String,
        /// The I/O failure, when there was one.
        #[source]
        source: Option<io::Error>,
    },

    /// The proxy could not reach the target.
    #[error("proxy could not reach the target: {detail}")]
    ProxyTargetUnreachable {
        /// The proxy's reply code.
        reply: ProxyReply,
        /// What failed.
        detail: String,
    },
}

impl NetError {
    pub(crate) fn kind(&self) -> Kind {
        match self {
            Self::Dns(_) => Kind::Dns,
            Self::TcpConnect(_) => Kind::Connect,
            Self::ProxyTargetUnreachable { .. } => Kind::ProxyTarget,
            Self::Proxy { .. } => Kind::Proxy,
        }
    }

    pub(crate) fn proxy(detail: impl Into<String>) -> Self {
        Self::Proxy {
            status: None,
            detail: detail.into(),
            source: None,
        }
    }

    pub(crate) fn proxy_io(source: io::Error) -> Self {
        Self::Proxy {
            status: None,
            detail: "proxy connection failed".into(),
            source: Some(source),
        }
    }

    pub(crate) fn proxy_target(reply: ProxyReply, detail: impl Into<String>) -> Self {
        Self::ProxyTargetUnreachable {
            reply,
            detail: detail.into(),
        }
    }

    pub(crate) fn connect_timeout() -> Self {
        Self::TcpConnect(io::Error::new(io::ErrorKind::TimedOut, "connect timeout"))
    }

    pub(crate) fn into_proxy(self) -> Self {
        match self {
            Self::Proxy { .. } | Self::ProxyTargetUnreachable { .. } => self,
            Self::Dns(e) | Self::TcpConnect(e) => Self::proxy_io(e),
        }
    }

    pub(crate) fn copy(&self) -> Self {
        let io = |e: &io::Error| io::Error::new(e.kind(), e.to_string());
        match self {
            Self::TcpConnect(e) => Self::TcpConnect(io(e)),
            Self::Dns(e) => Self::Dns(io(e)),
            Self::Proxy {
                status,
                detail,
                source,
            } => Self::Proxy {
                status: *status,
                detail: detail.clone(),
                source: source.as_ref().map(io),
            },
            Self::ProxyTargetUnreachable { reply, detail } => Self::ProxyTargetUnreachable {
                reply: *reply,
                detail: detail.clone(),
            },
        }
    }
}

impl Cause for NetError {
    fn kind(&self) -> Option<Kind> {
        Some(NetError::kind(self))
    }

    fn is_proxy(&self) -> bool {
        matches!(self, Self::Proxy { .. })
    }

    fn proxy_status(&self) -> Option<u16> {
        match self {
            Self::Proxy { status, .. } => *status,
            Self::ProxyTargetUnreachable { reply, .. } => Some(match reply {
                ProxyReply::HttpStatus(code) => *code,
                ProxyReply::Socks5(code) => u16::from(*code),
            }),
            _ => None,
        }
    }

    fn is_transient(&self) -> bool {
        match self {
            Self::Proxy { status, source, .. } => {
                source.is_some() || status.is_some_and(is_gateway_status)
            }
            Self::ProxyTargetUnreachable {
                reply: ProxyReply::HttpStatus(code),
                ..
            } => is_gateway_status(*code),
            _ => false,
        }
    }

    fn is_connect_phase(&self) -> bool {
        matches!(self, Self::Dns(_) | Self::TcpConnect(_))
    }

    fn io_source(&self) -> Option<&io::Error> {
        match self {
            Self::TcpConnect(e) | Self::Dns(e) => Some(e),
            Self::Proxy { source, .. } => source.as_ref(),
            Self::ProxyTargetUnreachable { .. } => None,
        }
    }

    fn duplicate(&self) -> Option<Box<dyn Cause>> {
        Some(Box::new(self.copy()))
    }
}
