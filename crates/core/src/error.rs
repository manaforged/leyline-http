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
    Tls(String),

    /// HTTP protocol error.
    #[error("http: {0}")]
    Http(String),

    /// IO error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// Certificate pinning verification failed.
    #[error("pin mismatch for {host}: expected {expected}, got {got}")]
    PinningFailed {
        /// The host that failed pinning.
        host: String,
        /// Expected pin hash(es).
        expected: String,
        /// Actual pin hash.
        got: String,
    },
}
