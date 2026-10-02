use http::StatusCode;

use super::{Error, Kind};
use crate::tls::TlsError;

const LABELS: [&str; 16] = [
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
    "protocol",
    "redirect",
    "url",
    "config",
    "request",
    "other",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCategory {
    Timeout,
    Dns,
    Connect,
    Tls,
    Proxy,
    ProxyTarget,
    Status,
    BodyLimit,
    Body,
    Decode,
    Protocol,
    Redirect,
    Url,
    Config,
    Request,
    Other,
}

const _: () = assert!(LABELS.len() == ErrorCategory::Other as usize + 1);

impl ErrorCategory {
    pub(crate) const ALL: [ErrorCategory; 16] = [
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
        Self::Protocol,
        Self::Redirect,
        Self::Url,
        Self::Config,
        Self::Request,
        Self::Other,
    ];

    pub fn as_str(self) -> &'static str {
        LABELS[self as usize]
    }

    pub fn gateway_status(self) -> Option<StatusCode> {
        match self {
            Self::Status => None,
            Self::Timeout => Some(StatusCode::GATEWAY_TIMEOUT),
            Self::Url | Self::Config | Self::Request => Some(StatusCode::INTERNAL_SERVER_ERROR),
            _ => Some(StatusCode::BAD_GATEWAY),
        }
    }
}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Error {
    pub fn category(&self) -> ErrorCategory {
        if self.is_timeout() {
            return ErrorCategory::Timeout;
        }
        if self.is_status() {
            return ErrorCategory::Status;
        }
        if self.is_body_limit() {
            return ErrorCategory::BodyLimit;
        }
        match self.tls() {
            Some(tls) => tls_category(tls),
            None => self.kind_category(),
        }
    }

    pub fn is_dns(&self) -> bool {
        matches!(self.tls(), Some(TlsError::Dns(_)))
    }

    fn kind_category(&self) -> ErrorCategory {
        match self.inner.kind {
            Kind::Proxy => ErrorCategory::Proxy,
            Kind::Tls => ErrorCategory::Tls,
            Kind::Connect => ErrorCategory::Connect,
            _ if self.is_socket_unreachable() => ErrorCategory::Connect,
            Kind::Status => ErrorCategory::Status,
            Kind::Body => ErrorCategory::Body,
            Kind::Decode | Kind::Json => ErrorCategory::Decode,
            Kind::Http2 | Kind::Http3 => ErrorCategory::Protocol,
            Kind::Redirect => ErrorCategory::Redirect,
            Kind::Url => ErrorCategory::Url,
            Kind::Config => ErrorCategory::Config,
            Kind::Request => ErrorCategory::Request,
            _ => ErrorCategory::Other,
        }
    }
}

fn tls_category(err: &TlsError) -> ErrorCategory {
    match err {
        TlsError::Dns(_) => ErrorCategory::Dns,
        TlsError::TcpConnect(_) => ErrorCategory::Connect,
        TlsError::ProxyTargetUnreachable { .. } => ErrorCategory::ProxyTarget,
        TlsError::Proxy { .. } => ErrorCategory::Proxy,
        TlsError::SslConfig(_) | TlsError::Profile(_) | TlsError::TrustStore(_) => {
            ErrorCategory::Config
        }
        TlsError::Handshake(_)
        | TlsError::Rejected(_)
        | TlsError::HandshakeIo(_)
        | TlsError::Certificate { .. }
        | TlsError::Hostname(_)
        | TlsError::Pinning(_) => ErrorCategory::Tls,
    }
}
