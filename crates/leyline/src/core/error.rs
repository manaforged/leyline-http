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
