use std::sync::Arc;

use crate::cookie::Jar;
use crate::core::config::HostLimits;
use crate::core::proxy_pool::ProxyPool;
use crate::core::{
    CompressionConfig, DnsConfig, IntoParamPair, IntoUrl, PoolConfig, ProxyConfig, RedirectPolicy,
    SocketConfig, TimeoutConfig, WebSocketConfig,
};
use crate::profile::{Browser, BrowserProfile, ChromiumBrand, Platform};
use crate::tcp::TcpProfile;
use crate::tls::TlsTrustConfig;
use crate::trace::Trace;

pub(crate) use self::bearer::BearerToken;
use self::derive::{IdentityInputs, IdentitySource, derive_identity};
use super::proxy::InvalidEnvProxy;
use super::{Identity, ProtocolPolicy, Session};
use crate::core::error::Result;

const AUTHORIZATION: &str = "authorization";
const USER_AGENT: &str = "user-agent";

mod assemble;
mod bearer;
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
    expected_profile_id: Option<String>,
    websocket_config: WebSocketConfig,
    https_only: bool,
    audit: bool,
    cookie_jar: Option<Jar>,
    tcp_profile: Option<TcpProfile>,
    protocol_policy: ProtocolPolicy,
    #[cfg(feature = "http3")]
    protocol_explicit: bool,
    config_error: Option<String>,
    default_headers: Vec<(String, String)>,
    bearer: Option<BearerToken>,
    http_identity: Option<Browser>,
    tls_trust: TlsTrustConfig,
    default_retry: crate::core::retry::RetryPolicy,
    trace: Option<Arc<dyn Trace>>,
    base_url: Option<url::Url>,
    languages: Option<Vec<String>>,
    host_limits: HostLimits,
    proxy_pool: Option<ProxyPool>,
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
            expected_profile_id: None,
            websocket_config: WebSocketConfig::default(),
            https_only: false,
            audit: false,
            cookie_jar: None,
            tcp_profile: None,
            protocol_policy: ProtocolPolicy::Auto,
            #[cfg(feature = "http3")]
            protocol_explicit: false,
            config_error: None,
            default_headers: Vec::new(),
            bearer: None,
            http_identity: None,
            tls_trust: TlsTrustConfig::default(),
            default_retry: crate::core::retry::RetryPolicy::none(),
            trace: None,
            base_url: None,
            languages: None,
            host_limits: HostLimits::default(),
            proxy_pool: None,
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

    fn identity_inputs<'a>(&'a self, tcp: &'a TcpProfile) -> IdentityInputs<'a> {
        IdentityInputs {
            source: self.identity_source(),
            platform: self.platform,
            brand: self.brand,
            compression: self.compression,
            #[cfg(feature = "http3")]
            h3_required: self.protocol_policy.requires_h3(),
            tcp,
            audit: self.audit,
            default_headers: &self.default_headers,
            languages: self.languages.as_deref(),
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

    pub fn expect_profile_id(mut self, id: &str) -> Self {
        self.expected_profile_id = Some(id.to_owned());
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
        #[cfg(feature = "http3")]
        {
            self.protocol_explicit = true;
        }
        self
    }

    pub fn identity(mut self, id: Identity) -> Self {
        self = self.browser(id.tls()).platform(id.platform());
        self.http_identity = (id.http() != id.tls()).then(|| id.http());
        if let Some(brand) = id.brand() {
            self.brand = brand;
        }
        self
    }

    pub fn user_agent(mut self, value: &str) -> Self {
        self.check_header(USER_AGENT, value);
        self.remove_default(USER_AGENT);
        self.default_headers
            .push((USER_AGENT.to_owned(), value.to_owned()));
        self
    }

    pub fn languages<I, S>(mut self, langs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let tags: Vec<String> = langs
            .into_iter()
            .map(|tag| tag.as_ref().trim().to_owned())
            .collect();
        let checked = if tags.is_empty() {
            Err("languages needs at least one language tag".to_owned())
        } else {
            tags.iter()
                .try_for_each(|tag| crate::profile::languages::validate_language(tag))
        };
        if let Err(error) = checked {
            self.config_error.get_or_insert(error);
        }
        self.languages = Some(tags);
        self
    }

    pub fn host_limits(mut self, limits: HostLimits) -> Self {
        if let Some(error) = limits.config_error() {
            self.config_error.get_or_insert(error);
        }
        self.host_limits = limits;
        self
    }

    pub fn proxy_pool(mut self, pool: ProxyPool) -> Self {
        self.proxy_pool = Some(pool);
        self
    }

    pub fn base_url(mut self, url: impl IntoUrl) -> Self {
        match super::helpers::base_url(url) {
            Ok(url) => self.base_url = Some(url),
            Err(error) => {
                self.base_url = None;
                self.config_error.get_or_insert(error.to_string());
            }
        }
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
            self.push_default(name, value);
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
        let expected = self.expected_profile_id.take();
        let session = self.into_session(derived, connector);
        Self::check_profile_id(expected.as_deref(), session.identity().profile_id())?;
        Ok(session)
    }
}
