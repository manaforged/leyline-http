//! TLS-specific error types.

/// Errors from the TLS connector.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TlsError {
    /// BoringSSL configuration error.
    #[error("ssl config: {0}")]
    SslConfig(#[from] boring2::ssl::Error),

    /// BoringSSL handshake error.
    #[error("ssl handshake: {0}")]
    Handshake(#[from] boring2::error::ErrorStack),

    /// TCP connection failed.
    #[error("tcp connect: {0}")]
    TcpConnect(#[source] std::io::Error),

    /// DNS resolution failed.
    #[error("dns: {0}")]
    Dns(#[source] std::io::Error),

    /// SSL connect/negotiation error.
    #[error("ssl connect: {0}")]
    SslConnect(String),

    /// Invalid profile configuration.
    #[error("profile: {0}")]
    Profile(String),
}
