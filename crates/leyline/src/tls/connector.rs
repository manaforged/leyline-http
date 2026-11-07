use std::num::NonZeroUsize;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use leyline_bssl::ssl::SslConnector;
use lru::LruCache;

use crate::deadline::{Elapsed, within};
use crate::net::{Dialer, NetError, ProxyUrl};
use crate::profile::BrowserProfile;
use crate::profile::TcpProfile;

use crate::profile::TlsMinVersion;
use crate::tls::AlpnPlan;
use crate::tls::HelloOptions;
use crate::tls::TlsStream;
use crate::tls::builder::{Transport, apply_tls_with_trust};
use crate::tls::error::TlsError;
use crate::tls::proxy::{self, Route, Tunnel};
use crate::tls::session_cache::SessionCache;
use crate::tls::trust::{Tls, TrustIdentity, TrustKey};
use crate::util::lock;

mod handshake;
#[cfg(test)]
mod shared_tests;

const TCP: Transport = Transport::Tcp(TlsMinVersion::Tls12);

#[derive(Clone)]
pub struct FingerprintConnector {
    ssl_connector: SslConnector,
    trust: Tls,
    hello: HelloOptions,
    session_cache: SessionCache,
    dialer: Dialer,
    connect_timeout: Option<Duration>,
}

const SHARED_CONTEXTS: NonZeroUsize = match NonZeroUsize::new(8) {
    Some(n) => n,
    None => NonZeroUsize::MIN,
};

static PLAIN_CONTEXTS: LazyLock<Mutex<LruCache<TrustKey, Context>>> =
    LazyLock::new(|| Mutex::new(LruCache::new(SHARED_CONTEXTS)));

#[derive(Clone)]
struct Context {
    ssl_connector: SslConnector,
    identity: TrustIdentity,
    hello: HelloOptions,
}

impl Context {
    fn build(profile: &BrowserProfile, trust: &Tls) -> Result<Self, TlsError> {
        let mut builder = SslConnector::bare_builder(leyline_bssl::ssl::SslMethod::tls())
            .map_err(TlsError::from_stack)?;
        let identity = apply_tls_with_trust(&mut builder, &profile.tls, TCP, trust)?;
        let hello = HelloOptions::from_tls(&profile.tls)?;
        builder
            .set_alpn_protos(crate::tls::alpn::BROWSER_WIRE)
            .map_err(TlsError::from_stack)?;
        SessionCache::register(&mut builder)?;
        Ok(Self {
            ssl_connector: builder.build(),
            identity,
            hello,
        })
    }

    fn shared(profile: &BrowserProfile, trust: &Tls) -> Result<Self, TlsError> {
        let key = std::ptr::eq(profile, &*BrowserProfile::bare_shared())
            .then(|| trust.shared_key())
            .flatten();
        let Some(key) = key else {
            return Self::build(profile, trust);
        };
        if let Some(context) = lock(&*PLAIN_CONTEXTS).get(&key) {
            return Ok(context.clone());
        }
        let context = Self::build(profile, trust)?;
        lock(&*PLAIN_CONTEXTS).put(key, context.clone());
        Ok(context)
    }
}

impl FingerprintConnector {
    #[cfg(feature = "bench-internals")]
    pub fn new(profile: &BrowserProfile, tcp: TcpProfile) -> Result<Self, TlsError> {
        Self::new_with_trust(profile, tcp, &Tls::default())
    }

    #[cfg(any(test, feature = "bench-internals"))]
    pub fn new_with_trust(
        profile: &BrowserProfile,
        tcp: TcpProfile,
        trust: &Tls,
    ) -> Result<Self, TlsError> {
        Self::with_dialer(profile, Dialer::new(tcp), trust)
    }

    pub(crate) fn with_dialer(
        profile: &BrowserProfile,
        dialer: Dialer,
        trust: &Tls,
    ) -> Result<Self, TlsError> {
        let context = Context::shared(profile, trust)?;
        Ok(Self {
            ssl_connector: context.ssl_connector,
            hello: context.hello,
            session_cache: SessionCache::new(context.identity),
            dialer,
            connect_timeout: None,
            trust: trust.clone(),
        })
    }

    pub(crate) fn with_profile(
        &self,
        profile: &BrowserProfile,
        tcp: TcpProfile,
    ) -> Result<Self, TlsError> {
        let mut next = Self::with_dialer(profile, self.dialer.clone().with_tcp(tcp), &self.trust)?;
        next.connect_timeout = self.connect_timeout;
        Ok(next)
    }

    pub(crate) fn session_cache(&self) -> &SessionCache {
        &self.session_cache
    }

    pub(crate) fn dialer(&self) -> &Dialer {
        &self.dialer
    }

    pub(crate) fn has_origin_tls_identity(&self) -> bool {
        !self.trust.pinned_leaf_sha256().is_empty() || self.trust.has_client_identity()
    }

    #[cfg(feature = "bench-internals")]
    pub fn with_resolver(mut self, resolver: std::sync::Arc<dyn crate::net::Resolver>) -> Self {
        self.dialer = self.dialer.with_resolver(resolver);
        self
    }

    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    pub async fn connect(
        &self,
        host: &str,
        port: u16,
        proxy: Option<&ProxyUrl>,
        plan: AlpnPlan,
    ) -> Result<TlsStream, TlsError> {
        match proxy {
            Some(proxy) => self.connect_via_proxy(host, port, proxy, plan).await,
            None => {
                self.with_timeout(self.connect_direct(host, port, plan))
                    .await
            }
        }
    }

    async fn connect_via_proxy(
        &self,
        host: &str,
        port: u16,
        proxy: &ProxyUrl,
        plan: AlpnPlan,
    ) -> Result<TlsStream, TlsError> {
        let started = Instant::now();
        let tunnel = self
            .with_timeout(proxy::open(self, host, port, proxy, Route::Connect))
            .await
            .map_err(TlsError::into_proxy)?;
        let remaining = self
            .connect_timeout
            .map(|budget| budget.saturating_sub(started.elapsed()));
        let session_key = SessionCache::key(host, port, Some(proxy.url()));
        let handshake = async {
            match tunnel {
                Tunnel::Plain(tcp) => self.tls_handshake(tcp, host, &session_key, plan).await,
                Tunnel::Tls(io) => {
                    self.tls_handshake_nested(io, host, &session_key, plan)
                        .await
                }
            }
        };
        within(remaining, handshake)
            .await
            .map_err(|Elapsed| TlsError::from(NetError::connect_timeout()))?
    }

    pub(crate) async fn with_timeout<T, E, F>(&self, fut: F) -> Result<T, E>
    where
        E: From<NetError>,
        F: std::future::Future<Output = Result<T, E>>,
    {
        within(self.connect_timeout, fut)
            .await
            .map_err(|Elapsed| E::from(NetError::connect_timeout()))?
    }

    async fn connect_direct(
        &self,
        host: &str,
        port: u16,
        plan: AlpnPlan,
    ) -> Result<TlsStream, TlsError> {
        let tcp_stream = self.dialer.dial(host, port).await?;
        let session_key = SessionCache::key(host, port, None);
        self.tls_handshake(tcp_stream, host, &session_key, plan)
            .await
    }
}

impl std::fmt::Debug for FingerprintConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FingerprintConnector")
            .field("tcp_profile", self.dialer.tcp_profile())
            .field("ech_grease", &self.hello.ech_grease())
            .finish_non_exhaustive()
    }
}
