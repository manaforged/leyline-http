//! TLS connector that creates fingerprinted connections from browser profiles.
//!
//! The connector is the single hub that wires:
//!   - BoringSSL configuration (ciphers, extensions, GREASE, ECH, ALPS).
//!   - TCP profile (SYN options, window scale).
//!   - Direct vs proxied connects — proxy paths delegate to the
//!     [`crate::tls::proxy`] module (HTTP CONNECT + SOCKS5).
//!   - Happy-Eyeballs dual-stack racing for direct connects.

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
///
/// Configures BoringSSL with exact cipher suites, curves, extensions,
/// GREASE behavior, ALPS, cert compression (zlib/brotli/zstd), and ECH
/// from TOML browser profiles.
#[derive(Clone)]
pub struct FingerprintConnector {
    ssl_connector: SslConnector,
    tcp_profile: TcpProfile,
    ech_grease: bool,
    /// ALPS protocol (e.g. "h2") — applied per-connection.
    alps_proto: Option<Vec<u8>>,
    /// Use new ALPS codepoint (0x4469 for Chrome 131+).
    alps_new_codepoint: bool,
    /// Advertise Trust Anchor Identifiers (ext 0xCA34/51764) with an empty
    /// list, as Chrome 148+ does — keeps the ClientHello JA4 at t13d1517.
    request_trust_anchors: bool,
    /// Per-host session ticket cache for TLS resumption (DER-encoded).
    session_cache: Arc<Mutex<LruCache<String, Vec<u8>>>>,
    /// When `true`, skip peer certificate verification entirely.
    /// **Dangerous** — off by default; only the `leyline` CLI's
    /// `-k/--insecure` flag and explicit test fixtures should turn
    /// this on.
    accept_invalid_certs: bool,
    /// DNS resolver for direct connections. Defaults to
    /// [`SystemResolver`] which wraps blocking `getaddrinfo(3)` in
    /// [`tokio::task::spawn_blocking`].
    resolver: Arc<dyn Resolver>,
    /// Happy Eyeballs (RFC 8305) tunables for the dual-stack race.
    happy_eyeballs: HappyEyeballsConfig,
    /// Optional timeout around DNS + TCP + TLS connect.
    connect_timeout: Option<Duration>,
    /// Optional socket-level direct-connect overrides.
    socket_config: SocketConfig,
    /// Configured leaf pins. Pinning replaces
    /// BoringSSL's built-in verifier with a custom callback that does
    /// chain + pin but NOT the `X509_check_host` SAN match — so the
    /// hostname is re-verified explicitly after the handshake.
    pins: Vec<[u8; 32]>,
    /// `true` when a client certificate (mTLS identity) is configured. Like
    /// pins, it is origin-specific and must never be presented to an `https://`
    /// proxy — see [`Self::has_origin_tls_identity`].
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

        // Drive every TLS-level knob from the profile via the shared factory.
        apply_profile_with_trust(&mut builder, profile, TlsMinVersion::Tls12, &trust)?;

        let tls = &profile.tls;

        // ALPN — advertise h2 and http/1.1. (Per-connection override for H1-only
        // WebSocket lives in `tls_handshake`.)
        builder.set_alpn_protos(b"\x02h2\x08http/1.1")?;

        // Session resumption — enable external client-side caching.
        let session_cache = Arc::new(Mutex::new(LruCache::new(
            std::num::NonZeroUsize::new(256).unwrap(),
        )));
        builder
            .set_session_cache_mode(SslSessionCacheMode::CLIENT | SslSessionCacheMode::NO_INTERNAL);
        let cache_clone = session_cache.clone();
        builder.set_new_session_callback(move |ssl, session| {
            if let Some(hostname) = ssl.servername(NameType::HOST_NAME) {
                if let Ok(der) = session.to_der() {
                    // Recover from poison like the read side does — a
                    // silently-dropped ticket means every later handshake
                    // to this host presents the cold (non-resumed) JA4.
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
            accept_invalid_certs: false,
            resolver: Arc::new(SystemResolver),
            happy_eyeballs: HappyEyeballsConfig::default(),
            connect_timeout: None,
            socket_config: SocketConfig::default(),
            pins: trust.pinned_leaf_sha256().to_vec(),
            has_client_identity: trust.client_identity().is_some(),
        })
    }

    /// `true` when this connector carries an origin-specific TLS identity — a
    /// client certificate (mTLS) or leaf pins — that must NOT be presented to,
    /// or applied against, an `https://` CONNECT proxy. The proxy is a separate
    /// peer: leaking the origin client cert to it, or checking its cert against
    /// the origin's pins, would be wrong (and the pin check would fail).
    pub(crate) fn has_origin_tls_identity(&self) -> bool {
        !self.pins.is_empty() || self.has_client_identity
    }

    /// Skip peer certificate verification. **Dangerous** — off by
    /// default; only the `leyline` CLI's `-k/--insecure` flag and
    /// explicit test fixtures should turn this on.
    pub fn set_accept_invalid_certs(&mut self, accept: bool) {
        self.accept_invalid_certs = accept;
    }

    /// Override the DNS resolver (for `/etc/hosts`-style tests or
    /// deterministic offline rigs).
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

    /// Connect to `host:port`, optionally through `proxy_url`. Default
    /// ALPN (h2 preferred).
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

    /// Connect with HTTP/1.1 ALPN only (for WebSocket upgrade). Same
    /// TLS fingerprint, but negotiates http/1.1 instead of h2.
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
        // TLS handshake with all per-connection fingerprint settings.
        self.tls_handshake(tcp_stream, host, alpn_override.is_none())
            .await
    }

    /// Open a fingerprinted TCP connection to `host:port` through the
    /// connector's pluggable resolver and Happy-Eyeballs racer, applying the
    /// browser [`TcpProfile`] SYN options via [`connect_one`].
    ///
    /// Shared by the direct path and the proxy leg (see
    /// [`crate::tls::proxy::connect_to_proxy`]) so a proxy dial honours the same
    /// resolver as a direct connect — a custom/DoH resolver is not bypassed for
    /// the proxy hostname (no DNS leak vs. caller intent) — and the SYN to the
    /// proxy carries the same browser TCP fingerprint.
    pub(crate) async fn dial_tcp(&self, host: &str, port: u16) -> Result<TcpStream, TlsError> {
        // Resolve host. The resolver is pluggable — default
        // [`SystemResolver`] runs blocking `getaddrinfo(3)` off-thread;
        // tests and /etc/hosts-style overrides can substitute their own.
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

        // Race TCP connects across resolved addresses with RFC 8305
        // staggering so IPv6-broken networks still reach IPv4 hosts
        // within one `resolve_delay`. `TcpProfile` is `Copy`, so each
        // attempt gets its own value without a heap clone.
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
    /// Used by all connection paths (direct, proxied, SOCKS5, WebSocket).
    /// `include_alps`: false for h1-only (WebSocket) since Chrome never sends
    /// ALPS when offering only http/1.1.
    ///
    /// `pub(crate)` so the proxy submodules can drive the TLS handshake
    /// over the proxied TCP stream without re-plumbing fingerprint
    /// state.
    pub(crate) async fn tls_handshake(
        &self,
        tcp_stream: TcpStream,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let (stream, meta) = self.handshake_over(tcp_stream, host, include_alps).await?;
        Ok(meta.into_tls_stream(TlsIo::Boring(stream)))
    }

    /// Drive the same fingerprinted TLS handshake over an already-established
    /// TLS stream — the inner (origin-facing) leg of an `https://` CONNECT
    /// proxy, so the origin TLS nests inside the proxy TLS.
    pub(crate) async fn tls_handshake_nested(
        &self,
        inner: TlsIo,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let (stream, meta) = self.handshake_over(inner, host, include_alps).await?;
        Ok(meta.into_tls_stream(TlsIo::Nested(Box::new(stream))))
    }

    /// The fingerprint-bearing TLS handshake, generic over the byte stream so
    /// the direct path (`TcpStream`) and the `https://`-proxy inner leg
    /// (`TlsIo`) share ONE ClientHello — the single source of truth for the
    /// JA4. Returns the handshaked stream plus negotiated metadata; the caller
    /// wraps it in the matching [`TlsIo`] arm.
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

        // ALPS — only when negotiating h2 (not for h1-only WebSocket).
        if include_alps {
            if let Some(ref alps) = self.alps_proto {
                config.add_application_settings(alps)?;
                if self.alps_new_codepoint {
                    config.set_alps_use_new_codepoint(true);
                }
            }
        } else {
            // WebSocket: override ALPN to http/1.1 only.
            config.set_alpn_protos(b"\x08http/1.1")?;
        }

        let mut ssl = config.into_ssl(host)?;

        // Danger mode: skip peer verification entirely. Only wired
        // through the CLI's -k/--insecure flag; off by default.
        if self.accept_invalid_certs {
            ssl.set_verify(SslVerifyMode::NONE);
        }
        let verification_failure = (!self.accept_invalid_certs && !self.pins.is_empty())
            .then(|| install_pinning_verifier(&mut ssl, &self.pins));

        // Session resumption — install cached session ticket before handshake.
        // Recover from a poisoned lock rather than silently skipping resumption
        // (a skip downgrades a resumed JA4 to the cold form, changing the fp).
        {
            let mut cache = lock_unpoisoned(&self.session_cache);
            if let Some(der) = cache.get(host).cloned() {
                if let Ok(session) = SslSession::from_der(&der) {
                    // SAFETY: BoringSSL requires `set_session` to be
                    // called on an Ssl not yet handed to `connect()`.
                    // `ssl` was just constructed via `config.into_ssl`
                    // and has not started its handshake; it will be
                    // driven via `leyline_bssl_tokio::connect` below. The
                    // `SslSession` is owned for the duration of this
                    // block. No concurrent access.
                    unsafe {
                        let _ = ssl.set_session(&session);
                    }
                }
            }
        }

        // ECH GREASE.
        if self.ech_grease {
            ssl.set_enable_ech_grease(true);
        }

        // Trust Anchor Identifiers (extension 0xCA34 / 51764). Chrome 148+ sends
        // an empty `TrustAnchorIdentifierList` to advertise support; emit the
        // same so the ClientHello JA4 matches real Chrome (t13d1517). Without it
        // leyline sends t13d1516 and a CDN edge's JA4+H2 join check soft-blocks
        // the connection — TLS completes but ALPN is stripped, surfacing here as
        // `alpn: negotiated none, expected h2`.
        if self.request_trust_anchors {
            if let Err(e) = ssl.set_requested_trust_anchors(&[]) {
                tracing::warn!(
                    target: "leyline::tls",
                    "set_requested_trust_anchors failed ({e}); trust_anchors \
                     extension may be absent and JA4 will not match Chrome 148"
                );
            }
        }

        // TLS handshake. leyline-bssl-tokio::SslStream::connect requires
        // Pin<&mut Self>; `S: Unpin` lets us pin on the stack.
        let mut stream = leyline_bssl_tokio::SslStream::new(ssl, io)
            .map_err(|e| TlsError::SslConfig(e.to_string()))?;
        if let Err(e) = std::pin::Pin::new(&mut stream).connect().await {
            return Err(classify_handshake(
                verification_failure.as_ref(),
                stream.ssl().verify_result().err(),
                e,
            ));
        }

        // Pinning replaced BoringSSL's built-in verifier, which skips
        // its hostname (SAN) check. Re-verify the leaf against the
        // requested host so a CA-trusted, correctly-pinned cert issued
        // for a different name can't be accepted here. Skipped only
        // when the caller explicitly opted out of verification.
        if !self.pins.is_empty() && !self.accept_invalid_certs {
            let leaf = stream.ssl().peer_certificate().ok_or_else(|| {
                TlsError::Certificate("pinned connection presented no peer certificate".into())
            })?;
            // Mirror `setup_verify_hostname` (leyline-bssl): an IP-literal host
            // must be matched against IP SANs and any other host against DNS
            // SANs. `X509_check_host` never matches IP SANs, so without this
            // branch a valid IP-SAN certificate on a pinned IP connection is
            // wrongly rejected. BoringSSL's `X509_check_host` also rejects
            // partial wildcards unconditionally (`X509_CHECK_FLAG_NO_PARTIAL_
            // WILDCARDS` is 0 in this build), so the hardcoded flags=0 in
            // `check_host` already matches the default path's
            // `NO_PARTIAL_WILDCARDS` strictness — the DNS branch needs no flag.
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
    // BoringSSL reports INVALID_CALL when certificate verification never ran
    // (for example, the peer reset or sent a non-TLS record). It is not a
    // certificate failure and must not hide the typed transport/protocol cause.
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
        None => match error.into_io_error() {
            Ok(error) => TlsError::HandshakeIo(error),
            Err(error) => TlsError::Handshake(error.to_string()),
        },
    }
}

/// Negotiated TLS metadata captured at handshake, paired with the handshaked
/// stream so [`FingerprintConnector::handshake_over`] can stay generic over the
/// byte stream while each caller wraps the stream in its own [`TlsIo`] arm.
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
///
/// Both cache sites must use this: the read side (skipping = cold JA4 on
/// a connection that should resume) and the write side (skipping = every
/// later handshake to that host presents the cold JA4). The cache holds
/// plain DER blobs, so the worst poison outcome is a stale ticket — far
/// better than a permanent fingerprint downgrade.
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
mod tests {
    use super::lock_unpoisoned;
    use std::sync::{Arc, Mutex};

    #[test]
    fn lock_unpoisoned_recovers_after_panic() {
        let cache = Arc::new(Mutex::new(vec![1u8]));
        let poisoner = cache.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the mutex");
        })
        .join();
        assert!(cache.is_poisoned(), "test setup failed to poison the mutex");

        // Both the read and write paths must keep working.
        lock_unpoisoned(&cache).push(2);
        assert_eq!(*lock_unpoisoned(&cache), vec![1, 2]);
    }
}
