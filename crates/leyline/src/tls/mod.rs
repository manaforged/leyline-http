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

/// The BoringSSL context a browser profile builds, outside the semver promise.
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
impl std::fmt::Debug for TlsContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsContext").finish_non_exhaustive()
    }
}

#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
impl TlsContext {
    /// Builds the TLS context `browser` uses, with `min_version` as the floor when its profile declares none.
    pub fn new(
        browser: crate::Browser,
        min_version: crate::profile::TlsMinVersion,
    ) -> crate::Result<Self> {
        Ok(Self(builder::ssl_context(browser.profile(), min_version)?))
    }

    #[cfg(feature = "bench-internals")]
    pub(crate) fn from_builder(builder: leyline_bssl::ssl::SslContextBuilder) -> Self {
        Self(builder)
    }

    /// Returns the BoringSSL builder to change before it is used.
    #[cfg(leyline_unstable_bssl)]
    pub fn builder_mut(&mut self) -> &mut leyline_bssl::ssl::SslContextBuilder {
        &mut self.0
    }

    /// Returns the BoringSSL builder.
    #[cfg(leyline_unstable_bssl)]
    pub fn into_inner(self) -> leyline_bssl::ssl::SslContextBuilder {
        self.0
    }
}
