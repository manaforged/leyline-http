use std::sync::Arc;
use std::time::{Duration, Instant};

use leyline_bssl::ssl::SslConnector;
use tokio::net::TcpStream;

use crate::core::SocketConfig;
use crate::core::deadline::{Elapsed, within};
use crate::profile::BrowserProfile;
use crate::tcp::TcpProfile;

use crate::tls::TlsStream;
use crate::tls::builder::{TlsMinVersion, apply_profile_with_trust};
use crate::tls::error::TlsError;
use crate::tls::happy_eyeballs::{HappyEyeballsConfig, happy_eyeballs_connect};
use crate::tls::hello::HelloOptions;
use crate::tls::nonblocking::connect_one;
use crate::tls::resolver::{Resolver, SystemResolver};
use crate::tls::session_cache::SessionCache;
use crate::tls::trust::TlsTrustConfig;
use crate::trace;

mod handshake;

#[derive(Clone)]
pub struct FingerprintConnector {
    ssl_connector: SslConnector,
    tcp_profile: TcpProfile,
    trust: TlsTrustConfig,
    hello: HelloOptions,
    session_cache: SessionCache,
    accept_invalid_certs: std::sync::Arc<std::sync::atomic::AtomicBool>,
    resolver: Arc<dyn Resolver>,
    happy_eyeballs: HappyEyeballsConfig,
    connect_timeout: Option<Duration>,
    socket_config: SocketConfig,
    pins: Vec<[u8; 32]>,
    system_roots: bool,
    has_client_identity: bool,
}

impl FingerprintConnector {
    #[cfg(any(test, feature = "bench-internals"))]
    pub fn new(profile: &BrowserProfile, tcp: TcpProfile) -> Result<Self, TlsError> {
        Self::new_with_trust(profile, tcp, &TlsTrustConfig::default())
    }

    pub fn new_with_trust(
        profile: &BrowserProfile,
        tcp: TcpProfile,
        trust: &TlsTrustConfig,
    ) -> Result<Self, TlsError> {
        let trust = trust.clone();
        let mut builder = SslConnector::bare_builder(leyline_bssl::ssl::SslMethod::tls())
            .map_err(TlsError::from_stack)?;

        apply_profile_with_trust(&mut builder, profile, TlsMinVersion::Tls12, &trust)?;

        let hello = HelloOptions::from_tls(&profile.tls)?;

        builder
            .set_alpn_protos(b"\x02h2\x08http/1.1")
            .map_err(TlsError::from_stack)?;

        SessionCache::register(&mut builder)?;
        let accept_invalid_certs = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        Ok(Self {
            ssl_connector: builder.build(),
            tcp_profile: tcp,
            hello,
            session_cache: SessionCache::new(),
            accept_invalid_certs,
            resolver: Arc::new(SystemResolver),
            happy_eyeballs: HappyEyeballsConfig::default(),
            connect_timeout: None,
            socket_config: SocketConfig::default(),
            pins: trust.pinned_leaf_sha256().to_vec(),
            system_roots: trust.uses_system_roots(),
            has_client_identity: trust.has_client_identity(),
            trust,
        })
    }

    pub(crate) fn with_fresh_session_cache(&self) -> Self {
        Self {
            session_cache: SessionCache::new(),
            ..self.clone()
        }
    }

    pub(crate) fn with_profile(&self, profile: &BrowserProfile) -> Result<Self, TlsError> {
        use std::sync::atomic::Ordering;
        let mut next = Self::new_with_trust(profile, self.tcp_profile.clone(), &self.trust)?;
        next.resolver = self.resolver.clone();
        next.happy_eyeballs = self.happy_eyeballs;
        next.connect_timeout = self.connect_timeout;
        next.socket_config = self.socket_config.clone();
        if self.accept_invalid_certs.load(Ordering::Relaxed) {
            next.set_accept_invalid_certs(true);
        }
        Ok(next)
    }

    #[cfg(feature = "http3")]
    pub(crate) fn resolver(&self) -> &Arc<dyn Resolver> {
        &self.resolver
    }

    pub(crate) fn has_origin_tls_identity(&self) -> bool {
        !self.pins.is_empty() || self.has_client_identity
    }

    pub fn set_accept_invalid_certs(&mut self, accept: bool) {
        use std::sync::atomic::Ordering;
        self.accept_invalid_certs.store(accept, Ordering::Relaxed);
    }

    fn insecure_mode(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.accept_invalid_certs.load(Ordering::Relaxed)
    }

    pub fn with_resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    pub fn with_happy_eyeballs_config(mut self, config: HappyEyeballsConfig) -> Self {
        self.happy_eyeballs = config;
        self
    }

    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    pub fn with_socket_config(mut self, config: SocketConfig) -> Self {
        self.socket_config = config;
        self
    }

    pub async fn connect(
        &self,
        host: &str,
        port: u16,
        proxy: Option<&str>,
    ) -> Result<TlsStream, TlsError> {
        let fut = async {
            match proxy {
                Some(proxy_url) => {
                    crate::tls::proxy::connect_through_proxy(self, host, port, proxy_url, true)
                        .await
                }
                None => self.connect_direct_with_alpn(host, port, None).await,
            }
        };
        self.with_timeout(fut).await
    }

    pub async fn connect_h1(
        &self,
        host: &str,
        port: u16,
        proxy: Option<&str>,
    ) -> Result<TlsStream, TlsError> {
        let fut = async {
            match proxy {
                Some(proxy_url) => {
                    crate::tls::proxy::connect_through_proxy(self, host, port, proxy_url, false)
                        .await
                }
                None => {
                    self.connect_direct_with_alpn(host, port, Some(b"\x08http/1.1"))
                        .await
                }
            }
        };
        self.with_timeout(fut).await
    }

    pub(crate) fn tcp_profile(&self) -> &TcpProfile {
        &self.tcp_profile
    }

    pub(crate) async fn with_timeout<T, F>(&self, fut: F) -> Result<T, TlsError>
    where
        F: std::future::Future<Output = Result<T, TlsError>>,
    {
        within(self.connect_timeout, fut).await.map_err(|Elapsed| {
            TlsError::TcpConnect(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "connect timeout",
            ))
        })?
    }

    async fn connect_direct_with_alpn(
        &self,
        host: &str,
        port: u16,
        alpn_override: Option<&[u8]>,
    ) -> Result<TlsStream, TlsError> {
        let tcp_stream = self.dial_tcp(host, port).await?;
        let session_key = SessionCache::key(host, port, None);
        self.tls_handshake(tcp_stream, host, &session_key, alpn_override.is_none())
            .await
    }

    pub(crate) async fn dial_tcp(&self, host: &str, port: u16) -> Result<TcpStream, TlsError> {
        let started = Instant::now();
        let addrs = self
            .resolver
            .resolve(host, port)
            .await
            .map_err(TlsError::Dns)?;
        trace::dns(host, port, addrs.len(), started.elapsed());

        if addrs.is_empty() {
            return Err(TlsError::Dns(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no addresses resolved",
            )));
        }

        let tcp_profile = self.tcp_profile.clone();
        let socket_config = self.socket_config.clone();
        let started = Instant::now();
        let (tcp_stream, _addr) =
            happy_eyeballs_connect(addrs, self.happy_eyeballs, move |sock_addr| {
                let socket_config = socket_config.clone();
                let tcp_profile = tcp_profile.clone();
                async move { connect_one(sock_addr, &tcp_profile, &socket_config).await }
            })
            .await
            .map_err(TlsError::TcpConnect)?;
        trace::connect(host, port, false, started.elapsed());
        Ok(tcp_stream)
    }
}

impl std::fmt::Debug for FingerprintConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FingerprintConnector")
            .field("tcp_profile", &self.tcp_profile)
            .field("ech_grease", &self.hello.ech_grease())
            .finish_non_exhaustive()
    }
}
