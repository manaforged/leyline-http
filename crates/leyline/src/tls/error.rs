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
    /// Connect-phase failures a new TCP+TLS attempt can recover.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::TcpConnect(_)
                | Self::Dns(_)
                | Self::HandshakeIo(_)
                | Self::Handshake(_)
                | Self::SslConnect(_)
        )
    }
}

fn transport_eof(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("unexpected eof")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("connection aborted")
}

impl TlsError {
    /// Map a BoringSSL handshake error; crate-private so `leyline_bssl` stays off the public API.
    pub(crate) fn from_ssl(e: leyline_bssl::ssl::Error) -> Self {
        match e.into_io_error() {
            Ok(e) => Self::HandshakeIo(e),
            Err(e) => {
                let msg = e.to_string();
                if transport_eof(&msg) {
                    Self::HandshakeIo(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, msg))
                } else {
                    Self::Handshake(msg)
                }
            }
        }
    }

    /// Map a BoringSSL configuration error stack; crate-private so `leyline_bssl` stays off the public API.
    pub(crate) fn from_stack(e: leyline_bssl::error::ErrorStack) -> Self {
        Self::SslConfig(e.to_string())
    }
}

#[cfg(test)]
mod tests;
