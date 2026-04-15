//! TLS connector that creates fingerprinted connections from browser profiles.

use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex};

use boring::ssl::{NameType, SslConnector, SslSession, SslSessionCacheMode, SslVerifyMode};
use lru::LruCache;
use tokio::net::TcpStream;

use leyline_profile::BrowserProfile;
use leyline_tcp::TcpProfile;

use crate::builder::{apply_profile, TlsMinVersion};
use crate::error::TlsError;
use crate::TlsStream;

/// Creates TLS connections matching a browser's fingerprint.
///
/// Configures BoringSSL with exact cipher suites, curves, extensions,
/// GREASE behavior, ALPS, extension permutation, and ECH from TOML
/// browser profiles. Every field in the profile is wired — nothing
/// is silently ignored.
pub struct FingerprintConnector {
    ssl_connector: SslConnector,
    tcp_profile: TcpProfile,
    ech_grease: bool,
    /// ALPS protocol (e.g. "h2") — applied per-connection.
    alps_proto: Option<Vec<u8>>,
    /// Use new ALPS codepoint (0x4469 for Chrome 131+).
    alps_new_codepoint: bool,
    /// Fixed extension permutation indices (Firefox/Safari deterministic order).
    extension_permutation: Option<Vec<u8>>,
    /// Deterministic GREASE seed (7 bytes, one per GREASE type).
    grease_seed: Option<Vec<u8>>,
    /// Fixed ECH GREASE payload length.
    ech_grease_payload_len: Option<u16>,
    /// Per-host session ticket cache for TLS resumption (DER-encoded).
    session_cache: Arc<Mutex<LruCache<String, Vec<u8>>>>,
    /// When `true`, skip peer certificate verification entirely.
    /// **Dangerous** — off by default; only the `leyline` CLI's
    /// `-k/--insecure` flag and explicit test fixtures should turn
    /// this on.
    accept_invalid_certs: bool,
}

impl FingerprintConnector {
    /// Build a connector from a browser profile, TCP profile, and optional GREASE seed.
    pub fn new(
        profile: &BrowserProfile,
        tcp: TcpProfile,
        grease_seed: Option<&[u8]>,
    ) -> Result<Self, TlsError> {
        let mut builder = SslConnector::builder(boring::ssl::SslMethod::tls_client())?;

        // Drive every TLS-level knob from the profile via the shared factory.
        apply_profile(&mut builder, profile, TlsMinVersion::Tls12)?;

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
                    if let Ok(mut cache) = cache_clone.lock() {
                        cache.put(hostname.to_string(), der);
                    }
                }
            }
        });

        Ok(Self {
            ssl_connector: builder.build(),
            tcp_profile: tcp,
            ech_grease: tls.ech_grease,
            alps_proto: tls.alps.as_ref().map(|s| s.as_bytes().to_vec()),
            alps_new_codepoint: tls.alps_new_codepoint,
            extension_permutation: tls.extension_permutation.clone(),
            grease_seed: grease_seed.map(|s| s.to_vec()),
            ech_grease_payload_len: tls.ech_grease_payload_len,
            session_cache,
            accept_invalid_certs: false,
        })
    }

    /// Disable peer certificate verification. **Dangerous** — only
    /// use this for the `leyline` CLI's `-k/--insecure` flag or
    /// controlled test fixtures.
    pub fn set_accept_invalid_certs(&mut self, accept: bool) {
        self.accept_invalid_certs = accept;
    }

    /// Connect to a host:port, optionally through a proxy.
    #[tracing::instrument(
        name = "tls.connect",
        level = "debug",
        skip(self, proxy),
        fields(host, port, proxied = proxy.is_some())
    )]
    pub async fn connect(
        &self,
        host: &str,
        port: u16,
        proxy: Option<&str>,
    ) -> Result<TlsStream, TlsError> {
        if let Some(proxy_url) = proxy {
            return self.connect_proxied(host, port, proxy_url).await;
        }
        self.connect_direct(host, port).await
    }

    /// Connect with HTTP/1.1 ALPN only (for WebSocket upgrade).
    /// Same TLS fingerprint, but negotiates http/1.1 instead of h2.
    pub async fn connect_h1(
        &self,
        host: &str,
        port: u16,
        proxy: Option<&str>,
    ) -> Result<TlsStream, TlsError> {
        // For now, reuse the same connect path — the ALPN override
        // happens in the per-connection SSL configuration below.
        if let Some(proxy_url) = proxy {
            return self.connect_proxied_h1(host, port, proxy_url).await;
        }
        self.connect_direct_h1(host, port).await
    }

    /// Direct connection — no proxy.
    async fn connect_direct(&self, host: &str, port: u16) -> Result<TlsStream, TlsError> {
        self.connect_direct_with_alpn(host, port, None).await
    }

    /// Direct connection with HTTP/1.1 only ALPN (for WebSocket).
    async fn connect_direct_h1(&self, host: &str, port: u16) -> Result<TlsStream, TlsError> {
        self.connect_direct_with_alpn(host, port, Some(b"\x08http/1.1"))
            .await
    }

    /// Proxied connection with HTTP/1.1 only ALPN (for WebSocket).
    async fn connect_proxied_h1(
        &self,
        host: &str,
        port: u16,
        proxy_url: &str,
    ) -> Result<TlsStream, TlsError> {
        self.connect_proxied_inner(host, port, proxy_url, false)
            .await
    }

    /// Direct connection with optional ALPN override.
    async fn connect_direct_with_alpn(
        &self,
        host: &str,
        port: u16,
        alpn_override: Option<&[u8]>,
    ) -> Result<TlsStream, TlsError> {
        // Resolve DNS.
        let addr_str = format!("{}:{}", host, port);
        let sock_addr = tokio::task::spawn_blocking(move || {
            addr_str
                .to_socket_addrs()
                .map_err(TlsError::Dns)?
                .next()
                .ok_or_else(|| {
                    TlsError::Dns(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "no addresses resolved",
                    ))
                })
        })
        .await
        .map_err(|e| TlsError::Dns(std::io::Error::other(e)))??;

        // Create socket via socket2 for TCP fingerprinting.
        let domain = match sock_addr {
            std::net::SocketAddr::V4(_) => socket2::Domain::IPV4,
            std::net::SocketAddr::V6(_) => socket2::Domain::IPV6,
        };
        let socket =
            socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))
                .map_err(TlsError::TcpConnect)?;

        // Apply TCP fingerprint before connect.
        self.tcp_profile.apply(&socket);
        socket.set_nonblocking(true).map_err(TlsError::TcpConnect)?;

        // TCP connect.
        match socket.connect(&sock_addr.into()) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) => {}
            Err(e) => return Err(TlsError::TcpConnect(e)),
        }
        let std_stream: std::net::TcpStream = socket.into();
        let tcp_stream = TcpStream::from_std(std_stream).map_err(TlsError::TcpConnect)?;

        // Wait for TCP connect to complete.
        tcp_stream.writable().await.map_err(TlsError::TcpConnect)?;
        if let Some(e) = tcp_stream.take_error().map_err(TlsError::TcpConnect)? {
            return Err(TlsError::TcpConnect(e));
        }

        // TLS handshake with all per-connection fingerprint settings.
        self.tls_handshake(tcp_stream, host, alpn_override.is_none())
            .await
    }

    /// Perform TLS handshake with all per-connection fingerprint settings.
    /// Used by all connection paths (direct, proxied, SOCKS5, WebSocket).
    /// `include_alps`: false for h1-only (WebSocket) since Chrome never sends
    /// ALPS when offering only http/1.1.
    async fn tls_handshake(
        &self,
        tcp_stream: TcpStream,
        host: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        let mut config = self
            .ssl_connector
            .configure()
            .map_err(TlsError::Handshake)?;

        // ALPS — only when negotiating h2 (not for h1-only WebSocket).
        if include_alps {
            if let Some(ref alps) = self.alps_proto {
                config
                    .add_application_settings(alps)
                    .map_err(TlsError::Handshake)?;
                if self.alps_new_codepoint {
                    config.set_alps_use_new_codepoint(true);
                }
            }
        } else {
            // WebSocket: override ALPN to http/1.1 only.
            config
                .set_alpn_protos(b"\x08http/1.1")
                .map_err(TlsError::Handshake)?;
        }

        let mut ssl = config.into_ssl(host).map_err(TlsError::Handshake)?;

        // Danger mode: skip peer verification entirely. Only wired
        // through the CLI's -k/--insecure flag; off by default.
        if self.accept_invalid_certs {
            ssl.set_verify(SslVerifyMode::NONE);
        }

        // Session resumption — install cached session ticket before handshake.
        if let Ok(mut cache) = self.session_cache.lock() {
            if let Some(der) = cache.get(host).cloned() {
                if let Ok(session) = SslSession::from_der(&der) {
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

        // Fixed extension permutation (Firefox/Safari deterministic order).
        if let Some(ref perm) = self.extension_permutation {
            ssl.set_extension_permutation_fixed(perm);
        }

        // Deterministic GREASE seed (stable fingerprint per identity).
        if let Some(ref seed) = self.grease_seed {
            ssl.set_grease_seed(seed);
        }

        // Fixed ECH GREASE payload length.
        if let Some(len) = self.ech_grease_payload_len {
            ssl.set_ech_grease_payload_len(len);
        }

        // TLS handshake.
        let stream = tokio_boring::SslStreamBuilder::new(ssl, tcp_stream)
            .connect()
            .await
            .map_err(|e| TlsError::SslConnect(e.to_string()))?;

        let alpn = stream.ssl().selected_alpn_protocol().map(|p| p.to_vec());
        let peer_cert_der = stream
            .ssl()
            .peer_certificate()
            .and_then(|cert| cert.to_der().ok());
        let tls_version = Some(stream.ssl().version_str().to_string());
        let tls_cipher = stream.ssl().current_cipher().map(|c| c.name().to_string());

        Ok(TlsStream {
            stream,
            alpn,
            peer_cert_der,
            tls_version,
            tls_cipher,
        })
    }

    /// Connect through a SOCKS5 proxy (RFC 1928).
    async fn connect_socks5(
        &self,
        host: &str,
        port: u16,
        proxy: &url::Url,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let proxy_host = proxy
            .host_str()
            .ok_or_else(|| TlsError::Profile("socks5 proxy has no host".into()))?;
        let proxy_port = proxy.port().unwrap_or(1080);

        // TCP connect to SOCKS5 proxy.
        let proxy_addr = format!("{proxy_host}:{proxy_port}");
        let mut tcp_stream = TcpStream::connect(&proxy_addr)
            .await
            .map_err(TlsError::TcpConnect)?;

        let has_auth = !proxy.username().is_empty();

        // Greeting: version 5, auth methods.
        if has_auth {
            // Offer NO_AUTH (0x00) and USERNAME/PASSWORD (0x02).
            tcp_stream
                .write_all(&[0x05, 0x02, 0x00, 0x02])
                .await
                .map_err(TlsError::TcpConnect)?;
        } else {
            // Offer NO_AUTH only.
            tcp_stream
                .write_all(&[0x05, 0x01, 0x00])
                .await
                .map_err(TlsError::TcpConnect)?;
        }

        // Read server's chosen method.
        let mut method_resp = [0u8; 2];
        tcp_stream
            .read_exact(&mut method_resp)
            .await
            .map_err(TlsError::TcpConnect)?;

        if method_resp[0] != 0x05 {
            return Err(TlsError::Profile(
                "socks5: invalid version in response".into(),
            ));
        }

        match method_resp[1] {
            0x00 => {} // No auth needed.
            0x02 => {
                // Username/password auth (RFC 1929).
                let username = percent_decode(proxy.username());
                let password = proxy.password().map(percent_decode).unwrap_or_default();
                if username.len() > 255 || password.len() > 255 {
                    return Err(TlsError::Profile(
                        "socks5: username or password exceeds 255 bytes".into(),
                    ));
                }
                let mut auth_req = Vec::with_capacity(3 + username.len() + password.len());
                auth_req.push(0x01); // Sub-negotiation version.
                auth_req.push(username.len() as u8);
                auth_req.extend_from_slice(username.as_bytes());
                auth_req.push(password.len() as u8);
                auth_req.extend_from_slice(password.as_bytes());
                tcp_stream
                    .write_all(&auth_req)
                    .await
                    .map_err(TlsError::TcpConnect)?;

                let mut auth_resp = [0u8; 2];
                tcp_stream
                    .read_exact(&mut auth_resp)
                    .await
                    .map_err(TlsError::TcpConnect)?;
                if auth_resp[1] != 0x00 {
                    return Err(TlsError::Profile("socks5: authentication failed".into()));
                }
            }
            0xFF => {
                return Err(TlsError::Profile(
                    "socks5: no acceptable auth method".into(),
                ))
            }
            _ => {
                return Err(TlsError::Profile(format!(
                    "socks5: unsupported auth method 0x{:02x}",
                    method_resp[1]
                )))
            }
        }

        // CONNECT request.
        let host_bytes = host.as_bytes();
        let mut connect_req = Vec::with_capacity(7 + host_bytes.len());
        connect_req.push(0x05); // Version.
        connect_req.push(0x01); // CONNECT command.
        connect_req.push(0x00); // Reserved.
        connect_req.push(0x03); // Domain name address type.
        connect_req.push(host_bytes.len() as u8);
        connect_req.extend_from_slice(host_bytes);
        connect_req.push((port >> 8) as u8);
        connect_req.push(port as u8);
        tcp_stream
            .write_all(&connect_req)
            .await
            .map_err(TlsError::TcpConnect)?;

        // Read CONNECT response (at least 10 bytes for IPv4 bind address).
        let mut resp_buf = [0u8; 10];
        tcp_stream
            .read_exact(&mut resp_buf[..4])
            .await
            .map_err(TlsError::TcpConnect)?;

        if resp_buf[0] != 0x05 {
            return Err(TlsError::Profile(
                "socks5: invalid CONNECT response version".into(),
            ));
        }
        if resp_buf[1] != 0x00 {
            let reason = match resp_buf[1] {
                0x01 => "general failure",
                0x02 => "connection not allowed",
                0x03 => "network unreachable",
                0x04 => "host unreachable",
                0x05 => "connection refused",
                0x06 => "TTL expired",
                0x07 => "command not supported",
                0x08 => "address type not supported",
                _ => "unknown error",
            };
            return Err(TlsError::Profile(format!(
                "socks5: CONNECT failed: {reason}"
            )));
        }

        // Skip the bind address. Address type is at resp_buf[3].
        match resp_buf[3] {
            0x01 => {
                // IPv4: 4 bytes + 2 port bytes. We already read 4, need 6 more.
                let mut skip = [0u8; 6];
                tcp_stream
                    .read_exact(&mut skip)
                    .await
                    .map_err(TlsError::TcpConnect)?;
            }
            0x03 => {
                // Domain: 1 byte length + N bytes + 2 port bytes.
                let mut len_buf = [0u8; 1];
                tcp_stream
                    .read_exact(&mut len_buf)
                    .await
                    .map_err(TlsError::TcpConnect)?;
                let skip_len = len_buf[0] as usize + 2;
                let mut skip = vec![0u8; skip_len];
                tcp_stream
                    .read_exact(&mut skip)
                    .await
                    .map_err(TlsError::TcpConnect)?;
            }
            0x04 => {
                // IPv6: 16 bytes + 2 port bytes.
                let mut skip = [0u8; 18];
                tcp_stream
                    .read_exact(&mut skip)
                    .await
                    .map_err(TlsError::TcpConnect)?;
            }
            _ => {
                return Err(TlsError::Profile(format!(
                    "socks5: unknown address type 0x{:02x}",
                    resp_buf[3]
                )));
            }
        }

        // TLS handshake with the requested ALPN/ALPS settings.
        self.tls_handshake(tcp_stream, host, include_alps).await
    }

    /// Connect through a proxy (HTTP CONNECT or SOCKS5).
    async fn connect_proxied(
        &self,
        host: &str,
        port: u16,
        proxy_url: &str,
    ) -> Result<TlsStream, TlsError> {
        self.connect_proxied_inner(host, port, proxy_url, true)
            .await
    }

    /// HTTP CONNECT proxy with configurable ALPS.
    async fn connect_proxied_inner(
        &self,
        host: &str,
        port: u16,
        proxy_url: &str,
        include_alps: bool,
    ) -> Result<TlsStream, TlsError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Parse proxy URL.
        let proxy = url::Url::parse(proxy_url)
            .map_err(|e| TlsError::Profile(format!("invalid proxy URL: {e}")))?;

        // Dispatch based on scheme.
        if proxy.scheme() == "socks5" || proxy.scheme() == "socks5h" {
            return self.connect_socks5(host, port, &proxy, include_alps).await;
        }

        let proxy_host = proxy
            .host_str()
            .ok_or_else(|| TlsError::Profile("proxy has no host".into()))?;
        let proxy_port = proxy.port().unwrap_or(8080);

        // TCP connect to proxy.
        let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
        let mut tcp_stream = TcpStream::connect(&proxy_addr)
            .await
            .map_err(TlsError::TcpConnect)?;

        // Send CONNECT request.
        let connect_req = if let Some(password) = proxy.password() {
            let username = percent_decode(proxy.username());
            let password = percent_decode(password);
            let credentials = base64_encode(&format!("{username}:{password}"));
            format!(
                "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: Basic {credentials}\r\n\r\n"
            )
        } else {
            format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n")
        };

        tcp_stream
            .write_all(connect_req.as_bytes())
            .await
            .map_err(TlsError::TcpConnect)?;

        // Read response until \r\n\r\n (end of HTTP headers).
        // TCP may deliver in multiple segments, so loop.
        let mut response_buf = Vec::with_capacity(1024);
        let mut tmp = [0u8; 256];
        loop {
            let n = tcp_stream
                .read(&mut tmp)
                .await
                .map_err(TlsError::TcpConnect)?;
            if n == 0 {
                return Err(TlsError::Profile(
                    "proxy closed connection before CONNECT response".into(),
                ));
            }
            response_buf.extend_from_slice(&tmp[..n]);
            if response_buf.len() > 8192 {
                return Err(TlsError::Profile("proxy CONNECT response too large".into()));
            }
            if response_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = String::from_utf8_lossy(&response_buf);
        if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
            return Err(TlsError::Profile(format!(
                "proxy CONNECT failed: {}",
                response.lines().next().unwrap_or("no response")
            )));
        }

        // TLS handshake with full fingerprint settings.
        self.tls_handshake(tcp_stream, host, include_alps).await
    }
}

/// Simple base64 encoding for proxy auth.
fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(n >> 18 & 0x3F) as usize] as char);
        out.push(CHARS[(n >> 12 & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[(n >> 6 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decode percent-encoded URL component (e.g. proxy username/password).
fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
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
