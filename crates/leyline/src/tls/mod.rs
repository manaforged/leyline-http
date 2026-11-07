pub(crate) mod alpn;
pub(crate) mod builder;
mod connector;
pub(crate) mod error;
mod hello;
mod keylog;
#[cfg(target_os = "macos")]
mod macos_trust;
pub(crate) mod proxy;
mod session_cache;
mod stream;
pub(crate) mod trust;
#[cfg(windows)]
mod windows_trust;

pub use alpn::AlpnPlan;

pub use connector::FingerprintConnector;

pub(crate) use hello::HelloOptions;
pub(crate) use stream::TlsIo;
pub use stream::TlsStream;
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
        min_version: crate::profile::TlsMinVersion,
    ) -> Result<Self, error::TlsError> {
        builder::ssl_context(profile, min_version).map(Self)
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
