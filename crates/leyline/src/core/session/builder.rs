use std::sync::{Arc, LazyLock};

use crate::cookie::Jar;
#[cfg(feature = "tower")]
use crate::core::layer::{Call, Hold, Reply, Stack, Transport};
use crate::core::{
    CompressionConfig, DnsConfig, IntoParamPair, NoProxy, PoolConfig, ProxyConfig, ProxyUrl,
    RedirectPolicy, SocketConfig, TimeoutConfig, WebSocketConfig,
};
use crate::h2::H2Config;
use crate::pool::Pool;
use crate::profile::{Browser, BrowserProfile, ChromiumBrand, Platform, ProfileRegistry};
use crate::tcp::TcpProfile;
use crate::tls::{FingerprintConnector, HappyEyeballsConfig, Resolver, TlsTrustConfig};
use crate::trace::Trace;

use super::proxy::env_proxy;
use super::{Identity, ProtocolPolicy, Session, SessionInner};
use crate::audit::AuditTlsCache;
use crate::core::error::{Error, Kind, Result};

static BARE_PROFILE: LazyLock<BrowserProfile> = LazyLock::new(BrowserProfile::bare);

#[must_use = "builders are lazy: nothing happens until `.send()` / `.build()`"]
pub struct SessionBuilder {
    browser: Option<Browser>,
    platform: Platform,
    platform_explicit: bool,
    brand: ChromiumBrand,
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
    cookie_jar: Option<Jar>,
    tcp_profile: Option<TcpProfile>,
    protocol_policy: ProtocolPolicy,
    protocol_explicit: bool,
    config_error: Option<String>,
    accept_language_override: Option<String>,
    extra_identity_headers: Vec<(String, String)>,
    http_identity: Option<Browser>,
    accept_invalid_certs: bool,
    happy_eyeballs: Option<HappyEyeballsConfig>,
    tls_trust: TlsTrustConfig,
    default_retry: crate::core::retry::RetryPolicy,
    trace: Option<Arc<dyn Trace>>,
    #[cfg(feature = "tower")]
    layer: Option<Arc<dyn Stack>>,
}

type BrandOverlayEdits = (Vec<(String, String)>, Option<String>);

impl SessionBuilder {
    pub(super) fn new() -> Self {
        Self {
            browser: None,
            platform: Platform::default(),
            platform_explicit: false,
            brand: ChromiumBrand::default(),
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
            protocol_explicit: false,
            config_error: None,
            accept_language_override: None,
            extra_identity_headers: Vec::new(),
            http_identity: None,
            accept_invalid_certs: false,
            happy_eyeballs: None,
            tls_trust: TlsTrustConfig::default(),
            default_retry: crate::core::retry::RetryPolicy::none(),
            trace: None,
            #[cfg(feature = "tower")]
            layer: None,
        }
    }

    pub fn proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy_config = self.proxy_config.set_default_proxy(proxy);
        self
    }

    pub fn proxies(mut self, config: ProxyConfig) -> Self {
        self.proxy_config = config;
        self
    }

    pub fn no_proxy(mut self, no_proxy: NoProxy) -> Self {
        self.proxy_config = self.proxy_config.no_proxy(no_proxy);
        self
    }

    pub fn disable_env_proxies(mut self) -> Self {
        self.proxy_config = self.proxy_config.without_env();
        self
    }

    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeouts.total = timeout;
        self
    }

    pub fn retry(mut self, policy: crate::core::retry::RetryPolicy) -> Self {
        self.default_retry = policy;
        self
    }

    pub fn timeouts(mut self, config: TimeoutConfig) -> Self {
        self.timeouts = config;
        self
    }

    pub fn connect_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeouts.connect = Some(timeout);
        self
    }

    pub fn pool_config(mut self, config: PoolConfig) -> Self {
        self.pool_config = config;
        self
    }

    pub fn browser(mut self, browser: Browser) -> Self {
        self.browser = Some(browser);
        self
    }

    pub fn chrome(self) -> Self {
        self.browser(Browser::default_browser()).with_h3_race()
    }

    fn with_h3_race(self) -> Self {
        #[cfg(feature = "http3")]
        {
            if self.protocol_explicit {
                self
            } else {
                self.race()
            }
        }
        #[cfg(not(feature = "http3"))]
        {
            self
        }
    }

    fn chromium_or_default(self, default: Browser) -> Self {
        if self.browser.is_none() {
            self.browser(default)
        } else {
            self
        }
    }

    pub fn firefox(self) -> Self {
        self.browser(Browser::default_firefox())
    }

    pub fn safari(self) -> Self {
        if self.platform_explicit {
            let platform = self.platform;
            self.browser(Browser::Safari26.for_platform(platform))
        } else {
            self.browser(Browser::Safari26).macos()
        }
    }

    pub fn profile(self, browser: Browser, platform: Platform) -> Self {
        self.browser(browser).platform(platform)
    }

    pub fn brand(mut self, brand: ChromiumBrand) -> Self {
        self.brand = brand;
        self
    }

    pub fn edge(self) -> Self {
        self.chromium_or_default(Browser::default_browser())
            .with_h3_race()
            .brand(ChromiumBrand::Edge)
    }

    pub fn brave(self) -> Self {
        let this = if self.platform_explicit {
            let platform = self.platform;
            self.browser(Browser::Brave146.for_platform(platform))
        } else {
            self.browser(Browser::Brave146).macos()
        };
        this.with_h3_race()
    }

    pub fn opera(self) -> Self {
        self.chromium_or_default(Browser::default_browser())
            .with_h3_race()
            .brand(ChromiumBrand::Opera)
    }

    pub fn vivaldi(self) -> Self {
        self.chromium_or_default(Browser::Chrome147)
            .with_h3_race()
            .brand(ChromiumBrand::Vivaldi)
    }

    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self.platform_explicit = true;
        if let Some(browser) = self.browser {
            self.browser = Some(browser.for_platform(platform));
        }
        self
    }

    pub fn windows(self) -> Self {
        self.platform(Platform::Windows)
    }

    pub fn macos(self) -> Self {
        self.platform(Platform::MacOS)
    }

    pub fn linux(self) -> Self {
        self.platform(Platform::Linux)
    }

    pub fn android(self) -> Self {
        self.platform(Platform::Android)
    }

    pub fn ios(self) -> Self {
        self.platform(Platform::IOS)
    }

    pub fn max_redirects(mut self, n: usize) -> Self {
        self.redirect_policy = RedirectPolicy::limited(n);
        self
    }

    pub fn redirect_policy(mut self, policy: RedirectPolicy) -> Self {
        self.redirect_policy = policy;
        self
    }

    pub fn cookie_jar(mut self, jar: Jar) -> Self {
        self.cookie_jar = Some(jar);
        self
    }

    pub fn tcp_profile(mut self, profile: TcpProfile) -> Self {
        self.tcp_profile = Some(profile);
        self
    }

    pub fn resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.dns_config = self.dns_config.resolver(resolver);
        self
    }

    pub fn dns(mut self, config: DnsConfig) -> Self {
        self.dns_config = config;
        self
    }

    pub fn resolve_host(mut self, host: impl AsRef<str>, addr: std::net::SocketAddr) -> Self {
        self.dns_config = self.dns_config.resolve_host(host, addr);
        self
    }

    pub fn resolve_host_to_addrs<I>(mut self, host: impl AsRef<str>, addrs: I) -> Self
    where
        I: IntoIterator<Item = std::net::SocketAddr>,
    {
        self.dns_config = self.dns_config.resolve_host_to_addrs(host, addrs);
        self
    }

    pub fn happy_eyeballs(mut self, config: HappyEyeballsConfig) -> Self {
        self.happy_eyeballs = Some(config);
        self
    }

    pub fn tls_trust(mut self, trust: TlsTrustConfig) -> Self {
        self.tls_trust = trust;
        self
    }

    pub fn socket_config(mut self, config: SocketConfig) -> Self {
        self.socket_config = config;
        self
    }

    pub fn compression(mut self, config: CompressionConfig) -> Self {
        self.compression = config;
        self
    }

    pub fn websocket_config(mut self, config: WebSocketConfig) -> Self {
        self.websocket_config = config;
        self
    }

    pub fn https_only(mut self, enabled: bool) -> Self {
        self.https_only = enabled;
        self
    }

    pub fn trace(mut self, hook: impl Trace) -> Self {
        self.trace = Some(Arc::new(hook));
        self
    }

    #[cfg(feature = "tower")]
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: tower_layer::Layer<Transport>,
        L::Service: tower_service::Service<Call, Response = Reply, Error = Error>
            + Clone
            + Send
            + Sync
            + 'static,
        <L::Service as tower_service::Service<Call>>::Future: Send + 'static,
    {
        self.layer = Some(Arc::new(Hold(layer.layer(Transport))));
        self
    }

    pub fn audit(mut self, enabled: bool) -> Self {
        self.audit = enabled;
        self
    }

    pub fn add_root_certificate_file(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.tls_trust = self.tls_trust.add_ca_file(path);
        self
    }

    pub fn add_root_certificate_der(mut self, der: impl Into<Vec<u8>>) -> Self {
        self.tls_trust = self.tls_trust.add_ca_der(der);
        self
    }

    pub fn add_pinned_leaf_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.tls_trust = self.tls_trust.add_pinned_leaf_sha256(sha256);
        self
    }

    pub fn without_env_roots(mut self) -> Self {
        self.tls_trust = self.tls_trust.without_env_roots();
        self
    }

    pub fn without_system_roots(mut self) -> Self {
        self.tls_trust = self.tls_trust.without_system_roots();
        self
    }

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

    pub fn http3(mut self) -> Self {
        self.protocol_explicit = true;
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

    pub fn http1(mut self) -> Self {
        self.protocol_explicit = true;
        self.protocol_policy = ProtocolPolicy::Http1;
        self
    }

    pub fn http2(mut self) -> Self {
        self.protocol_explicit = true;
        self.protocol_policy = ProtocolPolicy::Http2;
        self
    }

    pub fn race(mut self) -> Self {
        self.protocol_explicit = true;
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

    pub fn protocol_policy(mut self, policy: ProtocolPolicy) -> Self {
        self.protocol_explicit = true;
        self.protocol_policy = policy;
        self
    }

    pub fn identity(self, id: Identity) -> Self {
        let builder = self.browser(id.tls()).platform(id.platform());
        if id.http() == id.tls() {
            builder
        } else {
            builder.http_identity(id.http())
        }
    }

    pub fn http_identity(mut self, browser: Browser) -> Self {
        self.http_identity = Some(browser);
        self
    }

    pub fn accept_language(mut self, lang: impl Into<String>) -> Self {
        let lang = lang.into();
        self.check_header("accept-language", &lang);
        self.accept_language_override = Some(lang);
        self
    }

    pub fn extra_headers<I, P>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        for pair in headers {
            let (name, value) = pair.into_param_pair();
            self.check_header(&name, &value);
            self.extra_identity_headers.push((name, value));
        }
        self
    }

    fn check_header(&mut self, name: &str, value: &str) {
        if self.config_error.is_none()
            && (crate::core::headers::name(name).is_err()
                || crate::core::headers::value(value).is_err())
        {
            self.config_error = Some(format!("invalid header `{}`", name.escape_debug()));
        }
    }

    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    pub(super) fn into_builtin(self) -> Session {
        match self.build() {
            Ok(session) => session,
            Err(err) => unreachable!("bundled profile failed to build: {err}"),
        }
    }

    pub fn build(mut self) -> Result<Session> {
        if let Some(error) = self.config_error.take() {
            return Err(Error::new(Kind::Config).with_message(error));
        }
        if let (Some(tls), Some(http)) = (self.browser, self.http_identity)
            && tls.family() != http.family()
        {
            return Err(Error::new(Kind::Config).with_message(format!(
                "http identity {http} is not the same family as TLS {tls}"
            )));
        }

        self.platform = if self.platform_explicit {
            self.platform.resolve()
        } else if self.browser.is_some() {
            static NOTICE: std::sync::Once = std::sync::Once::new();
            NOTICE.call_once(|| {
                tracing::info!(
                    target: "leyline::session",
                    "no .platform() set on an impersonation profile: defaulting to \
                     Windows; call .platform(...) to pin the OS identity"
                );
            });
            Platform::Windows
        } else {
            Platform::detect_host()
        };

        for rule in self.proxy_config.rules() {
            ProxyUrl::parse(rule.url())?;
        }
        if self.proxy_config.primary().is_none()
            && self.proxy_config.uses_env()
            && let Some(p) = env_proxy()
        {
            self.proxy_config = self.proxy_config.set_default_proxy(p).set_from_env();
        }
        #[cfg(feature = "http3")]
        {
            if self.proxy_config.primary().is_some()
                && matches!(self.protocol_policy, ProtocolPolicy::Http3)
            {
                return Err(Error::new(Kind::Config).with_message(
                    "HTTP/3 cannot run over a proxy (QUIC/UDP needs MASQUE, which proxies don't \
                     speak): drop `.http3()` to use HTTP/2 over the proxy's CONNECT tunnel, or \
                     drop `.proxy(...)` to dial HTTP/3 direct",
                ));
            }
        }

        let profile: &'static BrowserProfile = match self.browser {
            Some(b) => ProfileRegistry::global().get_browser(b).ok_or_else(|| {
                Error::new(Kind::Config).with_message(format!("no profile for {b}"))
            })?,
            None => &BARE_PROFILE,
        };

        let browser_label = self
            .browser
            .map(|b| b.to_string())
            .unwrap_or_else(|| "bare".to_string());

        let mut identity = if let Some(http_b) = self.http_identity {
            let http_profile = ProfileRegistry::global()
                .get_browser(http_b)
                .ok_or_else(|| {
                    Error::new(Kind::Config)
                        .with_message(format!("no profile for http identity {http_b}"))
                })?;
            http_profile
                .identity_for(self.platform)
                .ok_or_else(|| {
                    Error::new(Kind::Config).with_message(format!(
                        "no {} identity for http identity {http_b}",
                        self.platform
                    ))
                })?
                .clone()
        } else {
            profile
                .identity_for(self.platform)
                .ok_or_else(|| {
                    Error::new(Kind::Config)
                        .with_message(format!("no {} identity for {browser_label}", self.platform))
                })?
                .clone()
        };

        let (brand_extra_headers, brand_navigate_accept) =
            self.apply_brand_overlay(&mut identity)?;

        let tcp_profile = self.tcp_profile.unwrap_or_else(|| {
            let mut tcp = self.platform.tcp_profile();
            if self.browser.is_none() {
                tcp.mss = 0;
                tcp.window_size = 0;
                tcp.window_scale = 0;
            }
            tcp
        });

        let connector = self.build_connector(profile, tcp_profile)?;

        let resolved_h2 = profile.h2.resolve_for_platform(self.platform)?;
        let h2_config = H2Config::from_profile(&resolved_h2)?;

        let audit_cache = self
            .audit
            .then(|| Arc::new(self.compute_audit_cache(profile, &h2_config, tcp_profile)));

        let cookie_jar = self.cookie_jar.unwrap_or_default();

        let presentation = match self.browser {
            None => None,
            Some(tls) => {
                let http = self.http_identity.unwrap_or(tls);
                Some(
                    Identity::locked(http, self.platform)
                        .rotate_tls(tls)
                        .expect("build already rejected a family mismatch"),
                )
            }
        };

        Ok(Session {
            inner: std::sync::Arc::new(SessionInner {
                browser: self.browser,
                identity: presentation,
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
                proxy_config: self.proxy_config,
                timeouts: self.timeouts,
                redirect_policy: self.redirect_policy,
                compression: self.compression,
                #[cfg(feature = "websocket")]
                websocket_config: self.websocket_config,
                https_only: self.https_only,
                cookie_jar,
                url_cache: std::sync::Arc::new(std::sync::Mutex::new(None)),
                connector,
                h2_config,
                pool: Arc::new(if self.pool_config.keepalive {
                    Pool::with_limits(
                        self.pool_config.idle_timeout,
                        self.pool_config.max_connections.max(1),
                        self.pool_config.max_h1_conns_per_host.max(1),
                    )
                    .with_h2_ping(
                        self.pool_config.h2_ping_after_idle,
                        self.pool_config.h2_ping_timeout,
                    )
                } else {
                    Pool::with_limits(
                        std::time::Duration::ZERO,
                        1,
                        self.pool_config.max_h1_conns_per_host.max(1),
                    )
                    .with_h2_ping(
                        self.pool_config.h2_ping_after_idle,
                        self.pool_config.h2_ping_timeout,
                    )
                }),
                audit_tls: audit_cache,
                protocol_policy: self.protocol_policy,
                default_retry: self.default_retry,
                trace: self.trace,
                #[cfg(feature = "tower")]
                layer: self.layer,
                tls_trust: self.tls_trust.clone(),
                #[cfg(feature = "http3")]
                h3_config: match crate::quic::H3Config::for_family(&profile.meta.family) {
                    Ok(cfg) => Some(cfg),
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
    fn compute_audit_cache(
        &self,
        profile: &'static BrowserProfile,
        h2_config: &H2Config,
        tcp_profile: TcpProfile,
    ) -> AuditTlsCache {
        let extension_ids = crate::audit::extension_ids(&profile.tls);
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
                tls_record_version: 771,
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
        AuditTlsCache {
            ja4,
            ja3,
            h2_fingerprint: h2_fp,
            ja4t,
        }
    }

    fn build_connector(
        &self,
        profile: &'static BrowserProfile,
        tcp_profile: TcpProfile,
    ) -> Result<FingerprintConnector> {
        let tls_trust = if self.accept_invalid_certs {
            self.tls_trust.clone().without_system_roots()
        } else {
            self.tls_trust.clone()
        };
        let mut fp = FingerprintConnector::new_with_trust(profile, tcp_profile, &tls_trust)
            .map_err(Error::from)?;
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
        Ok(fp)
    }

    fn apply_brand_overlay(
        &self,
        identity: &mut crate::profile::PlatformIdentity,
    ) -> Result<BrandOverlayEdits> {
        let mut brand_extra_headers: Vec<(String, String)> = Vec::new();
        let mut brand_navigate_accept: Option<String> = None;
        if self.brand != ChromiumBrand::Chrome {
            let Some(chromium_major) = self
                .http_identity
                .or(self.browser)
                .and_then(|b| b.chromium_major())
            else {
                return Err(Error::new(Kind::Config).with_message(format!(
                    "{} overlay requires a Chromium HTTP identity",
                    self.brand.label()
                )));
            };
            let overlay = self
                .brand
                .overlay(chromium_major, self.platform, &identity.user_agent)
                .map_err(|e| Error::new(Kind::Config).with_message(format!("{e}")))?;
            if let Some(overlay) = overlay {
                identity.user_agent = overlay.user_agent;
                identity.sec_ch_ua = overlay.sec_ch_ua;
                brand_extra_headers = overlay.extra_headers;
                brand_navigate_accept = overlay.navigate_accept;
            }
        }
        Ok((brand_extra_headers, brand_navigate_accept))
    }
}
