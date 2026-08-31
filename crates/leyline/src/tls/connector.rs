//! TLS connector that creates fingerprinted connections from browser profiles.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use leyline_bssl::ssl::{NameType, SslConnector, SslSession, SslSessionCacheMode, SslVerifyMode};
use leyline_bssl::x509::X509VerifyError;
use lru::LruCache;
use tokio::net::TcpStream;

use crate::core::SocketConfig;
use crate::profile::BrowserProfile;
use crate::tcp::TcpProfile;

use crate::tls::builder::{TlsMinVersion, apply_profile_with_trust};
use crate::tls::error::TlsError;
use crate::tls::happy_eyeballs::{HappyEyeballsConfig, happy_eyeballs_connect};
use crate::tls::nonblocking::connect_one;
use crate::tls::resolver::{Resolver, SystemResolver};
use crate::tls::trust::{
    TlsTrustConfig, TrustFailure, VerificationFailure, install_pinning_verifier,
    take_verification_failure,
};
use crate::tls::{TlsIo, TlsStream};

/// Creates TLS connections matching a browser's fingerprint.
#[derive(Clone)]
pub struct FingerprintConnector {
    ssl_connector: SslConnector,
    tcp_profile: TcpProfile,
    ech_grease: bool,
    /// ALPS protocol (e.g. "h2") — applied per-connection.
    alps_proto: Option<Vec<u8>>,
    /// Use new ALPS codepoint (0x4469 for Chrome 131+).
    alps_new_codepoint: bool,
    /// Advertise Trust Anchor Identifiers (ext 0xCA34/51764) with an empty list when the selected browser profile requires it.
    request_trust_anchors: bool,
    /// Per-host session ticket cache for TLS resumption (DER-encoded).
    session_cache: Arc<Mutex<LruCache<String, Vec<u8>>>>,
    /// When `true`, skip peer certificate verification entirely.
    accept_invalid_certs: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// DNS resolver for direct connections.
    resolver: Arc<dyn Resolver>,
    /// Happy Eyeballs (RFC 8305) tunables for the dual-stack race.
    happy_eyeballs: HappyEyeballsConfig,
    /// Optional timeout around DNS + TCP + TLS connect.
    connect_timeout: Option<Duration>,
    /// Optional socket-level direct-connect overrides.
    socket_config: SocketConfig,
    /// Configured leaf pins.
    pins: Vec<[u8; 32]>,
    /// `true` when a client certificate (mTLS identity) is configured.
    has_client_identity: bool,
}

impl FingerprintConnector {
    /// Build a connector from a browser profile and TCP profile.
    pub fn new(profile: &BrowserProfile, tcp: TcpProfile) -> Result<Self, TlsError> {
        Self::new_with_trust(profile, tcp, &TlsTrustConfig::default())
    }

    /// Build a connector with explicit trust-root and mTLS settings.
    pub fn new_with_trust(
        profile: &BrowserProfile,
        tcp: TcpProfile,
        trust: &TlsTrustConfig,
    ) -> Result<Self, TlsError> {
        let trust = trust.clone();
        let mut builder = SslConnector::bare_builder(leyline_bssl::ssl::SslMethod::tls())?;

        apply_profile_with_trust(&mut builder, profile, TlsMinVersion::Tls12, &trust)?;

        let tls = &profile.tls;

        builder.set_alpn_protos(b"\x02h2\x08http/1.1")?;

        let session_cache = Arc::new(Mutex::new(LruCache::new(
            std::num::NonZeroUsize::new(256).expect("cache capacity literal is non-zero"),
        )));
        builder
            .set_session_cache_mode(SslSessionCacheMode::CLIENT | SslSessionCacheMode::NO_INTERNAL);
        let cache_clone = session_cache.clone();
        let accept_invalid_certs = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let insecure_flag = accept_invalid_certs.clone();
        builder.set_new_session_callback(move |ssl, session| {
            if insecure_flag.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            if let Some(hostname) = ssl.servername(NameType::HOST_NAME) {
                if let Ok(der) = session.to_der() {
                    lock_unpoisoned(&cache_clone).put(hostname.to_string(), der);
                }
            }
        });

        Ok(Self {
            ssl_connector: builder.build(),
            tcp_profile: tcp,
            ech_grease: tls.ech_grease,
            alps_proto: tls.alps.as_ref().map(|s| s.as_bytes().to_vec()),
            alps_new_codepoint: tls.alps_new_codepoint,
            request_trust_anchors: tls.request_trust_anchors,
            session_cache,
            accept_invalid_certs,
            resolver: Arc::new(SystemResolver),
            happy_eyeballs: HappyEyeballsConfig::default(),
            connect_timeout: None,
            socket_config: SocketConfig::default(),
            pins: trust.pinned_leaf_sha256().to_vec(),
            has_client_identity: trust.client_identity().is_some(),
        })
    }

    /// `true` when this connector carries an origin-specific TLS identity — a client certificate (mTLS) or leaf pins — that must NOT be presented to, or applied against, an `https://` CONNECT proxy.
    pub(crate) fn has_origin_tls_identity(&self) -> bool {
        !self.pins.is_empty() || self.has_client_identity
    }

    /// Skip peer certificate verification.
    pub fn set_accept_invalid_certs(&mut self, accept: bool) {
        use std::sync::atomic::Ordering;
        self.accept_invalid_certs.store(accept, Ordering::Relaxed);
    }

    /// Live insecure-mode flag, shared with the session-ticket callback so tickets minted while verification was off are never cached or reused.
    fn insecure_mode(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.accept_invalid_certs.load(Ordering::Relaxed)
    }

    /// Override the DNS resolver (for `/etc/hosts`-style tests or deterministic offline rigs).
    pub fn with_resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// Override the Happy-Eyeballs tunables.
    pub fn with_happy_eyeballs_config(mut self, config: HappyEyeballsConfig) -> Self {
        self.happy_eyeballs = config;
        self
    }

    /// Apply a timeout around DNS + TCP + TLS connection setup.
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    /// Apply low-level socket options for direct TCP connects.
    pub fn with_socket_config(mut self, config: SocketConfig) -> Self {
        self.socket_config = config;
        self
    }

    /// Connect to `host:port`, optionally through `proxy_url`.
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

    /// Connect with HTTP/1.1 ALPN only (for WebSocket upgrade).
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

    async fn with_timeout<F>(&self, fut: F) -> Result<TlsStream, TlsError>
    where
        F: std::future::Future<Output = Result<TlsStream, TlsError>>,
    {
        match self.connect_timeout {
            Some(timeout) => tokio::time::timeout(timeout, fut).await.map_err(|_| {
                TlsError::TcpConnect(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "connect timeout",
                ))
            })?,
            None => fut.await,
        }
    }

    /// Direct connection with optional ALPN override.
    async fn connect_direct_with_alpn(
        &self,
        host: &str,
        port: u16,
        alpn_override: Option<&[u8]>,
    ) -> Result<TlsStream, TlsError> {
        let tcp_stream = self.dial_tcp(host, port).await?;
        self.tls_handshake(tcp_stream, host, alpn_override.is_none())
            .await
    }

    /// Open a fingerprinted TCP connection to `host:port` through the connector's pluggable resolver and Happy-Eyeballs racer, applying the browser [`TcpProfile`] SYN options via [`connect_one`].
    pub(crate) async fn dial_tcp(&self, host: &str, port: u16) -> Result<TcpStream, TlsError> {
        let addrs = self
            .resolver
            .resolve(host, port)
            .await
            .map_err(TlsError::Dns)?;

        if addrs.is_empty() {
            return Err(TlsError::Dns(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no addresses resolved",
            )));
        }

        let tcp_profile = self.tcp_profile;
        let socket_config = self.socket_config.clone();
        let (tcp_stream, _addr) =
            happy_eyeballs_connect(addrs, self.happy_eyeballs, move |sock_addr| {
                let socket_config = socket_config.clone();
                async move { connect_one(sock_addr, &tcp_profile, &socket_config).await }
            })
            .await
            .map_err(TlsError::TcpConnect)?;
        Ok(tcp_stream)
    }

    /// Perform TLS handshake with all per-connection fingerprint settings.
    pub(crate) async fn tls_handshake(
        &self,
        tcp_stream: TcpStream,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let (stream, meta) = self.handshake_over(tcp_stream, host, include_alps).await?;
        Ok(meta.into_tls_stream(TlsIo::Boring(stream)))
    }

    /// Drive the same fingerprinted TLS handshake over an already-established TLS stream — the inner (origin-facing) leg of an `https://` CONNECT proxy, so the origin TLS nests inside the proxy TLS.
    pub(crate) async fn tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let (stream, meta) = self.handshake_over(inner, host, include_alps).await?;
        Ok(meta.into_tls_stream(TlsIo::Nested(Box::new(stream))))
    }

    /// The fingerprint-bearing TLS handshake, generic over the byte stream so the direct path (`TcpStream`) and the `https://`-proxy inner leg (`TlsIo`) share ONE ClientHello — the single source of truth for the JA4.
    async fn handshake_over<S>(
        &self,
        io: S,
        host: &str,
        include_alps: bool,
    ) -> Result<(leyline_bssl_tokio::SslStream<S>, TlsMeta), TlsError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let mut config = self.ssl_connector.configure()?;

        if include_alps {
            if let Some(ref alps) = self.alps_proto {
                config.add_application_settings(alps)?;
                if self.alps_new_codepoint {
                    config.set_alps_use_new_codepoint(true);
                }
            }
        } else {
            config.set_alpn_protos(b"\x08http/1.1")?;
        }

        let mut ssl = config.into_ssl(host)?;

        let insecure = self.insecure_mode();
        if insecure {
            ssl.set_verify(SslVerifyMode::NONE);
        }
        let verification_failure = (!insecure && !self.pins.is_empty())
            .then(|| install_pinning_verifier(&mut ssl, &self.pins));

        {
            let mut cache = lock_unpoisoned(&self.session_cache);
            if let Some(der) = (!insecure).then(|| cache.get(host).cloned()).flatten() {
                if let Ok(session) = SslSession::from_der(&der) {
                    // SAFETY: BoringSSL requires `set_session` to be called on an Ssl not yet handed to `connect()`. `ssl` was just constructed via `config.into_ssl` and has not started its handshake; it will be driven via `leyline_bssl_tokio::connect` below. The `SslSession` is owned for the duration of this block. No concurrent access.
                    unsafe {
                        let _ = ssl.set_session(&session);
                    }
                }
            }
        }

        if self.ech_grease {
            ssl.set_enable_ech_grease(true);
        }

        if self.request_trust_anchors {
            ssl.set_requested_trust_anchors(&[]).map_err(|e| {
                TlsError::SslConfig(format!(
                    "profile requires the trust_anchors extension (0xCA34), which BoringSSL \
                     rejected ({e}); the ClientHello JA4 would not match the captured browser"
                ))
            })?;
        }

        let mut stream = leyline_bssl_tokio::SslStream::new(ssl, io)
            .map_err(|e| TlsError::SslConfig(e.to_string()))?;
        if let Err(e) = std::pin::Pin::new(&mut stream).connect().await {
            return Err(classify_handshake(
                verification_failure.as_ref(),
                stream.ssl().verify_result().err(),
                e,
            ));
        }

        if !self.pins.is_empty() && !insecure {
            let leaf = stream.ssl().peer_certificate().ok_or_else(|| {
                TlsError::Certificate("pinned connection presented no peer certificate".into())
            })?;
            let matches = match host.parse::<std::net::IpAddr>() {
                Ok(_) => leaf.check_ip_asc(host),
                Err(_) => leaf.check_host(host),
            }
            .map_err(|e| TlsError::Hostname(format!("hostname check failed: {e}")))?;
            if !matches {
                return Err(TlsError::Hostname(format!(
                    "certificate is valid and pinned but does not match {host}"
                )));
            }
        }

        let alpn = stream.ssl().selected_alpn_protocol().map(|p| p.to_vec());
        let peer_cert_der = stream
            .ssl()
            .peer_certificate()
            .and_then(|cert| cert.to_der().ok());
        let tls_version = Some(stream.ssl().version_str().to_string());
        let tls_cipher = stream.ssl().current_cipher().map(|c| c.name().to_string());

        Ok((
            stream,
            TlsMeta {
                alpn,
                peer_cert_der,
                tls_version,
                tls_cipher,
            },
        ))
    }
}

fn classify_handshake(
    failure: Option<&VerificationFailure>,
    verify_error: Option<X509VerifyError>,
    error: leyline_bssl::ssl::Error,
) -> TlsError {
    let verify_error = verify_error.filter(|error| *error != X509VerifyError::INVALID_CALL);
    match failure.and_then(take_verification_failure) {
        Some(TrustFailure::Certificate) => TlsError::Certificate(error.to_string()),
        Some(TrustFailure::Pinning) => TlsError::Pinning(error.to_string()),
        None if matches!(
            verify_error,
            Some(X509VerifyError::HOSTNAME_MISMATCH | X509VerifyError::IP_ADDRESS_MISMATCH)
        ) =>
        {
            TlsError::Hostname(error.to_string())
        }
        None if verify_error.is_some() => TlsError::Certificate(error.to_string()),
        None => super::error::from_handshake_ssl(error),
    }
}

/// Negotiated TLS metadata captured at handshake, paired with the handshaked stream so [`FingerprintConnector::handshake_over`] can stay generic over the byte stream while each caller wraps the stream in its own [`TlsIo`] arm.
struct TlsMeta {
    alpn: Option<Vec<u8>>,
    peer_cert_der: Option<Vec<u8>>,
    tls_version: Option<String>,
    tls_cipher: Option<String>,
}

impl TlsMeta {
    fn into_tls_stream(self, stream: TlsIo) -> TlsStream {
        TlsStream {
            stream,
            alpn: self.alpn,
            peer_cert_der: self.peer_cert_der,
            tls_version: self.tls_version,
            tls_cipher: self.tls_cipher,
        }
    }
}

impl std::fmt::Debug for FingerprintConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FingerprintConnector")
            .field("tcp_profile", &self.tcp_profile)
            .field("ech_grease", &self.ech_grease)
            .finish_non_exhaustive()
    }
}

/// Lock the session-ticket cache, recovering from a poisoned mutex.
fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::warn!(
            target: "leyline::tls",
            "session cache mutex was poisoned; recovering"
        );
        poisoned.into_inner()
    })
}

#[cfg(test)]
mod tests;
