//! TLS-specific error types.

/// Errors from the TLS connector.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TlsError {
    /// BoringSSL configuration error.
    #[error("ssl config: {0}")]
    SslConfig(String),

    /// BoringSSL handshake error.
    #[error("ssl handshake: {0}")]
    Handshake(String),

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

    /// System trust store could not be loaded (no roots available).
    #[error("trust store: {0}")]
    TrustStore(String),
}

// btls errors are stringified at the boundary so no `btls` type is nameable in
// the public `TlsError` enum (a btls major bump can change these impls without
// breaking the variants consumers match on). These conversions keep `?`
// ergonomic across the BoringSSL build/handshake paths.
impl From<btls::ssl::Error> for TlsError {
    fn from(e: btls::ssl::Error) -> Self {
        TlsError::SslConfig(e.to_string())
    }
}

impl From<btls::error::ErrorStack> for TlsError {
    fn from(e: btls::error::ErrorStack) -> Self {
        TlsError::Handshake(e.to_string())
    }
}
