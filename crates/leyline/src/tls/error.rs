use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProxyReply {
    HttpStatus(u16),
    Socks5(u8),
}

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

    #[error("tcp connect")]
    TcpConnect(#[source] io::Error),

    #[error("dns resolution")]
    Dns(#[source] io::Error),

    #[error("proxy: {detail}")]
    Proxy {
        status: Option<u16>,
        detail: String,
        #[source]
        source: Option<io::Error>,
    },

    #[error("proxy could not reach the target: {detail}")]
    ProxyTargetUnreachable { reply: ProxyReply, detail: String },
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

fn transport_eof(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("unexpected eof")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("connection aborted")
}

impl TlsError {
    pub(crate) fn from_handshake<S>(e: &leyline_bssl_tokio::HandshakeError<S>) -> Self {
        if let Some(io) = e.as_io_error() {
            let copy = io::Error::new(io.kind(), io.to_string());
            if peer_closed(io.kind()) {
                return Self::Rejected(copy);
            }
            return Self::HandshakeIo(copy);
        }
        let msg = e.to_string();
        if transport_eof(&msg) {
            Self::Rejected(io::Error::new(io::ErrorKind::UnexpectedEof, msg))
        } else {
            Self::Handshake(msg)
        }
    }

    pub(crate) fn from_stack(e: leyline_bssl::error::ErrorStack) -> Self {
        Self::SslConfig(e.to_string())
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

    pub(crate) fn into_proxy(self) -> Self {
        match self {
            Self::Proxy { .. } | Self::ProxyTargetUnreachable { .. } => self,
            Self::Dns(e) | Self::TcpConnect(e) | Self::HandshakeIo(e) | Self::Rejected(e) => {
                Self::proxy_io(e)
            }
            other => Self::proxy(other.to_string()),
        }
    }

    pub(crate) fn io_source(&self) -> Option<&io::Error> {
        match self {
            Self::TcpConnect(e) | Self::Dns(e) | Self::HandshakeIo(e) | Self::Rejected(e) => {
                Some(e)
            }
            Self::Proxy { source, .. } => source.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn duplicate(&self) -> Self {
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
