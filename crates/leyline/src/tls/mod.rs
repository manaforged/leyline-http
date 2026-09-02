//! BoringSSL TLS connector with profile-driven fingerprint control.

mod builder;
mod connector;
mod error;
mod happy_eyeballs;
mod keylog;
#[cfg(target_os = "macos")]
mod macos_trust;
mod nonblocking;
pub(crate) mod proxy;
mod resolver;
mod stream;
mod trust;
#[cfg(windows)]
mod windows_trust;

pub use builder::TlsMinVersion;
pub use error::TlsError;
pub use happy_eyeballs::HappyEyeballsConfig;
pub use resolver::{ResolveFuture, Resolver, SystemResolver};
pub use trust::{ClientIdentity, TlsTrustConfig};

#[doc(hidden)]
pub use connector::FingerprintConnector;

pub(crate) use builder::apply_profile_with_trust;
pub(crate) use builder::build_ssl_context;
pub(crate) use stream::TlsIo;
pub(crate) use trust::install_pinning_verifier_ctx;

/// A BoringSSL TLS context preconfigured to a browser profile's fingerprint.
pub struct TlsContext(
    #[cfg_attr(
        not(feature = "unstable-bssl"),
        expect(dead_code, reason = "read only through the unstable-bssl accessors")
    )]
    leyline_bssl::ssl::SslContextBuilder,
);

impl TlsContext {
    /// Build a context matching `profile`, pinned to `min_version`.
    pub fn from_profile(
        profile: &crate::profile::BrowserProfile,
        min_version: TlsMinVersion,
    ) -> Result<Self, TlsError> {
        build_ssl_context(profile, min_version).map(Self)
    }

    /// Mutable access to the underlying BoringSSL `SslContextBuilder`. Unstable: no semver promise, and the BoringSSL type may change with any release.
    #[cfg(feature = "unstable-bssl")]
    pub fn builder_mut(&mut self) -> &mut leyline_bssl::ssl::SslContextBuilder {
        &mut self.0
    }

    /// Consume the wrapper and return the underlying BoringSSL builder. Unstable: no semver promise, and the BoringSSL type may change with any release.
    #[cfg(feature = "unstable-bssl")]
    pub fn into_inner(self) -> leyline_bssl::ssl::SslContextBuilder {
        self.0
    }
}

/// A connected TLS stream with ALPN result.
#[doc(hidden)]
pub struct TlsStream {
    /// The async TLS stream.
    pub(crate) stream: TlsIo,
    /// Negotiated ALPN protocol (e.g. "h2" or "http/1.1").
    pub alpn: Option<Vec<u8>>,
    /// Peer certificate in DER encoding, captured at handshake time.
    pub peer_cert_der: Option<Vec<u8>>,
    /// Negotiated TLS protocol version (e.g. `"TLS 1.3"`).
    pub tls_version: Option<String>,
    /// Negotiated TLS cipher suite name (e.g. `"TLS_AES_128_GCM_SHA256"`).
    pub tls_cipher: Option<String>,
}

/// Drives a TLS handshake over an already-connected TCP stream.
pub(crate) trait TlsHandshake {
    fn do_tls_handshake(
        &self,
        tcp_stream: tokio::net::TcpStream,
        host: &str,
        include_alps: bool,
    ) -> impl std::future::Future<Output = Result<TlsStream, TlsError>> + Send;

    /// Drive the same fingerprinted handshake over an already-established TLS stream — the origin-facing inner leg of an `https://` CONNECT proxy, where the origin TLS nests inside the client→proxy TLS.
    fn do_tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        include_alps: bool,
    ) -> impl std::future::Future<Output = Result<TlsStream, TlsError>> + Send;

    /// `true` when the connector carries an origin-specific TLS identity (client certificate / leaf pins) that must not be presented to, or applied against, an `https://` proxy.
    fn has_origin_tls_identity(&self) -> bool;

    /// Open a fingerprinted TCP connection to `host:port` through the connector's resolver + Happy-Eyeballs + `TcpProfile` path.
    fn dial_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> impl std::future::Future<Output = Result<tokio::net::TcpStream, TlsError>> + Send;
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

    async fn do_tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        self.tls_handshake_nested(inner, host, include_alps).await
    }

    fn has_origin_tls_identity(&self) -> bool {
        FingerprintConnector::has_origin_tls_identity(self)
    }

    async fn dial_tcp(&self, host: &str, port: u16) -> Result<tokio::net::TcpStream, TlsError> {
        FingerprintConnector::dial_tcp(self, host, port).await
    }
}
