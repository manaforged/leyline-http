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

    /// I/O failure while driving the TLS handshake.
    #[error("ssl handshake io: {0}")]
    HandshakeIo(#[source] std::io::Error),

    /// The server certificate chain is not trusted.
    #[error("certificate: {0}")]
    Certificate(String),

    /// The server certificate does not match the requested host.
    #[error("hostname: {0}")]
    Hostname(String),

    /// The server certificate does not match a configured leaf pin.
    #[error("certificate pin: {0}")]
    Pinning(String),

    /// TCP connection failed.
    #[error("tcp connect: {0}")]
    TcpConnect(#[source] std::io::Error),

    /// DNS resolution failed.
    #[error("dns: {0}")]
    Dns(#[source] std::io::Error),

    /// SSL stream initialization error.
    #[error("ssl connect: {0}")]
    SslConnect(String),

    /// Invalid profile configuration.
    #[error("profile: {0}")]
    Profile(String),

    /// System trust store could not be loaded (no roots available).
    #[error("trust store: {0}")]
    TrustStore(String),
}

impl TlsError {
    /// Whether retrying on a fresh connection can resolve this failure.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::TcpConnect(_) | Self::Dns(_) | Self::HandshakeIo(_)
        )
    }
}

// leyline_bssl errors are stringified at the boundary so no `leyline_bssl` type is nameable in
// the public `TlsError` enum (a leyline_bssl major bump can change these impls without
// breaking the variants consumers match on). These conversions keep `?`
// ergonomic across the BoringSSL build/handshake paths.
impl From<leyline_bssl::ssl::Error> for TlsError {
    fn from(e: leyline_bssl::ssl::Error) -> Self {
        match e.into_io_error() {
            Ok(e) => TlsError::HandshakeIo(e),
            Err(e) => TlsError::Handshake(e.to_string()),
        }
    }
}

impl From<leyline_bssl::error::ErrorStack> for TlsError {
    fn from(e: leyline_bssl::error::ErrorStack) -> Self {
        TlsError::SslConfig(e.to_string())
    }
}
