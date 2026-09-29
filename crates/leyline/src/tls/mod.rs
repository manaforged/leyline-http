pub(crate) mod builder;
mod connector;
pub(crate) mod error;
mod happy_eyeballs;
mod hello;
mod keylog;
#[cfg(target_os = "macos")]
mod macos_trust;
mod nonblocking;
pub(crate) mod proxy;
mod resolver;
mod session_cache;
mod stream;
pub(crate) mod trust;
#[cfg(windows)]
mod windows_trust;

pub(crate) use error::TlsError;
pub use happy_eyeballs::HappyEyeballsConfig;
pub use resolver::{ResolveFuture, Resolver, SystemResolver};
pub(crate) use trust::TlsTrustConfig;

#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use connector::FingerprintConnector;
#[cfg(not(feature = "bench-internals"))]
pub(crate) use connector::FingerprintConnector;

#[cfg(any(feature = "http3", feature = "bench-internals", leyline_unstable_bssl))]
pub(crate) use builder::TlsMinVersion;
#[cfg(feature = "http3")]
pub(crate) use builder::apply_tls_with_trust;
#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
pub(crate) use builder::build_ssl_context;
#[cfg(feature = "http3")]
pub(crate) use hello::HelloOptions;
pub(crate) use session_cache::SessionCache;
pub(crate) use stream::TlsIo;
#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use stream::TlsStream;
#[cfg(not(feature = "bench-internals"))]
pub(crate) use stream::TlsStream;
#[cfg(feature = "http3")]
pub(crate) use trust::install_verifier_ctx;

#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
pub struct TlsContext(
    #[cfg_attr(
        not(leyline_unstable_bssl),
        expect(
            dead_code,
            reason = "read only through the leyline_unstable_bssl accessors"
        )
    )]
    leyline_bssl::ssl::SslContextBuilder,
);

#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
impl TlsContext {
    pub fn from_profile(
        profile: &crate::profile::BrowserProfile,
        min_version: TlsMinVersion,
    ) -> Result<Self, TlsError> {
        build_ssl_context(profile, min_version).map(Self)
    }

    #[cfg(leyline_unstable_bssl)]
    pub fn builder_mut(&mut self) -> &mut leyline_bssl::ssl::SslContextBuilder {
        &mut self.0
    }

    #[cfg(leyline_unstable_bssl)]
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
