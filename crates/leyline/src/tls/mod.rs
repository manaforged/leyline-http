//! BoringSSL TLS connector with profile-driven fingerprint control.
//!
//! Creates TLS connections from TOML browser profiles by configuring
//! BoringSSL with the profile's cipher suites, curves, extensions, GREASE
//! behavior, and ALPS settings.

mod builder;
mod connector;
mod error;
mod happy_eyeballs;
mod nonblocking;
mod proxy;
mod resolver;
mod stream;
mod trust;
#[cfg(feature = "tls-rustls")]
mod rustls_trust;
#[cfg(feature = "tls-rustls")]
mod rustls_connector;
#[cfg(windows)]
mod windows_trust;

pub use builder::{build_ssl_context, TlsMinVersion};
pub use connector::FingerprintConnector;
pub use error::TlsError;
pub use happy_eyeballs::HappyEyeballsConfig;
pub use resolver::{ResolveFuture, Resolver, SystemResolver};
pub use trust::{ClientIdentity, TlsTrustConfig};

pub(crate) use stream::TlsIo;
#[cfg(feature = "tls-rustls")]
pub use rustls_connector::RustlsConnector;

/// A connected TLS stream with ALPN result.
pub struct TlsStream {
    /// The async TLS stream, behind the backend-agnostic [`TlsIo`] seam.
    /// `pub(crate)` because the concrete backend is an internal detail —
    /// consumers read the neutral metadata fields below, not the raw IO.
    pub(crate) stream: TlsIo,
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

/// The active connector backend, selected once by the session builder.
/// Bare `Session::new()` sessions use the pure-Rust [`RustlsConnector`];
/// browser/profile sessions use the BoringSSL [`FingerprintConnector`].
/// The rest of the stack dispatches `connect`/`connect_h1` through here.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ConnectorVariant {
    /// BoringSSL fingerprinting connector — browser/profile sessions.
    Fingerprint(FingerprintConnector),
    /// Pure-Rust rustls connector — bare `Session::new()` sessions.
    #[cfg(feature = "tls-rustls")]
    Bare(RustlsConnector),
}

impl ConnectorVariant {
    pub(crate) async fn connect(
        &self,
        host: &str,
        port: u16,
        proxy: Option<&str>,
    ) -> Result<TlsStream, TlsError> {
        match self {
            ConnectorVariant::Fingerprint(c) => c.connect(host, port, proxy).await,
            #[cfg(feature = "tls-rustls")]
            ConnectorVariant::Bare(c) => c.connect(host, port, proxy).await,
        }
    }

    pub(crate) async fn connect_h1(
        &self,
        host: &str,
        port: u16,
        proxy: Option<&str>,
    ) -> Result<TlsStream, TlsError> {
        match self {
            ConnectorVariant::Fingerprint(c) => c.connect_h1(host, port, proxy).await,
            #[cfg(feature = "tls-rustls")]
            ConnectorVariant::Bare(c) => c.connect_h1(host, port, proxy).await,
        }
    }
}

/// Drives a TLS handshake over an already-connected TCP stream.
///
/// The hook the [`proxy`] module needs so HTTP-CONNECT / SOCKS5
/// tunnels work for any TLS backend without re-plumbing fingerprint
/// state. `include_alps` is the direct-path ALPN policy: `true` for
/// h2, `false` for http/1.1-only (WebSocket upgrade).
pub(crate) trait TlsHandshake {
    fn do_tls_handshake(
        &self,
        tcp_stream: tokio::net::TcpStream,
        host: &str,
        include_alps: bool,
    ) -> impl std::future::Future<Output = Result<TlsStream, TlsError>> + Send;
}

impl TlsHandshake for FingerprintConnector {
    async fn do_tls_handshake(
        &self,
        tcp_stream: tokio::net::TcpStream,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        self.tls_handshake(tcp_stream, host, include_alps).await
    }
}
