//! BoringSSL TLS connector with profile-driven fingerprint control.
//!
//! Creates TLS connections from TOML browser profiles by configuring
//! BoringSSL with the profile's cipher suites, curves, extensions, GREASE
//! behavior, and ALPS settings.

mod builder;
mod connector;
mod error;
mod happy_eyeballs;
mod resolver;

pub use builder::{build_ssl_context, TlsMinVersion};
pub use connector::FingerprintConnector;
pub use error::TlsError;
pub use happy_eyeballs::HappyEyeballsConfig;
pub use resolver::{ResolveFuture, Resolver, SystemResolver};

/// A connected TLS stream with ALPN result.
pub struct TlsStream {
    /// The async TLS stream.
    pub stream: tokio_boring::SslStream<tokio::net::TcpStream>,
    /// Negotiated ALPN protocol (e.g. "h2" or "http/1.1").
    pub alpn: Option<Vec<u8>>,
    /// Peer certificate in DER encoding, captured at handshake time.
    /// `None` if the peer presented no certificate or DER
    /// serialization failed.
    pub peer_cert_der: Option<Vec<u8>>,
    /// Negotiated TLS protocol version (e.g. `"TLS 1.3"`).
    pub tls_version: Option<String>,
    /// Negotiated TLS cipher suite name (e.g. `"TLS_AES_128_GCM_SHA256"`).
    pub tls_cipher: Option<String>,
}
