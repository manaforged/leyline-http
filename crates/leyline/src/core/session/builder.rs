use std::sync::Arc;

use crate::cookie::Jar;
use crate::core::{
    CompressionConfig, DnsConfig, IntoParamPair, PoolConfig, ProxyConfig, ProxyUrl, RedirectPolicy,
    SocketConfig, TimeoutConfig, WebSocketConfig,
};
use crate::h2::H2Config;
use crate::profile::{
    Browser, BrowserProfile, ChromiumBrand, Platform, ResolvedIdentity, resolve_identity,
};
use crate::tcp::TcpProfile;
use crate::tls::TlsTrustConfig;
use crate::trace::Trace;

use super::proxy::env_proxy;
use super::{Identity, ProtocolPolicy, Session, SessionInner};
use crate::core::error::{Error, Kind, Result};

mod connect;

#[must_use = "builders are lazy: nothing happens until `.send()` / `.build()`"]
pub struct SessionBuilder {
    browser: Option<Browser>,
    profile: Option<Arc<BrowserProfile>>,
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
    config_error: Option<String>,
    default_headers: Vec<(String, String)>,
    http_identity: Option<Browser>,
    tls_trust: TlsTrustConfig,
    default_retry: crate::core::retry::RetryPolicy,
    trace: Option<Arc<dyn Trace>>,
}

impl SessionBuilder {
    pub(super) fn new() -> Self {
        Self {
            browser: None,
            profile: None,
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
            config_error: None,
            default_headers: Vec::new(),
            http_identity: None,
            tls_trust: TlsTrustConfig::default(),
            default_retry: crate::core::retry::RetryPolicy::none(),
            trace: None,
        }
    }

    pub fn proxy(mut self, config: impl Into<ProxyConfig>) -> Self {
        self.proxy_config = config.into();
        self
    }

    pub fn timeout(mut self, config: impl Into<TimeoutConfig>) -> Self {
        self.timeouts = config.into();
        self
    }

    pub fn retry(mut self, policy: crate::core::retry::RetryPolicy) -> Self {
        self.default_retry = policy;
        self
    }

    pub fn pool(mut self, config: PoolConfig) -> Self {
        self.pool_config = config;
        self
    }

    pub fn browser(mut self, browser: Browser) -> Self {
        self.profile = None;
        self.browser = Some(browser);
        if self.platform_explicit {
            self.browser = Some(browser.for_platform(self.platform));
        }
        self
    }

    pub fn profile(mut self, profile: BrowserProfile) -> Self {
        self.browser = None;
        self.http_identity = None;
        self.profile = Some(Arc::new(profile));
        self
    }

    fn impersonates(&self) -> bool {
        self.browser.is_some() || self.profile.is_some()
    }

    fn tls_profile(&self) -> Arc<BrowserProfile> {
        match (&self.profile, self.browser) {
            (Some(profile), _) => Arc::clone(profile),
            (None, Some(browser)) => browser.shared_profile(),
            (None, None) => BrowserProfile::bare_shared(),
        }
    }

    pub fn brand(mut self, brand: ChromiumBrand) -> Self {
        self.brand = brand;
        self
    }

    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self.platform_explicit = true;
        if let Some(browser) = self.browser {
            self.browser = Some(browser.for_platform(platform));
        }
        self
    }

    pub fn redirect(mut self, policy: RedirectPolicy) -> Self {
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

    pub fn dns(mut self, config: impl Into<DnsConfig>) -> Self {
        self.dns_config = config.into();
        self
    }

    pub fn tls_trust(mut self, trust: TlsTrustConfig) -> Self {
        self.tls_trust = trust;
        self
    }

    pub fn socket(mut self, config: SocketConfig) -> Self {
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

    pub fn audit(mut self, enabled: bool) -> Self {
        self.audit = enabled;
        self
    }

    pub fn protocol(mut self, policy: ProtocolPolicy) -> Self {
        self.protocol_policy = policy;
        self
    }

    pub fn identity(mut self, id: Identity) -> Self {
        self = self.browser(id.tls()).platform(id.platform());
        self.http_identity = (id.http() != id.tls()).then(|| id.http());
        self
    }

    pub fn headers<I, P>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
        for pair in headers {
            let (name, value) = pair.into_param_pair();
            self.check_header(&name, &value);
            self.default_headers.push((name, value));
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

    pub(super) fn into_builtin(self) -> Session {
        self.build()
            .expect("bundled profile data is statically valid; default trust loading only warns")
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
        } else if self.impersonates() {
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
        if self.proxy_config.rules().is_empty()
            && self.proxy_config.uses_env()
            && let Some(p) = env_proxy()
        {
            self.proxy_config = self.proxy_config.set_default_proxy(p).set_from_env();
        }
        #[cfg(feature = "http3")]
        {
            if self.proxy_config.proxies_every_url()
                && matches!(self.protocol_policy, ProtocolPolicy::Http3)
            {
                return Err(Error::new(Kind::Config).with_message(
                    "HTTP/3 cannot run over a proxy (QUIC/UDP needs MASQUE, which proxies don't \
                     speak): use another `.protocol(..)` to run HTTP/2 over the proxy's CONNECT tunnel, \
                     or drop `.proxy(..)` to dial HTTP/3 direct",
                ));
            }
        }

        let profile = self.tls_profile();
        let http_profile = match self.http_identity.or(self.browser) {
            Some(browser) => browser.platform_profile(self.platform),
            None => &*profile,
        };
        let header_style = http_profile.meta.header_style;

        let ResolvedIdentity {
            identity,
            brand_extra_headers,
            brand_navigate_accept,
        } = resolve_identity(http_profile, self.platform, self.brand)
            .map_err(|e| Error::new(Kind::Config).with_message(e.to_string()))?;

        let tcp_profile = self.tcp_profile.clone().unwrap_or_else(|| {
            let mut tcp = self.platform.tcp_profile();
            if !self.impersonates() {
                tcp.mss = 0;
                tcp.window_size = 0;
                tcp.window_scale = 0;
            }
            tcp
        });

        let connector = self.build_connector(&profile, &tcp_profile)?;

        let resolved_h2 = profile.h2.resolve_for_platform(self.platform)?;
        let mut h2_config = H2Config::from_profile(&resolved_h2)?;
        h2_config.max_response_body_bytes = self.compression.max_body_size;
        #[cfg(feature = "http3")]
        let h3_config = self.h3_config(&profile)?;

        let audit_cache = self
            .audit
            .then(|| Arc::new(self.compute_audit_cache(&profile, &h2_config, &tcp_profile)));

        let impersonates = self.impersonates();
        let pool = Arc::new(self.build_pool());
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
                impersonates,
                header_style,
                identity: presentation,
                platform: self.platform,
                brand: self.brand,
                user_agent: identity.user_agent,
                sec_ch_ua: identity.sec_ch_ua,
                accept_language: identity
                    .accept_language
                    .unwrap_or_else(|| "en-US,en;q=0.9".to_string()),
                brand_extra_headers,
                brand_navigate_accept,
                identity_extra_headers: {
                    let mut h = identity.extra_headers.clone();
                    h.extend(self.default_headers);
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
                pool,
                audit_tls: audit_cache,
                protocol_policy: self.protocol_policy,
                default_retry: self.default_retry,
                trace: self.trace,
                tls_trust: self.tls_trust.clone(),
                #[cfg(feature = "http3")]
                h3_config,
                profile,
            }),
        })
    }
}
