use std::sync::Arc;

use crate::cookie::Jar;
use crate::core::{
    CompressionConfig, DnsConfig, IntoParamPair, PoolConfig, ProxyConfig, RedirectPolicy,
    SocketConfig, TimeoutConfig, WebSocketConfig,
};
use crate::profile::{Browser, BrowserProfile, ChromiumBrand, Platform};
use crate::tcp::TcpProfile;
use crate::tls::TlsTrustConfig;
use crate::trace::Trace;

use self::derive::{IdentityInputs, IdentitySource, derive_identity};
use super::proxy::InvalidEnvProxy;
use super::{Identity, ProtocolPolicy, Session};
use crate::core::error::Result;

mod assemble;
pub(super) mod connect;
mod debug;
pub(super) mod derive;
mod validate;

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
        self.http_identity = None;
        self.browser = Some(browser);
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

    fn identity_source(&self) -> IdentitySource {
        match (&self.profile, self.browser) {
            (Some(profile), _) => IdentitySource::Profile(Arc::clone(profile)),
            (None, Some(tls)) => IdentitySource::Browser {
                tls,
                http: self.http_identity.unwrap_or(tls),
            },
            (None, None) => IdentitySource::Bare,
        }
    }

    fn identity_inputs<'a>(&self, tcp: &'a TcpProfile) -> IdentityInputs<'a> {
        IdentityInputs {
            source: self.identity_source(),
            platform: self.platform,
            brand: self.brand,
            compression: self.compression,
            #[cfg(feature = "http3")]
            h3_required: self.protocol_policy.requires_h3(),
            tcp,
            audit: self.audit,
        }
    }

    pub fn brand(mut self, brand: ChromiumBrand) -> Self {
        self.brand = brand;
        self
    }

    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self.platform_explicit = true;
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
        self.build_with(InvalidEnvProxy::FailRequests)
            .expect("bundled profile data is statically valid; default trust loading only warns")
    }

    pub fn build(self) -> Result<Session> {
        self.build_with(InvalidEnvProxy::FailBuild)
    }

    fn build_with(mut self, on_invalid: InvalidEnvProxy) -> Result<Session> {
        self.validate(on_invalid)?;
        let tcp_profile = self.resolve_tcp_profile();
        let derived = derive_identity(self.identity_inputs(&tcp_profile))?;
        let connector = self.build_connector(&derived.profile, &tcp_profile)?;
        Ok(self.into_session(derived, connector))
    }
}
