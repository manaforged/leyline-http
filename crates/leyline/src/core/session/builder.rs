use std::sync::{Arc, LazyLock};

use crate::cookie::Jar as CookieJar;
use crate::core::{
    CompressionConfig, DnsConfig, NoProxy, PoolConfig, ProxyConfig, ProxyUrl, RedirectPolicy,
    SocketConfig, TimeoutConfig, WebSocketConfig,
};
use crate::h2::H2Config;
use crate::pool::Pool;
use crate::profile::{Browser, ChromiumBrand, Platform, ProfileRegistry};
use crate::tcp::TcpProfile;
use crate::tls::{
    ConnectorVariant, FingerprintConnector, HappyEyeballsConfig, Resolver, TlsTrustConfig,
};

use super::proxy::env_proxy;
use super::{ProtocolPolicy, Session, SessionInner};
use crate::audit::AuditTlsCache;
use crate::core::error::{Error, Result};

static PROFILES: LazyLock<ProfileRegistry> = LazyLock::new(ProfileRegistry::builtin);

/// The synthetic bare profile, materialised once. Backs the default
/// (no-impersonation) session so `&'static crate::profile::BrowserProfile`
/// is available to the connector and the H3 path, exactly like the
/// registry-backed browser profiles.
static BARE_PROFILE: LazyLock<crate::profile::BrowserProfile> =
    LazyLock::new(crate::profile::BrowserProfile::bare);

/// Session builder - configure browser, platform, proxy, timeout, cookies.
pub struct SessionBuilder {
    /// The browser to impersonate. `None` (the default) means **bare** —
    /// no impersonation, a plain `leyline/<version>` client. `.browser(b)`
    /// opts into a browser fingerprint.
    browser: Option<Browser>,
    platform: Platform,
    /// `true` once `.platform(...)` (or a platform-pinning convenience like
    /// `.safari()`) was called. When still `false` at build time the
    /// platform is chosen by default: an impersonation profile defaults to
    /// Windows (the dominant real-user OS; a one-time notice fires), while a
    /// bare session follows the host OS.
    platform_explicit: bool,
    brand: ChromiumBrand,
    proxy: Option<String>,
    /// Set when `proxy` was discovered from the environment at build
    /// time (vs an explicit `.proxy(...)` call).
    proxy_from_env: bool,
    max_redirects: usize,
    proxy_config: ProxyConfig,
    dns_config: DnsConfig,
    timeouts: TimeoutConfig,
    pool_config: PoolConfig,
    socket_config: SocketConfig,
    redirect_policy: RedirectPolicy,
    compression: CompressionConfig,
    websocket_config: WebSocketConfig,
    https_only: bool,
    audit: bool,
    cookie_jar: Option<CookieJar>,
    tcp_profile: Option<TcpProfile>,
    protocol_policy: ProtocolPolicy,
    config_error: Option<String>,
    accept_language_override: Option<String>,
    extra_identity_headers: Vec<(String, String)>,
    accept_invalid_certs: bool,
    pool_idle_timeout: Option<std::time::Duration>,
    happy_eyeballs: Option<HappyEyeballsConfig>,
    tls_trust: TlsTrustConfig,
    /// Session-wide default retry policy (none unless set via `retry`).
    default_retry: crate::core::retry::RetryPolicy,
}

impl SessionBuilder {
    pub(super) fn new() -> Self {
        Self {
            browser: None,
            platform: Platform::default(),
            platform_explicit: false,
            brand: ChromiumBrand::default(),
            proxy: None,
            proxy_from_env: false,
            max_redirects: 10,
            proxy_config: ProxyConfig::default(),
            dns_config: DnsConfig::default(),
            timeouts: TimeoutConfig::default(),
            pool_config: PoolConfig::default(),
            socket_config: SocketConfig::default(),
            redirect_policy: RedirectPolicy::default(),
            compression: CompressionConfig::default(),
            websocket_config: WebSocketConfig::default(),
            https_only: false,
            audit: false,
            cookie_jar: None,
            tcp_profile: None,
            protocol_policy: ProtocolPolicy::Auto,
            config_error: None,
            accept_language_override: None,
            extra_identity_headers: Vec::new(),
            accept_invalid_certs: false,
            pool_idle_timeout: None,
            happy_eyeballs: None,
            tls_trust: TlsTrustConfig::default(),
            default_retry: crate::core::retry::RetryPolicy::none(),
        }
    }

    /// Set a proxy URL (http:// with CONNECT tunnel).
    pub fn proxy(mut self, proxy: impl Into<String>) -> Self {
        let proxy = proxy.into();
        self.proxy_config = self.proxy_config.set_default_proxy(proxy.clone());
        self.proxy = Some(proxy);
        self
    }

    /// Set a validated proxy URL.
    pub fn proxy_url(mut self, proxy: ProxyUrl) -> Self {
        let proxy = proxy.into_string();
        self.proxy_config = self.proxy_config.set_default_proxy(proxy.clone());
        self.proxy = Some(proxy);
        self
    }

    /// Replace the full proxy configuration.
    pub fn proxies(mut self, config: ProxyConfig) -> Self {
        self.proxy = config.first_proxy().map(ToOwned::to_owned);
        self.proxy_config = config;
        self
    }

    /// Replace the proxy bypass matcher.
    pub fn no_proxy(mut self, no_proxy: NoProxy) -> Self {
        self.proxy_config = self.proxy_config.no_proxy(no_proxy);
        self
    }

    /// Do not honour `HTTP_PROXY` / `HTTPS_PROXY` for this session.
    pub fn disable_env_proxies(mut self) -> Self {
        self.proxy_config = self.proxy_config.without_env();
        self
    }

    /// Set request timeout (default: 5 minutes).
    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeouts.total = timeout;
        self
    }

    /// Set a session-wide default retry policy, inherited by every request that
    /// does not override it via [`crate::RequestBuilder::retry`]. Default: no
    /// retries.
    pub fn retry(mut self, policy: crate::core::retry::RetryPolicy) -> Self {
        self.default_retry = policy;
        self
    }

    /// Replace timeout configuration.
    pub fn timeouts(mut self, config: TimeoutConfig) -> Self {
        self.timeouts = config;
        self
    }

    /// Set DNS + TCP + TLS connect timeout.
    pub fn connect_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeouts.connect = Some(timeout);
        self
    }

    /// Set buffered response-body read timeout.
    pub fn read_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeouts.read = Some(timeout);
        self
    }

    /// Override the connection pool's idle-eviction timeout (default: 300s).
    ///
    /// Pool entries whose `last_use` is older than this duration are
    /// evicted on the next `evict_idle` pass. Raise this when a caller has
    /// its own keep-warm cadence (e.g. a 30-minute heartbeat) and wants
    /// the pooled TLS connection to survive between ticks instead of
    /// being torn down at the default 300 seconds.
    pub fn pool_idle_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.pool_idle_timeout = Some(timeout);
        self.pool_config.idle_timeout = timeout;
        self
    }

    /// Replace connection-pool configuration.
    pub fn pool_config(mut self, config: PoolConfig) -> Self {
        self.pool_idle_timeout = Some(config.idle_timeout);
        self.pool_config = config;
        self
    }

    /// Set pool idle timeout and maximum pooled destinations.
    pub fn pool_limits(
        mut self,
        idle_timeout: std::time::Duration,
        max_connections: usize,
    ) -> Self {
        self.pool_idle_timeout = Some(idle_timeout);
        self.pool_config.idle_timeout = idle_timeout;
        self.pool_config.max_connections = max_connections.max(1);
        self
    }

    /// Set the maximum simultaneous HTTP/1.1 connections per destination
    /// `(host, port, proxy)`.
    ///
    /// HTTP/1.1 cannot multiplex, so per-host concurrency comes from opening
    /// several connections. The default is 256 (throughput-favouring — H1 is
    /// the rare ALPN fallback, so the per-host socket *count* is a fingerprint
    /// signal that is already moot once you are off HTTP/2). Set it to **6** to
    /// strictly mirror a real Chrome's per-host socket limit
    /// (`kMaxSocketsPerGroup`). Has no effect on HTTP/2 (one multiplexed
    /// connection per host).
    pub fn h1_max_conns_per_host(mut self, max: usize) -> Self {
        self.pool_config.max_h1_conns_per_host = max.max(1);
        self
    }

    /// Disable connection reuse for this session.
    pub fn disable_keepalive(mut self) -> Self {
        self.pool_config.keepalive = false;
        self
    }

    /// Impersonate a specific browser. Without this call the session is
    /// **bare** — a plain `leyline/<version>` client with no browser
    /// fingerprint. Call this (or a convenience like [`Self::chrome`]) to
    /// opt into browser parity.
    pub fn browser(mut self, browser: Browser) -> Self {
        self.browser = Some(browser);
        self
    }

    /// Use the latest bundled Chrome profile.
    pub fn chrome(self) -> Self {
        self.browser(Browser::Chrome147)
    }

    /// Use the latest bundled Firefox profile.
    pub fn firefox(self) -> Self {
        self.browser(Browser::Firefox150)
    }

    /// Use the latest bundled Safari/macOS profile.
    pub fn safari(self) -> Self {
        self.browser(Browser::Safari18).platform(Platform::MacOS)
    }

    /// Use a browser/platform pair in one call.
    pub fn profile(self, browser: Browser, platform: Platform) -> Self {
        self.browser(browser).platform(platform)
    }

    /// Apply a Chromium-family identity overlay (Edge, Brave, Opera).
    ///
    /// Leaves TLS ClientHello and HTTP/2 SETTINGS untouched - those
    /// are byte-identical across Chromium siblings at a given
    /// Chromium version. What changes is a small set of HTTP
    /// identity headers:
    ///
    /// - `user-agent` suffix - `Edg/NNN` for Edge, `OPR/NNN` for
    ///   Opera, unchanged for Brave (Brave matches Chrome's UA by
    ///   design).
    /// - `sec-ch-ua` brand list - `"Microsoft Edge"`, `"Brave"`, or
    ///   `"Opera"` in place of `"Google Chrome"`. Opera additionally
    ///   uses the `"Not:A-Brand"` placeholder form.
    /// - Extra privacy headers - `dnt: 1` for Edge, `sec-gpc: 1`
    ///   for Brave.
    /// - Navigation `accept` - Brave drops `signed-exchange;v=b3`
    ///   because it disables signed exchanges by default.
    ///
    /// The brand setter is a no-op when applied to non-Chromium
    /// profiles (Firefox, Safari, OkHttp) - the overlay only
    /// affects Chrome profiles. Opera is typically based on
    /// `Chromium N-2`; pair with `Browser::Chrome145` for Opera 129.
    pub fn brand(mut self, brand: ChromiumBrand) -> Self {
        self.brand = brand;
        self
    }

    /// Use the latest bundled Microsoft Edge identity.
    pub fn edge(self) -> Self {
        self.brand(ChromiumBrand::Edge)
    }

    /// Use the latest bundled Brave identity.
    pub fn brave(self) -> Self {
        self.browser(Browser::Brave146).platform(Platform::MacOS)
    }

    /// Use the latest bundled Opera identity.
    pub fn opera(self) -> Self {
        self.brand(ChromiumBrand::Opera)
    }

    /// Use the latest bundled Vivaldi identity.
    pub fn vivaldi(self) -> Self {
        self.brand(ChromiumBrand::Vivaldi)
    }

    /// Set the target platform.
    ///
    /// When never called, the session defaults to [`Platform::Windows`]
    /// (the dominant real-user OS) regardless of the host it builds on — a
    /// Mac/Linux build that forgets this ships a Windows TLS+TCP+UA
    /// fingerprint. The default is deliberate; build emits a one-time
    /// `tracing::info` so the silent choice is observable.
    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self.platform_explicit = true;
        self
    }

    /// Set maximum number of redirects to follow.
    pub fn max_redirects(mut self, n: usize) -> Self {
        self.max_redirects = n;
        self.redirect_policy = RedirectPolicy::limited(n);
        self
    }

    /// Replace redirect follow policy.
    pub fn redirect_policy(mut self, policy: RedirectPolicy) -> Self {
        self.max_redirects = policy.max_redirects_hint();
        self.redirect_policy = policy;
        self
    }

    /// Provide a pre-populated cookie jar.
    pub fn cookie_jar(mut self, jar: CookieJar) -> Self {
        self.cookie_jar = Some(jar);
        self
    }

    /// Set a custom TCP fingerprint profile.
    pub fn tcp_profile(mut self, profile: TcpProfile) -> Self {
        self.tcp_profile = Some(profile);
        self
    }

    /// Use a custom DNS resolver for direct connections.
    ///
    /// Proxy connections still resolve at the proxy unless the proxy
    /// protocol itself requires local resolution.
    pub fn resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.dns_config = self.dns_config.resolver(resolver);
        self
    }

    /// Replace direct-connect DNS configuration.
    pub fn dns(mut self, config: DnsConfig) -> Self {
        self.dns_config = config;
        self
    }

    /// Override one host to one socket address for direct connects.
    pub fn resolve_host(mut self, host: impl AsRef<str>, addr: std::net::SocketAddr) -> Self {
        self.dns_config = self.dns_config.resolve_host(host, addr);
        self
    }

    /// Override one host to multiple socket addresses for direct connects.
    pub fn resolve_host_to_addrs<I>(mut self, host: impl AsRef<str>, addrs: I) -> Self
    where
        I: IntoIterator<Item = std::net::SocketAddr>,
    {
        self.dns_config = self.dns_config.resolve_host_to_addrs(host, addrs);
        self
    }

    /// Override Happy Eyeballs dual-stack connect tunables.
    pub fn happy_eyeballs(mut self, config: HappyEyeballsConfig) -> Self {
        self.happy_eyeballs = Some(config);
        self
    }

    /// Replace TLS trust-root and client-certificate configuration.
    pub fn tls_trust(mut self, trust: TlsTrustConfig) -> Self {
        self.tls_trust = trust;
        self
    }

    /// Replace low-level socket options for direct connects.
    pub fn socket_config(mut self, config: SocketConfig) -> Self {
        self.socket_config = config;
        self
    }

    /// Bind direct sockets to a local IP address.
    pub fn local_address(mut self, address: std::net::IpAddr) -> Self {
        self.socket_config.local_address = Some(address);
        self
    }

    /// Override TCP_NODELAY.
    pub fn tcp_nodelay(mut self, enabled: bool) -> Self {
        self.socket_config.tcp_nodelay = Some(enabled);
        self
    }

    /// Set TCP keepalive idle time.
    pub fn tcp_keepalive(mut self, idle: std::time::Duration) -> Self {
        self.socket_config.tcp_keepalive = Some(idle);
        self
    }

    /// Replace response decompression policy.
    pub fn compression(mut self, config: CompressionConfig) -> Self {
        self.compression = config;
        self
    }

    /// Replace WebSocket defaults.
    pub fn websocket_config(mut self, config: WebSocketConfig) -> Self {
        self.websocket_config = config;
        self
    }

    /// Reject non-HTTPS request URLs at execution time.
    pub fn https_only(mut self, enabled: bool) -> Self {
        self.https_only = enabled;
        self
    }

    /// Enable per-response fingerprint introspection.
    ///
    /// Off by default. When off, the execute path skips cloning the request
    /// headers, responses don't retain them, and [`Response::audit`] returns
    /// `None` — so high-throughput callers that never introspect pay nothing.
    /// Turn this on to populate [`Response::audit`] (JA3/JA4/JA4H/JA4T/H2) and
    /// [`Response::request_headers`].
    ///
    /// A registered [`observe`](crate::observe) response observer implies
    /// header retention regardless of this flag, since its snapshot needs
    /// them; this flag additionally gates the `audit()` fingerprint block.
    ///
    /// [`Response::audit`]: crate::Response::audit
    /// [`Response::request_headers`]: crate::Response::request_headers
    pub fn audit(mut self, enabled: bool) -> Self {
        self.audit = enabled;
        self
    }

    /// Add a PEM CA file or bundle to the TLS trust store.
    pub fn add_root_certificate_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.tls_trust = self.tls_trust.add_ca_file(path);
        self
    }

    /// Add a DER-encoded CA certificate to the TLS trust store.
    pub fn add_root_certificate_der(mut self, der: impl Into<Vec<u8>>) -> Self {
        self.tls_trust = self.tls_trust.add_ca_der(der);
        self
    }

    /// Add a SHA-256 pin for the DER-encoded leaf certificate.
    ///
    /// The certificate chain must still validate against configured
    /// roots; this adds a leaf hash requirement on top.
    pub fn add_pinned_leaf_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.tls_trust = self.tls_trust.add_pinned_leaf_sha256(sha256);
        self
    }

    /// Do not honour `SSL_CERT_FILE` / `SSL_CERT_DIR` for this session.
    pub fn without_env_roots(mut self) -> Self {
        self.tls_trust = self.tls_trust.without_env_roots();
        self
    }

    /// Do not load platform system roots for this session.
    pub fn without_system_roots(mut self) -> Self {
        self.tls_trust = self.tls_trust.without_system_roots();
        self
    }

    /// Use a PEM client certificate chain and private key for mTLS.
    pub fn client_identity_files(
        mut self,
        certificate_chain_file: impl Into<std::path::PathBuf>,
        private_key_file: impl Into<std::path::PathBuf>,
    ) -> Self {
        self.tls_trust = self
            .tls_trust
            .client_identity_files(certificate_chain_file, private_key_file);
        self
    }

    /// Force HTTP/3 over QUIC.
    ///
    /// ```rust,ignore
    /// let session = Session::builder()
    ///     .browser(Browser::Chrome147)
    ///     .http3()
    ///     .build()?;
    /// ```
    pub fn http3(mut self) -> Self {
        #[cfg(feature = "http3")]
        {
            self.protocol_policy = ProtocolPolicy::Http3;
        }
        #[cfg(not(feature = "http3"))]
        {
            self.config_error = Some(
                "HTTP/3 support requires the `http3` feature; rebuild leyline with feature `http3`"
                    .into(),
            );
        }
        self
    }

    /// Force HTTP/1.1.
    pub fn http1(mut self) -> Self {
        self.protocol_policy = ProtocolPolicy::Http1;
        self
    }

    /// Force HTTP/2.
    pub fn http2(mut self) -> Self {
        self.protocol_policy = ProtocolPolicy::Http2;
        self
    }

    /// Race the HTTP/3 (QUIC) and HTTP/2 (TCP+TLS) handshakes, Chrome-style:
    /// whichever connection establishes first carries the request (sent once),
    /// with HTTP/1.1 fallback via the Auto path when neither comes up.
    pub fn race(mut self) -> Self {
        #[cfg(feature = "http3")]
        {
            self.protocol_policy = ProtocolPolicy::Race;
        }
        #[cfg(not(feature = "http3"))]
        {
            self.config_error = Some(
                "HTTP/3 race support requires the `http3` feature; rebuild leyline with feature `http3`"
                    .into(),
            );
        }
        self
    }

    /// Set the protocol selection policy.
    pub fn protocol_policy(mut self, policy: ProtocolPolicy) -> Self {
        self.protocol_policy = policy;
        self
    }

    /// Override the session's `accept-language` header for every request.
    /// When not called, the browser profile's TOML identity value is used.
    pub fn accept_language(mut self, lang: impl Into<String>) -> Self {
        self.accept_language_override = Some(lang.into());
        self
    }

    /// Append session-level default headers injected on every request after
    /// the preset identity block. Use this to carry per-account client hints
    /// (`device-memory`, `viewport-width`, `dpr`, etc.) without touching
    /// every request site in the module. Per-request `.header()` calls and
    /// preset-emitted names take precedence over these defaults.
    pub fn extra_headers(mut self, headers: impl IntoIterator<Item = (String, String)>) -> Self {
        self.extra_identity_headers.extend(headers);
        self
    }

    /// Disable peer certificate verification. **Dangerous** - any
    /// man-in-the-middle between the client and the target can serve
    /// arbitrary content without detection. Intended only for the
    /// `leyline` CLI's `-k/--insecure` flag and controlled test
    /// fixtures against self-signed local servers.
    ///
    /// The method is named with a `danger_` prefix so it is grep-able
    /// in audit reviews - if you see this called in production code,
    /// that is itself a finding.
    ///
    /// Also skips system trust-store wiring at build time (the roots
    /// would never be consulted), so a machine with an unloadable
    /// system store can still build a session with this flag set.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Build the session.
    pub fn build(mut self) -> Result<Session> {
        if let Some(error) = self.config_error.take() {
            return Err(Error::Config(error));
        }

        // Resolve the effective platform:
        //  - explicit `.platform(...)` always wins (resolving `Host` if set);
        //  - an impersonation profile with no explicit platform defaults to
        //    Windows (dominant real-user OS) and warns once;
        //  - a bare session follows the host OS (honest for internal calls).
        self.platform = if self.platform_explicit {
            self.platform.resolve()
        } else if self.browser.is_some() {
            static NOTICE: std::sync::Once = std::sync::Once::new();
            NOTICE.call_once(|| {
                tracing::info!(
                    target: "leyline::session",
                    "no .platform() set on an impersonation profile — defaulting to \
                     Windows; call .platform(...) to pin the OS identity"
                );
            });
            Platform::Windows
        } else {
            Platform::detect_host()
        };

        // Honour standard proxy environment variables when no explicit
        // `proxy(..)` has been set. A CLI user exporting `HTTPS_PROXY`
        // gets it picked up without re-plumbing the session. The
        // precedence order is:
        //   1. Explicit `SessionBuilder::proxy(..)` (highest).
        //   2. `HTTPS_PROXY` (upper or lower case).
        //   3. `HTTP_PROXY` (upper or lower case).
        // `NO_PROXY` is honoured per-request in the transport layer
        // Building the session with a proxy plus `NO_PROXY`
        // patterns means some hosts bypass it.
        if self.proxy.is_none() && self.proxy_config.uses_env() {
            if let Some(p) = env_proxy() {
                self.proxy_config = self.proxy_config.set_default_proxy(p.clone());
                self.proxy = Some(p);
                // Provenance matters: env-inherited NO_PROXY patterns may
                // bypass this proxy, but never an explicitly-set one.
                self.proxy_from_env = true;
            }
        }
        // HTTP/3 and proxies are mutually exclusive by design. Tunnelling QUIC
        // (UDP) through a forward proxy requires MASQUE (RFC 9298 — UDP over
        // HTTP), a large separate protocol effort no mainstream proxy speaks; an
        // HTTP CONNECT / SOCKS proxy carries only TCP. So when a proxy is set,
        // the correct path is HTTP/2 over the proxy's TCP tunnel — request that
        // explicitly rather than silently dialing UDP direct (which would leak
        // the real egress IP past the proxy). Fail fast at build time.
        #[cfg(feature = "http3")]
        {
            if (self.proxy.is_some() || self.proxy_config.first_proxy().is_some())
                && matches!(self.protocol_policy, ProtocolPolicy::Http3)
            {
                return Err(Error::Config(
                    "HTTP/3 cannot run over a proxy (QUIC/UDP needs MASQUE, which proxies don't \
                     speak): drop `.http3()` to use HTTP/2 over the proxy's CONNECT tunnel, or \
                     drop `.proxy(...)` to dial HTTP/3 direct"
                        .into(),
                ));
            }
        }

        // `None` browser = bare (the default): a synthetic, non-impersonating
        // profile. `Some(b)` = impersonate that browser from the registry.
        let profile: &'static crate::profile::BrowserProfile = match self.browser {
            Some(b) => PROFILES
                .get_browser(b)
                .ok_or_else(|| Error::Config(format!("no profile for {b}")))?,
            None => &BARE_PROFILE,
        };

        let browser_label = self
            .browser
            .map(|b| b.to_string())
            .unwrap_or_else(|| "bare".to_string());

        let mut identity = profile
            .identity_for(self.platform)
            .ok_or_else(|| {
                Error::Config(format!("no {} identity for {browser_label}", self.platform))
            })?
            .clone();

        // Apply the Chromium-sibling identity overlay if the caller
        // asked for one. Only meaningful on Chrome profiles;
        // `chromium_major()` returns None for Firefox / Safari /
        // OkHttp, which short-circuits the overlay to a no-op. An
        // unverified (brand, Chromium, platform) combination
        // returns an error rather than silently emitting headers
        // we haven't captured against a real browser.
        let mut brand_extra_headers: Vec<(String, String)> = Vec::new();
        let mut brand_navigate_accept: Option<String> = None;
        if self.brand != ChromiumBrand::Chrome {
            // A brand overlay only applies to an explicit Chromium browser;
            // bare sessions have no browser and never carry brand headers.
            if let Some(chromium_major) = self.browser.and_then(|b| b.chromium_major()) {
                let overlay = self
                    .brand
                    .overlay(
                        chromium_major,
                        self.platform,
                        &identity.user_agent,
                        &identity.sec_ch_ua,
                    )
                    .map_err(|e| Error::Config(format!("{e}")))?;
                if let Some(overlay) = overlay {
                    identity.user_agent = overlay.user_agent;
                    identity.sec_ch_ua = overlay.sec_ch_ua;
                    brand_extra_headers = overlay.extra_headers;
                    brand_navigate_accept = overlay.navigate_accept;
                }
            }
        }

        let tcp_profile = self
            .tcp_profile
            .unwrap_or_else(|| self.platform.tcp_profile());

        let cookie_jar = self.cookie_jar.unwrap_or_default();

        // Build TLS connector from profile. When peer verification is
        // disabled, skip system trust-store wiring entirely: loading roots
        // we will never verify against is pointless, and an unloadable
        // store (e.g. a broken Windows ROOT hive) would otherwise fail the
        // build before `set_accept_invalid_certs` ever runs — killing the
        // `-k` escape hatch on exactly the machines that need it.
        let tls_trust = if self.accept_invalid_certs {
            self.tls_trust.clone().without_system_roots()
        } else {
            self.tls_trust.clone()
        };
        // Every session uses the BoringSSL fingerprint connector: browser/
        // profile sessions impersonate that browser, and a bare
        // `Session::new()` uses the synthetic `BARE_PROFILE` resolved above.
        let mut fp = FingerprintConnector::new_with_trust(profile, tcp_profile, &tls_trust)
            .map_err(Error::Tls)?;
        if self.accept_invalid_certs {
            fp.set_accept_invalid_certs(true);
        }
        fp = fp.with_resolver(self.dns_config.clone().into_resolver());
        fp = fp.with_socket_config(self.socket_config.clone());
        if let Some(connect_timeout) = self.timeouts.connect {
            fp = fp.with_connect_timeout(connect_timeout);
        }
        if let Some(config) = self.happy_eyeballs {
            fp = fp.with_happy_eyeballs_config(config);
        }
        let connector = ConnectorVariant::Fingerprint(fp);

        // Build H2 config from profile, applying any per-platform
        // override (e.g. Chromium-on-macOS drops `unknown_setting8`).
        let resolved_h2 = profile.h2.resolve_for_platform(self.platform)?;
        let h2_config = H2Config::from_profile(&resolved_h2)?;

        // Pre-compute audit data from profile.
        let extension_ids = crate::audit::chrome_extension_ids(&profile.tls);
        let ja4 = {
            let input = crate::audit::Ja4Input {
                ciphers: &profile.tls.ciphers,
                sigalgs: &profile.tls.sigalgs,
                curves: &profile.tls.curves,
                extension_ids: &extension_ids,
                tls_version: "1.3",
                has_sni: true,
                alpn: "h2",
            };
            crate::audit::compute_ja4(&input)
        };
        let ja3 = {
            let input = crate::audit::Ja3Input {
                ciphers: &profile.tls.ciphers,
                curves: &profile.tls.curves,
                extension_ids: &extension_ids,
                tls_record_version: 771, // TLS 1.2 record layer
            };
            crate::audit::compute_ja3(&input)
        };
        let h2_fp = h2_config.akamai_fingerprint();
        let is_windows = self.platform == Platform::Windows;
        let ja4t = crate::audit::compute_ja4t(
            tcp_profile.window_size,
            tcp_profile.mss as u16,
            tcp_profile.window_scale as u8,
            is_windows,
        );

        Ok(Session {
            inner: std::sync::Arc::new(SessionInner {
                browser: self.browser,
                platform: self.platform,
                brand: self.brand,
                user_agent: identity.user_agent,
                sec_ch_ua: identity.sec_ch_ua,
                accept_language: self.accept_language_override.unwrap_or_else(|| {
                    identity
                        .accept_language
                        .unwrap_or_else(|| "en-US,en;q=0.9".to_string())
                }),
                brand_extra_headers,
                brand_navigate_accept,
                identity_extra_headers: {
                    let mut h = identity.extra_headers.clone();
                    h.extend(self.extra_identity_headers);
                    h
                },
                identity_navigate_accept: identity.navigate_accept_override.clone(),
                identity_request_header_order: identity.request_header_order.clone(),
                proxy: self.proxy,
                proxy_from_env: self.proxy_from_env,
                max_redirects: self.max_redirects,
                proxy_config: self.proxy_config,
                timeouts: self.timeouts,
                redirect_policy: self.redirect_policy,
                compression: self.compression,
                #[cfg(feature = "websocket")]
                websocket_config: self.websocket_config,
                https_only: self.https_only,
                cookie_jar,
                connector,
                h2_config,
                pool: Arc::new(if self.pool_config.keepalive {
                    Pool::with_limits(
                        self.pool_config.idle_timeout,
                        self.pool_config.max_connections.max(1),
                        self.pool_config.max_h1_conns_per_host.max(1),
                    )
                } else {
                    // Keepalive disabled: zero idle timeout means connections are
                    // never reused. The per-host H1 cap still applies (it governs
                    // concurrency, not reuse), so honour the configured value
                    // rather than forcing it to 1 — disabling reuse must not
                    // silently serialise concurrent H1 requests to a host.
                    Pool::with_limits(
                        std::time::Duration::ZERO,
                        1,
                        self.pool_config.max_h1_conns_per_host.max(1),
                    )
                }),
                audit_tls: Arc::new(AuditTlsCache {
                    ja4,
                    ja3,
                    h2_fingerprint: h2_fp,
                    ja4t,
                }),
                audit_enabled: self.audit,
                protocol_policy: self.protocol_policy,
                default_retry: self.default_retry,
                #[cfg(feature = "http3")]
                h3_config: match crate::quic::H3Config::for_family(&profile.meta.family) {
                    Ok(cfg) => Some(cfg),
                    // No H3 fingerprint for this family — only fatal if HTTP/3 was requested.
                    Err(e)
                        if matches!(
                            self.protocol_policy,
                            ProtocolPolicy::Http3 | ProtocolPolicy::Race
                        ) =>
                    {
                        return Err(e);
                    }
                    Err(_) => None,
                },
                #[cfg(feature = "http3")]
                profile,
            }),
        })
    }
}
