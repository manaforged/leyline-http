use std::io;

use leyline_bssl::ssl::ErrorCode;

use crate::error::{Cause, Error, Kind};
use crate::net::NetError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TlsError {
    #[error("ssl config: {0}")]
    SslConfig(String),

    #[error("tls profile: {0}")]
    Profile(String),

    #[error("trust store: {0}")]
    TrustStore(String),

    #[error("tls handshake: {0}")]
    Handshake(String),

    #[error("tls handshake closed or reset by peer")]
    Rejected(#[source] io::Error),

    #[error("tls handshake io")]
    HandshakeIo(#[source] io::Error),

    #[error("certificate verification failed: {detail}")]
    Certificate {
        verify_code: Option<i32>,
        reason: Option<&'static str>,
        detail: String,
    },

    #[error("hostname: {0}")]
    Hostname(String),

    #[error("certificate pin: {0}")]
    Pinning(String),

    #[error(transparent)]
    Net(#[from] NetError),
}

fn peer_closed(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    )
}

impl TlsError {
    pub(crate) fn kind(&self) -> Kind {
        match self {
            Self::Net(net) => net.kind(),
            Self::SslConfig(_) | Self::Profile(_) | Self::TrustStore(_) => Kind::Config,
            Self::Handshake(_)
            | Self::Rejected(_)
            | Self::HandshakeIo(_)
            | Self::Certificate { .. }
            | Self::Hostname(_)
            | Self::Pinning(_) => Kind::Tls,
        }
    }

    pub(crate) fn from_handshake<S>(e: &leyline_bssl_tokio::HandshakeError<S>) -> Self {
        if let Some(io) = e.as_io_error() {
            let copy = io::Error::new(io.kind(), io.to_string());
            if peer_closed(io.kind()) {
                return Self::Rejected(copy);
            }
            return Self::HandshakeIo(copy);
        }
        let msg = e.to_string();
        if e.code() == Some(ErrorCode::SYSCALL) {
            Self::Rejected(io::Error::new(io::ErrorKind::UnexpectedEof, msg))
        } else {
            Self::Handshake(msg)
        }
    }

    pub(crate) fn from_stack(e: leyline_bssl::error::ErrorStack) -> Self {
        Self::SslConfig(e.to_string())
    }

    fn net(&self) -> Option<&NetError> {
        match self {
            Self::Net(e) => Some(e),
            _ => None,
        }
    }

    pub(crate) fn into_proxy(self) -> Self {
        match self {
            Self::Net(e) => Self::Net(e.into_proxy()),
            Self::HandshakeIo(e) | Self::Rejected(e) => Self::Net(NetError::proxy_io(e)),
            other => Self::Net(NetError::proxy(other.to_string())),
        }
    }

    fn copy(&self) -> Self {
        let io = |e: &io::Error| io::Error::new(e.kind(), e.to_string());
        match self {
            Self::SslConfig(m) => Self::SslConfig(m.clone()),
            Self::Profile(m) => Self::Profile(m.clone()),
            Self::TrustStore(m) => Self::TrustStore(m.clone()),
            Self::Handshake(m) => Self::Handshake(m.clone()),
            Self::Rejected(e) => Self::Rejected(io(e)),
            Self::HandshakeIo(e) => Self::HandshakeIo(io(e)),
            Self::Certificate {
                verify_code,
                reason,
                detail,
            } => Self::Certificate {
                verify_code: *verify_code,
                reason: *reason,
                detail: detail.clone(),
            },
            Self::Hostname(m) => Self::Hostname(m.clone()),
            Self::Pinning(m) => Self::Pinning(m.clone()),
            Self::Net(e) => Self::Net(e.copy()),
        }
    }
}

impl Cause for TlsError {
    fn kind(&self) -> Option<Kind> {
        Some(TlsError::kind(self))
    }

    fn is_connect_phase(&self) -> bool {
        self.net().is_some_and(Cause::is_connect_phase) || self.is_handshake()
    }

    fn is_handshake(&self) -> bool {
        matches!(
            self,
            Self::Handshake(_) | Self::HandshakeIo(_) | Self::Rejected(_)
        )
    }

    fn is_proxy(&self) -> bool {
        self.net().is_some_and(Cause::is_proxy)
    }

    fn proxy_status(&self) -> Option<u16> {
        self.net()?.proxy_status()
    }

    fn is_transient(&self) -> bool {
        self.net().is_some_and(Cause::is_transient)
    }

    fn io_source(&self) -> Option<&io::Error> {
        match self {
            Self::HandshakeIo(e) | Self::Rejected(e) => Some(e),
            Self::Net(e) => e.io_source(),
            _ => None,
        }
    }

    fn duplicate(&self) -> Option<Box<dyn Cause>> {
        Some(Box::new(self.copy()))
    }
}

impl From<TlsError> for Error {
    fn from(e: TlsError) -> Self {
        Error::new(e.kind()).with_cause(e)
    }
}

impl From<NetError> for Error {
    fn from(e: NetError) -> Self {
        Error::from(TlsError::from(e))
    }
}

#[cfg(test)]
impl Error {
    pub(crate) fn tls(&self) -> Option<&TlsError> {
        self.source_as()
    }
}
