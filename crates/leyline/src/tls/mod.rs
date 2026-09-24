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
mod session_cache;
mod stream;
mod trust;
#[cfg(windows)]
mod windows_trust;

pub use builder::TlsMinVersion;
pub use error::TlsError;
pub use happy_eyeballs::HappyEyeballsConfig;
pub use resolver::{ResolveFuture, Resolver, SystemResolver};
pub use trust::{ClientIdentity, TlsTrustConfig};

#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use connector::FingerprintConnector;
#[cfg(not(feature = "bench-internals"))]
pub(crate) use connector::FingerprintConnector;

pub(crate) use builder::apply_profile_with_trust;
#[cfg(any(feature = "unstable-bssl", feature = "bench-internals"))]
pub(crate) use builder::build_ssl_context;
pub(crate) use session_cache::SessionCache;
pub(crate) use stream::TlsIo;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use stream::TlsStream;
#[cfg(not(feature = "bench-internals"))]
pub(crate) use stream::TlsStream;
pub(crate) use trust::install_verifier_ctx;

#[cfg(any(feature = "unstable-bssl", feature = "bench-internals"))]
pub struct TlsContext(
    #[cfg_attr(
        not(feature = "unstable-bssl"),
        expect(dead_code, reason = "read only through the unstable-bssl accessors")
    )]
    leyline_bssl::ssl::SslContextBuilder,
);

#[cfg(any(feature = "unstable-bssl", feature = "bench-internals"))]
impl TlsContext {
    pub fn from_profile(
        profile: &crate::profile::BrowserProfile,
        min_version: TlsMinVersion,
    ) -> Result<Self, TlsError> {
        build_ssl_context(profile, min_version).map(Self)
    }

    #[cfg(feature = "unstable-bssl")]
    pub fn builder_mut(&mut self) -> &mut leyline_bssl::ssl::SslContextBuilder {
        &mut self.0
    }

    #[cfg(feature = "unstable-bssl")]
    pub fn into_inner(self) -> leyline_bssl::ssl::SslContextBuilder {
        self.0
    }
}

pub(crate) trait TlsHandshake {
    fn do_tls_handshake(
        &self,
        tcp_stream: tokio::net::TcpStream,
        host: &str,
        session_key: &str,
        include_alps: bool,
    ) -> impl std::future::Future<Output = Result<TlsStream, TlsError>> + Send;

    fn do_tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        session_key: &str,
        include_alps: bool,
    ) -> impl std::future::Future<Output = Result<TlsStream, TlsError>> + Send;

    fn has_origin_tls_identity(&self) -> bool;

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
        session_key: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        self.tls_handshake(tcp_stream, host, session_key, include_alps)
            .await
    }

    async fn do_tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        session_key: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        self.tls_handshake_nested(inner, host, session_key, include_alps)
            .await
    }

    fn has_origin_tls_identity(&self) -> bool {
        FingerprintConnector::has_origin_tls_identity(self)
    }

    async fn dial_tcp(&self, host: &str, port: u16) -> Result<tokio::net::TcpStream, TlsError> {
        FingerprintConnector::dial_tcp(self, host, port).await
    }
}
