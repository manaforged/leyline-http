//! BoringSSL TLS connector with full fingerprint control.
//!
//! Creates TLS connections that match real browser ClientHello fingerprints
//! by configuring BoringSSL with exact cipher suites, curves, extensions,
//! GREASE behavior, and ALPS settings from TOML browser profiles.

mod connector;
mod error;

pub use connector::FingerprintConnector;
pub use error::TlsError;

/// A connected TLS stream with ALPN result.
pub struct TlsStream {
    /// The async TLS stream.
    pub stream: tokio_boring2::SslStream<tokio::net::TcpStream>,
    /// Negotiated ALPN protocol (e.g. "h2" or "http/1.1").
    pub alpn: Option<Vec<u8>>,
}
