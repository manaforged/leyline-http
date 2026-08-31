//! Error types for Leyline.

/// Leyline result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during Leyline operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Invalid configuration (proxy URL, profile, etc).
    #[error("config: {0}")]
    Config(String),

    /// URL parsing failed.
    #[error("url: {0}")]
    Url(#[from] url::ParseError),

    /// JSON serialization/deserialization failed.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    /// Request timed out.
    #[error("timeout")]
    Timeout,

    /// TLS handshake or connection error.
    #[error("tls: {0}")]
    Tls(#[from] crate::tls::TlsError),

    /// Proxy configuration or tunnel error.
    #[error("proxy: {0}")]
    Proxy(String),

    /// HTTP/2 protocol or transport error.
    #[error("http2: {0}")]
    Http2(#[from] crate::h2::H2Error),

    /// ALPN negotiated a protocol other than `h2` when `h2` was required.
    #[error("alpn: negotiated {negotiated}, expected h2")]
    AlpnMismatch {
        /// The protocol the peer negotiated (or `none` if none was offered).
        negotiated: String,
    },

    /// HTTP/3 / QUIC protocol or transport error.
    #[error("http3: {0}")]
    Http3(String),

    /// Body streaming or buffering error.
    #[error("body: {0}")]
    Body(String),

    /// Response decompression or decoding error.
    #[error("decode: {0}")]
    Decode(String),

    /// Redirect handling error.
    #[error("redirect: {0}")]
    Redirect(String),

    /// HTTP protocol error.
    #[error("http: {0}")]
    Http(String),

    /// HTTP status code error (4xx/5xx from `error_for_status()`).
    #[error("HTTP {code} for {url}")]
    Status {
        /// HTTP status code.
        code: u16,
        /// Request URL.
        url: String,
        /// Response body prefix (for debugging 403s, rate limits, etc), truncated to the first 16 KiB by [`Response::error_for_status`](crate::Response::error_for_status) so a large body is not retained in — and logged with — the error.
        body: Vec<u8>,
    },

    /// IO error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// True if this error is a request timeout.
    pub fn is_timeout(&self) -> bool {
        match self {
            Error::Timeout => true,
            Error::Io(e) if e.kind() == std::io::ErrorKind::TimedOut => true,
            Error::Tls(e) => io_kind(e).is_some_and(|k| k == std::io::ErrorKind::TimedOut),
            _ => false,
        }
    }

    /// True if this is a connection-establishment failure (TCP, DNS, TLS handshake, or proxy tunnel), not body or file I/O.
    pub fn is_connect(&self) -> bool {
        use crate::tls::TlsError;
        match self {
            Error::Proxy(_) => true,
            Error::Tls(
                TlsError::TcpConnect(_)
                | TlsError::Dns(_)
                | TlsError::Handshake(_)
                | TlsError::HandshakeIo(_)
                | TlsError::SslConnect(_),
            ) => true,
            Error::Io(e) => matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::AddrNotAvailable
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::NetworkUnreachable
            ),
            _ => false,
        }
    }

    /// True if this error carries an HTTP status (from [`Response::error_for_status`](crate::Response::error_for_status)).
    pub fn is_status(&self) -> bool {
        matches!(self, Error::Status { .. })
    }

    /// The HTTP status code, when this error carries one.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Status { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// True if the error indicates the connection went away (peer closed, graceful GOAWAY, or a transport-level EOF/reset) and the request can be safely retried on a fresh connection.
    pub fn is_connection_closed(&self) -> bool {
        use crate::h2::H2Error;
        use crate::h2::error::ErrorCode;
        match self {
            Error::Io(e) => matches!(
                e.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
            ),
            Error::Http2(H2Error::Io(_)) => true,
            Error::Http2(H2Error::Connection {
                code: ErrorCode::NoError,
                ..
            }) => true,
            Error::Http2(H2Error::Stream {
                code: ErrorCode::RefusedStream,
                ..
            }) => true,
            Error::Tls(err) => err.is_retryable(),
            _ => false,
        }
    }
}

fn io_kind(err: &crate::tls::TlsError) -> Option<std::io::ErrorKind> {
    use crate::tls::TlsError;
    match err {
        TlsError::TcpConnect(e) | TlsError::Dns(e) | TlsError::HandshakeIo(e) => Some(e.kind()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "error/tests.rs"]
mod tests;
