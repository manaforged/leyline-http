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
        /// Response body (for debugging 403s, rate limits, etc).
        body: Vec<u8>,
    },

    /// IO error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// True if this error is a request timeout. Mirrors
    /// `reqwest::Error::is_timeout` so retry/backoff code ports across.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Error::Timeout)
    }

    /// True if this is a connection-establishment failure (TCP, TLS
    /// handshake, proxy tunnel, or low-level IO) rather than a protocol
    /// or status error. Mirrors `reqwest::Error::is_connect`.
    pub fn is_connect(&self) -> bool {
        matches!(self, Error::Io(_) | Error::Tls(_) | Error::Proxy(_))
    }

    /// True if this error carries an HTTP status (from
    /// [`Response::error_for_status`](crate::Response::error_for_status)).
    /// Mirrors `reqwest::Error::is_status`.
    pub fn is_status(&self) -> bool {
        matches!(self, Error::Status { .. })
    }

    /// The HTTP status code, when this error carries one. Returns `u16`
    /// to match [`Response::status`](crate::Response::status) (reqwest
    /// returns `Option<StatusCode>`); `None` for non-status errors.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Status { code, .. } => Some(*code),
            _ => None,
        }
    }

    /// True if the error indicates the connection went away (peer closed,
    /// graceful GOAWAY, or a transport-level EOF/reset) and the request can
    /// be safely retried on a fresh connection.
    pub fn is_connection_closed(&self) -> bool {
        use crate::h2::error::ErrorCode;
        use crate::h2::H2Error;
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
            _ => false,
        }
    }
}
