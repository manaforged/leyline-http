use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    TlsTrustConfig, TrustFailure, VerificationFailure, install_verifier, take_verification_failure,
};
use crate::tls::{TlsIo, TlsStream};
use crate::trace;

#[derive(Clone)]
pub struct FingerprintConnector {
    ssl_connector: SslConnector,
    tcp_profile: TcpProfile,
    ech_grease: bool,
    alps_proto: Option<Vec<u8>>,
    alps_new_codepoint: bool,
    request_trust_anchors: bool,
    session_cache: Arc<Mutex<LruCache<String, Vec<u8>>>>,
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

        let tls = &profile.tls;

        builder
            .set_alpn_protos(b"\x02h2\x08http/1.1")
            .map_err(TlsError::from_stack)?;

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
            if let Some(hostname) = ssl.servername(NameType::HOST_NAME)
                && let Ok(der) = session.to_der()
            {
                lock_unpoisoned(&cache_clone).put(hostname.to_string(), der);
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
            system_roots: trust.uses_system_roots(),
            has_client_identity: trust.client_identity().is_some(),
        })
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

        let tcp_profile = self.tcp_profile;
        let socket_config = self.socket_config.clone();
        let started = Instant::now();
        let (tcp_stream, _addr) =
            happy_eyeballs_connect(addrs, self.happy_eyeballs, move |sock_addr| {
                let socket_config = socket_config.clone();
                async move { connect_one(sock_addr, &tcp_profile, &socket_config).await }
            })
            .await
            .map_err(TlsError::TcpConnect)?;
        trace::connect(host, port, false, started.elapsed());
        Ok(tcp_stream)
    }

    pub(crate) async fn tls_handshake(
        &self,
        tcp_stream: TcpStream,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let (stream, meta) = self.handshake_over(tcp_stream, host, include_alps).await?;
        Ok(meta.into_tls_stream(TlsIo::Boring(stream)))
    }

    pub(crate) async fn tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let (stream, meta) = self.handshake_over(inner, host, include_alps).await?;
        Ok(meta.into_tls_stream(TlsIo::Nested(Box::new(stream))))
    }

    async fn handshake_over<S>(
        &self,
        io: S,
        host: &str,
        include_alps: bool,
    ) -> Result<(leyline_bssl_tokio::SslStream<S>, TlsMeta), TlsError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let started = Instant::now();
        let mut config = self
            .ssl_connector
            .configure()
            .map_err(TlsError::from_stack)?;

        if include_alps {
            if let Some(ref alps) = self.alps_proto {
                config
                    .add_application_settings(alps)
                    .map_err(TlsError::from_stack)?;
                if self.alps_new_codepoint {
                    config.set_alps_use_new_codepoint(true);
                }
            }
        } else {
            config
                .set_alpn_protos(b"\x08http/1.1")
                .map_err(TlsError::from_stack)?;
        }

        let mut ssl = config.into_ssl(host).map_err(TlsError::from_stack)?;

        let insecure = self.insecure_mode();
        if insecure {
            ssl.set_verify(SslVerifyMode::NONE);
        }
        let verification_failure = (!insecure
            && (!self.pins.is_empty() || cfg!(target_os = "macos") && self.system_roots))
            .then(|| install_verifier(&mut ssl, &self.pins, host, self.system_roots));

        {
            let mut cache = lock_unpoisoned(&self.session_cache);
            if let Some(der) = (!insecure).then(|| cache.get(host).cloned()).flatten()
                && let Ok(session) = SslSession::from_der(&der)
            {
                // SAFETY: BoringSSL requires `set_session` to be called on an Ssl not yet handed to `connect()`. `ssl` was just constructed via `config.into_ssl` and has not started its handshake; it will be driven via `leyline_bssl_tokio::connect` below. The `SslSession` is owned for the duration of this block. No concurrent access.
                unsafe {
                    let _ = ssl.set_session(&session);
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

        let alpn = stream.ssl().selected_alpn_protocol().map(|p| p.to_vec());
        let peer_cert_der = stream
            .ssl()
            .peer_certificate()
            .and_then(|cert| cert.to_der().ok());
        let tls_version = Some(stream.ssl().version_str().to_string());
        let tls_cipher = stream.ssl().current_cipher().map(|c| c.name().to_string());

        if trace::on() {
            let proto = alpn
                .as_deref()
                .map(|p| String::from_utf8_lossy(p).into_owned());
            trace::tls(
                host,
                tls_version.as_deref(),
                tls_cipher.as_deref(),
                proto.as_deref(),
                started.elapsed(),
            );
        }

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
        Some(TrustFailure::Hostname) => TlsError::Hostname(error.to_string()),
        Some(TrustFailure::Pinning) => TlsError::Pinning(error.to_string()),
        None if matches!(
            verify_error,
            Some(X509VerifyError::HOSTNAME_MISMATCH | X509VerifyError::IP_ADDRESS_MISMATCH)
        ) =>
        {
            TlsError::Hostname(error.to_string())
        }
        None if verify_error.is_some() => TlsError::Certificate(error.to_string()),
        None => TlsError::from_ssl(error),
    }
}

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
