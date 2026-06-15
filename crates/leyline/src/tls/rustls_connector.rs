//! Pure-Rust (rustls) TLS connector for the bare / non-fingerprint path.
//!
//! Mirrors [`FingerprintConnector`](crate::tls::FingerprintConnector)'s
//! connect surface (`connect` / `connect_h1`, Happy-Eyeballs racing,
//! proxy delegation, the per-connect timeout) but performs the handshake
//! with rustls instead of BoringSSL. Used only for bare `Session::new()`
//! sessions; browser/profile sessions keep the BoringSSL path. The
//! resulting [`TlsStream`] is backend-agnostic — only the inner
//! [`TlsIo::Rustls`] arm differs.

use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio_rustls::TlsConnector as TokioTlsConnector;

use rustls::pki_types::ServerName;
use rustls::ClientConfig;

use crate::core::SocketConfig;
use crate::tcp::TcpProfile;
use crate::tls::error::TlsError;
use crate::tls::happy_eyeballs::{happy_eyeballs_connect, HappyEyeballsConfig};
use crate::tls::nonblocking::connect_one;
use crate::tls::resolver::{Resolver, SystemResolver};
use crate::tls::rustls_trust::build_client_config;
use crate::tls::trust::TlsTrustConfig;
use crate::tls::{TlsHandshake, TlsIo, TlsStream};

/// ALPN advertised on the default (h2-preferred) path.
const ALPN_H2_H1: &[&[u8]] = &[b"h2", b"http/1.1"];
/// ALPN advertised on the HTTP/1.1-only path (WebSocket upgrade).
const ALPN_H1: &[&[u8]] = &[b"http/1.1"];

/// rustls connector for bare sessions. `Clone` is cheap — the configs are
/// `Arc`, every other field is `Copy` or `Arc`.
#[derive(Clone)]
pub struct RustlsConnector {
    /// ALPN = h2 + http/1.1.
    config_h2: Arc<ClientConfig>,
    /// ALPN = http/1.1 only (WebSocket).
    config_h1: Arc<ClientConfig>,
    tcp_profile: TcpProfile,
    resolver: Arc<dyn Resolver>,
    happy_eyeballs: HappyEyeballsConfig,
    connect_timeout: Option<Duration>,
    socket_config: SocketConfig,
}

impl RustlsConnector {
    /// Build a bare rustls connector from a TCP profile and trust config.
    pub(crate) fn new(
        tcp: TcpProfile,
        trust: &TlsTrustConfig,
        accept_invalid_certs: bool,
    ) -> Result<Self, TlsError> {
        // Two configs sharing the same roots/verifier, differing only in
        // ALPN — rustls ALPN lives on the config, not the connection.
        let config_h2 = Arc::new(build_client_config(
            trust,
            accept_invalid_certs,
            ALPN_H2_H1,
        )?);
        let config_h1 = Arc::new(build_client_config(trust, accept_invalid_certs, ALPN_H1)?);
        Ok(Self {
            config_h2,
            config_h1,
            tcp_profile: tcp,
            resolver: Arc::new(SystemResolver),
            happy_eyeballs: HappyEyeballsConfig::default(),
            connect_timeout: None,
            socket_config: SocketConfig::default(),
        })
    }

    pub(crate) fn with_resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    pub(crate) fn with_socket_config(mut self, config: SocketConfig) -> Self {
        self.socket_config = config;
        self
    }

    pub(crate) fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    pub(crate) fn with_happy_eyeballs_config(mut self, config: HappyEyeballsConfig) -> Self {
        self.happy_eyeballs = config;
        self
    }

    /// Connect to `host:port`, optionally through `proxy`. Default ALPN
    /// (h2 preferred).
    pub(crate) async fn connect(
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
                None => self.connect_direct(host, port, true).await,
            }
        };
        self.with_timeout(fut).await
    }

    /// Connect with HTTP/1.1 ALPN only (WebSocket upgrade).
    pub(crate) async fn connect_h1(
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
                None => self.connect_direct(host, port, false).await,
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

    async fn connect_direct(&self, host: &str, port: u16, h2: bool) -> Result<TlsStream, TlsError> {
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

        self.handshake(tcp_stream, host, h2).await
    }

    /// Drive the rustls handshake over an established TCP stream and
    /// capture the backend-neutral metadata.
    async fn handshake(
        &self,
        tcp_stream: TcpStream,
        host: &str,
        h2: bool,
    ) -> Result<TlsStream, TlsError> {
        let config = if h2 {
            self.config_h2.clone()
        } else {
            self.config_h1.clone()
        };
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|e| TlsError::SslConnect(format!("invalid server name `{host}`: {e}")))?;

        let tls = TokioTlsConnector::from(config)
            .connect(server_name, tcp_stream)
            .await
            .map_err(|e| TlsError::SslConnect(format!("rustls handshake: {e}")))?;

        let (_io, conn) = tls.get_ref();
        let alpn = conn.alpn_protocol().map(|p| p.to_vec());
        let peer_cert_der = conn
            .peer_certificates()
            .and_then(|certs| certs.first())
            .map(|cert| cert.as_ref().to_vec());
        let tls_version = conn.protocol_version().map(|v| format!("{v:?}"));
        let tls_cipher = conn
            .negotiated_cipher_suite()
            .map(|cs| format!("{:?}", cs.suite()));

        Ok(TlsStream {
            stream: TlsIo::Rustls(tls),
            alpn,
            peer_cert_der,
            tls_version,
            tls_cipher,
        })
    }
}

impl TlsHandshake for RustlsConnector {
    async fn do_tls_handshake(
        &self,
        tcp_stream: TcpStream,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        // `include_alps == true` is the h2 path (matches the BoringSSL
        // connector's ALPS-on-h2 convention); false is http/1.1-only.
        self.handshake(tcp_stream, host, include_alps).await
    }
}

impl std::fmt::Debug for RustlsConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RustlsConnector")
            .field("tcp_profile", &self.tcp_profile)
            .finish_non_exhaustive()
    }
}
