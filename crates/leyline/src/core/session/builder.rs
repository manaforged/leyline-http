use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::cookie::Jar;
use crate::core::block::Rules;
use crate::core::config::HostLimits;
use crate::core::proxy_pool::ProxyPool;
use crate::core::{
    BodyRules, DnsConfig, Encoding, IntoUrl, PoolConfig, Proxy, Redirect, SocketOptions, Timeout,
    WebSocketOptions,
};
use crate::profile::{Browser, BrowserProfile, ChromiumBrand, FingerprintSpec, Platform};
use crate::tcp::TcpProfile;
use crate::tls::{Resolver, Tls};
use crate::trace::Trace;

pub(crate) use self::bearer::BearerToken;
use self::derive::{IdentityInputs, IdentitySource, derive_identity};
use self::inherit::Parent;
use super::identity::Source;
use super::proxy::InvalidEnvProxy;
use super::{Identity, Protocol, Session};
use crate::core::error::Result;

const AUTHORIZATION: &str = "authorization";

mod assemble;
mod bearer;
pub(super) mod connect;
mod debug;
pub(super) mod derive;
mod inherit;
mod socket;
mod validate;
#[cfg(feature = "websocket")]
mod websocket;

#[doc = include_str!("../../../docs/session-builder.md")]
#[derive(Clone)]
#[must_use = "builders are lazy: nothing happens until `.send()` / `.build()`"]
pub struct SessionBuilder {
    browser: Option<Browser>,
    profile: Option<Arc<BrowserProfile>>,
    layered: bool,
    platform: Platform,
    platform_explicit: bool,
    platform_pinned: bool,
    brand: ChromiumBrand,
    proxy_config: Proxy,
    dns_config: DnsConfig,
    timeouts: Timeout,
    pool_config: PoolConfig,
    socket_config: SocketOptions,
    redirect_policy: Redirect,
    compression: BodyRules,
    websocket_config: WebSocketOptions,
    https_only: bool,
    audit: bool,
    cookie_jar: Option<Jar>,
    tcp_profile: Option<TcpProfile>,
    protocol_policy: Protocol,
    #[cfg(feature = "http3")]
    protocol_explicit: bool,
    config_error: Option<String>,
    default_headers: Vec<(String, String)>,
    bearer: Option<BearerToken>,
    http_identity: Option<Browser>,
    tls_trust: Tls,
    default_retry: crate::core::retry::Retry,
    trace: Option<Arc<dyn Trace>>,
    base_url: Option<url::Url>,
    languages: Option<Vec<String>>,
    host_limits: HostLimits,
    proxy_pool: Option<ProxyPool>,
    block_rules: Rules,
    parent: Option<Parent>,
    saved: Option<crate::core::saved::Saved>,
}

impl SessionBuilder {
    pub(super) fn new() -> Self {
        Self {
            browser: None,
            profile: None,
            layered: false,
            platform: Platform::default(),
            platform_explicit: false,
            platform_pinned: false,
            brand: ChromiumBrand::default(),
            proxy_config: Proxy::default(),
            dns_config: DnsConfig::default(),
            timeouts: Timeout::default(),
            pool_config: PoolConfig::default(),
            socket_config: SocketOptions::default(),
            redirect_policy: Redirect::default(),
            compression: BodyRules::default(),
            websocket_config: WebSocketOptions::default(),
            https_only: false,
            audit: false,
            cookie_jar: None,
            tcp_profile: None,
            protocol_policy: Protocol::Auto,
            #[cfg(feature = "http3")]
            protocol_explicit: false,
            config_error: None,
            default_headers: Vec::new(),
            bearer: None,
            http_identity: None,
            tls_trust: Tls::default(),
            default_retry: crate::core::retry::Retry::new(0),
            trace: None,
            base_url: None,
            languages: None,
            host_limits: HostLimits::default(),
            proxy_pool: None,
            block_rules: Rules::builtin(),
            parent: None,
            saved: None,
        }
    }

    /// Sets the proxy or proxy pool for every request.
    pub fn proxy(mut self, config: impl Into<Proxy>) -> Self {
        let mut config = config.into();
        self.proxy_pool = config.pool.take();
        self.proxy_config = if self.proxy_pool.is_some() {
            Proxy::new().env(false)
        } else {
            config
        };
        self
    }

    /// Sets the default timeouts for every request.
    pub fn timeout(mut self, config: impl Into<Timeout>) -> Self {
        self.timeouts = config.into();
        self
    }

    /// Sets the default retry policy, which makes no retries by default.
    pub fn retry(mut self, retry: impl Into<crate::core::retry::Retry>) -> Self {
        self.default_retry = retry.into();
        self
    }

    /// Sets the maximum number of open connections in the pool.
    pub fn max_connections(mut self, n: usize) -> Self {
        self.own_pool();
        self.pool_config.max_connections = n;
        self
    }

    /// Sets the maximum number of HTTP/1.1 connections per host.
    pub fn max_connections_per_host(mut self, n: usize) -> Self {
        self.own_pool();
        self.pool_config.max_h1_conns_per_host = n;
        self
    }

    /// Sets how long an idle pooled connection stays open.
    pub fn pool_idle_timeout(mut self, timeout: Duration) -> Self {
        self.own_pool();
        self.pool_config.idle_timeout = timeout;
        self
    }

    /// Sets whether connections are kept open for reuse.
    pub fn keep_alive(mut self, on: bool) -> Self {
        self.own_pool();
        self.pool_config.keepalive = on;
        self
    }

    /// Sets the idle time after which an HTTP/2 connection sends a ping, or `None` for no pings.
    pub fn http2_ping_after_idle(mut self, idle: impl Into<Option<Duration>>) -> Self {
        self.own_pool();
        self.pool_config.h2_ping_after_idle = idle.into();
        self
    }

    /// Sets how long an HTTP/2 ping waits for its acknowledgement.
    pub fn http2_ping_timeout(mut self, timeout: Duration) -> Self {
        self.own_pool();
        self.pool_config.h2_ping_timeout = timeout;
        self
    }

    /// Sets whether a session built from `Session::to_builder` shares its parent's connection pool.
    pub fn share_connections(mut self, share: bool) -> Self {
        if !share {
            self.own_pool();
        }
        self
    }

    fn set_browser(mut self, browser: Browser) -> Self {
        self.profile = None;
        self.http_identity = None;
        self.browser = Some(browser);
        self
    }

    /// Sets the browser the session sends as, with that browser's default platform unless one is set.
    pub fn browser(mut self, browser: Browser) -> Self {
        if self.layered {
            self.config_error.get_or_insert(
                "browser(..) replaces the fingerprint layers set before it; call browser(..) first"
                    .to_owned(),
            );
            return self;
        }
        let platform = if self.platform_pinned {
            self.platform
        } else {
            Identity::from(browser).platform()
        };
        let mut next = self.identity(Identity::new(browser, platform));
        next.brand = browser.brand().unwrap_or_default();
        next
    }

    /// Sets the operating system the session reports.
    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self.platform_explicit = true;
        self.platform_pinned = true;
        self
    }

    /// Sets the language tags the session sends in `Accept-Language`.
    pub fn languages<I, S>(mut self, languages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.set_languages(
            languages
                .into_iter()
                .map(|tag| tag.as_ref().to_owned())
                .collect(),
        );
        self
    }

    /// Sets the TLS ClientHello to another version of the same browser family as the session browser.
    pub fn hello(mut self, hello: Browser) -> Self {
        let Some(tls) = self.browser else {
            self.config_error
                .get_or_insert("hello needs a browser session; call browser(..) first".to_owned());
            return self;
        };
        let http = self.http_identity.unwrap_or(tls);
        let hello = hello.base();
        if http.family() != hello.family() {
            self.config_error.get_or_insert(format!(
                "hello {hello} is not the same browser family as {http}"
            ));
            return self;
        }
        self.browser = Some(hello.hello_rep());
        self.http_identity = Some(http);
        self
    }

    /// Sets the browser profile from a TOML document.
    pub fn profile(self, toml: &str) -> Self {
        match BrowserProfile::from_toml(toml) {
            Ok(profile) => self.set_profile(profile),
            Err(error) => self.fail(error),
        }
    }

    /// Sets the TLS fingerprint from a JA3 string, layered over the current profile.
    pub fn ja3(self, raw: impl Into<String>) -> Self {
        self.fingerprint(|spec| spec.ja3(raw))
    }

    /// Sets the TLS fingerprint from a raw JA4 string, layered over the current profile.
    pub fn ja4_r(self, raw: impl Into<String>) -> Self {
        self.fingerprint(|spec| spec.ja4_r(raw))
    }

    /// Sets the HTTP/2 fingerprint from an Akamai string, layered over the current profile.
    pub fn akamai(self, raw: impl Into<String>) -> Self {
        self.fingerprint(|spec| spec.akamai(raw))
    }

    /// Sets the order of request header names, layered over the current profile.
    pub fn header_order<I, S>(self, order: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let order: Vec<String> = order.into_iter().map(|s| s.as_ref().to_owned()).collect();
        self.fingerprint(|spec| spec.header_order(order))
    }

    fn fingerprint(self, layer: impl FnOnce(FingerprintSpec) -> FingerprintSpec) -> Self {
        let base = match (&self.profile, self.browser) {
            (Some(profile), _) => Some(BrowserProfile::clone(profile)),
            (None, Some(browser)) => Some(browser.shared_profile().as_ref().clone()),
            (None, None) => None,
        };
        let mut spec = FingerprintSpec::new();
        if let Some(base) = base {
            spec = spec.base(base);
        }
        match BrowserProfile::from_fingerprint(layer(spec)) {
            Ok(profile) => {
                let mut next = self.set_profile(profile);
                next.layered = true;
                next
            }
            Err(error) => self.fail(error),
        }
    }

    pub(crate) fn set_profile(mut self, profile: BrowserProfile) -> Self {
        self.layered = false;
        self.browser = None;
        self.http_identity = None;
        self.profile = Some(Arc::new(profile));
        self
    }

    fn fail(mut self, error: crate::core::error::Error) -> Self {
        self.config_error.get_or_insert(error.to_string());
        self
    }

    pub(super) fn loaded_profile(&self) -> Option<&Arc<BrowserProfile>> {
        self.profile.as_ref()
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

    /// Sets the redirect policy for every request.
    pub fn redirect(mut self, policy: impl Into<Redirect>) -> Self {
        self.redirect_policy = policy.into();
        self
    }

    /// Sets the cookie jar the session reads and stores cookies in.
    pub fn cookies(mut self, jar: Jar) -> Self {
        self.cookie_jar = Some(jar);
        self
    }

    /// Sets the TCP fingerprint used for new connections.
    pub fn tcp_profile(mut self, profile: TcpProfile) -> Self {
        self.own_pool();
        self.tcp_profile = Some(profile);
        self
    }

    /// Sets the DNS resolver used for new connections.
    pub fn dns_resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.own_pool();
        self.dns_config = std::mem::take(&mut self.dns_config).resolver(resolver);
        self
    }

    /// Pins `host` to the given socket addresses instead of resolving it through DNS.
    pub fn resolve(
        mut self,
        host: impl AsRef<str>,
        addrs: impl IntoIterator<Item = SocketAddr>,
    ) -> Self {
        self.own_pool();
        self.dns_config = std::mem::take(&mut self.dns_config).resolve_host(host, addrs);
        self
    }

    /// Sets the TLS trust settings for server certificates.
    pub fn tls(mut self, trust: Tls) -> Self {
        self.own_pool();
        self.tls_trust = trust;
        self
    }

    /// Sets the content encodings the session offers in `Accept-Encoding`.
    pub fn accept_encoding(mut self, codings: impl IntoIterator<Item = Encoding>) -> Self {
        self.own_pool();
        self.compression.offer(codings);
        self
    }

    /// Sets the maximum decoded response body size in bytes.
    pub fn max_body_size(mut self, bytes: usize) -> Self {
        self.own_pool();
        self.compression.max_body_size = bytes;
        self
    }

    /// Sets the maximum number of error response body bytes kept in an `Error`.
    pub fn max_error_body_size(mut self, bytes: usize) -> Self {
        self.compression.max_error_body = bytes;
        self
    }

    /// Sets whether the session refuses plain HTTP URLs, which is off by default.
    pub fn https_only(mut self, enabled: bool) -> Self {
        self.https_only = enabled;
        self
    }

    /// Adds a trace hook, alongside any hook already set.
    pub fn trace(mut self, hook: impl Trace) -> Self {
        let hook: Arc<dyn Trace> = Arc::new(hook);
        self.trace = Some(match self.trace.take() {
            Some(first) => Arc::new(crate::trace::Fanout::new(vec![first, hook])),
            None => hook,
        });
        self
    }

    /// Sets whether the session audits requests against its fingerprint, which is off by default.
    pub fn audit(mut self, enabled: bool) -> Self {
        self.audit = enabled;
        self
    }

    /// Sets the HTTP version policy, which is `Protocol::Auto` by default.
    pub fn protocol(mut self, policy: Protocol) -> Self {
        self.own_pool();
        self.protocol_policy = policy;
        #[cfg(feature = "http3")]
        {
            self.protocol_explicit = true;
        }
        self
    }

    pub(crate) fn identity(mut self, id: impl Into<Identity>) -> Self {
        let (source, platform, brand, languages) = id.into().into_parts();
        self.layered = false;
        match source {
            Source::Plain => {
                self.browser = None;
                self.http_identity = None;
                self.profile = None;
            }
            Source::Browser { http, tls } => {
                self = self.set_browser(tls);
                self.http_identity = (http != tls).then_some(http);
            }
            Source::Profile(profile) => {
                self.browser = None;
                self.http_identity = None;
                self.profile = Some(profile);
            }
        }
        self.platform = platform;
        self.platform_explicit = true;
        if let Some(brand) = brand {
            self.brand = brand;
        }
        if let Some(languages) = languages {
            self.set_languages(languages);
        }
        self
    }

    fn set_languages(&mut self, tags: Vec<String>) {
        let tags: Vec<String> = tags.iter().map(|tag| tag.trim().to_owned()).collect();
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
    }

    /// Sets the per-host concurrency and pacing limits.
    pub fn host_limits(mut self, limits: HostLimits) -> Self {
        if let Some(error) = limits.config_error() {
            self.config_error.get_or_insert(error);
        }
        if let Some(parent) = &mut self.parent {
            parent.own_limits = true;
        }
        self.host_limits = limits;
        self
    }

    /// Adds block detection rules from a TOML document to the built-in rules.
    pub fn block_rules(mut self, toml: &str) -> Self {
        match Rules::from_toml(toml) {
            Ok(rules) => self.block_rules.extend(rules),
            Err(error) => {
                self.config_error.get_or_insert(error.to_string());
            }
        }
        self
    }

    /// Adds response status codes that count as a block.
    pub fn block_statuses(mut self, statuses: impl IntoIterator<Item = u16>) -> Self {
        self.block_rules.extend(Rules::statuses(statuses));
        self
    }

    /// Sets the base URL that relative request URLs resolve against.
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

    /// Adds default headers sent on every request.
    pub fn headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: TryInto<http::HeaderName>,
        V: TryInto<http::HeaderValue>,
    {
        for (name, value) in headers {
            let name = crate::core::headers::name(name);
            let value = crate::core::headers::value(value);
            match (name, value) {
                (Ok(name), Ok(value)) => match std::str::from_utf8(value.as_bytes()) {
                    Ok(value) => self.push_default(name.as_str().to_owned(), value.to_owned()),
                    Err(_) => self.invalid_header(name.as_str()),
                },
                (Ok(name), Err(_)) => self.invalid_header(name.as_str()),
                (Err(error), _) => {
                    self.config_error.get_or_insert(error.to_string());
                }
            }
        }
        self
    }

    fn invalid_header(&mut self, name: &str) {
        self.config_error
            .get_or_insert(format!("invalid header `{}`", name.escape_debug()));
    }

    pub(super) fn into_builtin(self) -> Session {
        self.build_with(InvalidEnvProxy::FailRequests)
            .expect("bundled profile data is statically valid; default trust loading only warns")
    }

    /// Builds the session and fails with `Kind::Config` when a setting is invalid.
    pub fn build(self) -> Result<Session> {
        self.build_with(InvalidEnvProxy::FailBuild)
    }

    fn build_with(mut self, on_invalid: InvalidEnvProxy) -> Result<Session> {
        self.validate(on_invalid)?;
        let parent = self.parent.take();
        let saved = self.saved.take();
        let tcp_profile = self.resolve_tcp_profile();
        let derived = derive_identity(self.identity_inputs(&tcp_profile))?;
        let shared = parent
            .as_ref()
            .and_then(|parent| parent.shared_connector(&self));
        let shares = shared.is_some();
        let connector = match shared {
            Some(connector) => connector,
            None => self.build_connector(&derived.profile, &tcp_profile)?,
        };
        let mut session = self.into_session(derived, connector);
        if let Some(parent) = parent {
            parent.share_with(&mut session, shares.then_some(tcp_profile))?;
        }
        if let Some(saved) = saved {
            saved.apply(&session)?;
        }
        Ok(session)
    }

    pub(crate) fn saved(mut self, saved: crate::core::saved::Saved) -> Self {
        self.saved = Some(saved);
        self
    }

    fn own_pool(&mut self) {
        if let Some(parent) = &mut self.parent {
            parent.own_pool = true;
        }
    }
}
