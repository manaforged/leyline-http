use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::cookie::Jar;
use crate::core::block::Rules;
use crate::core::config::HostLimits;
use crate::core::config::Proxy;
use crate::core::config::WebSocketOptions;
use crate::core::into_url::IntoUrl;
use crate::core::proxy_pool::ProxyPool;
use crate::core::{BodyRules, Redirect};
use crate::deadline::Timeout;
use crate::net::{DnsConfig, Resolver, SocketOptions};
use crate::pool::PoolConfig;
use crate::profile::Encoding;
use crate::profile::TcpProfile;
use crate::profile::{Browser, BrowserProfile, ChromiumBrand, FingerprintSpec, Platform};
use crate::tls::trust::Tls;
use crate::trace::Trace;

pub(crate) use self::bearer::Credential;
use self::derive::{IdentityInputs, IdentitySource, derive_identity};
use self::inherit::Parent;
use super::identity::Source;
use super::proxy::InvalidEnvProxy;
use super::{Identity, Protocol, Session};
use crate::error::{Error, Kind, Result};
use crate::wire::Body;

const AUTHORIZATION: &str = "authorization";
const USER_AGENT: &str = "user-agent";

pub(crate) type BeforeSend = Arc<dyn Fn(&mut http::Request<Body>) -> Result<()> + Send + Sync>;

mod assemble;
mod bearer;
pub(super) mod connect;
mod cores;
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
    pub(in crate::core::session) brand: ChromiumBrand,
    pub(in crate::core::session) proxy_config: Proxy,
    pub(in crate::core::session) dns_config: DnsConfig,
    doh_endpoint: Option<url::Url>,
    pub(in crate::core::session) timeouts: Timeout,
    pub(in crate::core::session) pool_config: PoolConfig,
    socket_config: SocketOptions,
    pub(in crate::core::session) redirect_policy: Redirect,
    pub(in crate::core::session) compression: BodyRules,
    pub(in crate::core::session) websocket_config: WebSocketOptions,
    pub(in crate::core::session) https_only: bool,
    audit: bool,
    cookie_jar: Option<Jar>,
    pub(in crate::core::session) stores_cookies: bool,
    pub(in crate::core::session) before_send: Option<BeforeSend>,
    tcp_profile: Option<TcpProfile>,
    pub(in crate::core::session) protocol_policy: Protocol,
    #[cfg(feature = "http3")]
    protocol_explicit: bool,
    config_error: Option<Arc<Error>>,
    default_headers: Vec<(String, String)>,
    pub(in crate::core::session) credential: Option<Credential>,
    refresh_token: Option<bearer::Callback>,
    cache: usize,
    http_identity: Option<Browser>,
    pub(in crate::core::session) tls_trust: Tls,
    pub(in crate::core::session) default_retry: crate::core::retry::Retry,
    pub(in crate::core::session) trace: Option<Arc<dyn Trace>>,
    pub(in crate::core::session) base_url: Option<url::Url>,
    pub(in crate::core::session) languages: Option<Vec<String>>,
    host_limits: HostLimits,
    pub(in crate::core::session) proxy_pool: Option<ProxyPool>,
    pub(in crate::core::session) block_rules: Rules,
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
            doh_endpoint: None,
            timeouts: Timeout::default(),
            pool_config: PoolConfig::default(),
            socket_config: SocketOptions::default(),
            redirect_policy: Redirect::unset(),
            compression: BodyRules::default(),
            websocket_config: WebSocketOptions::default(),
            https_only: false,
            audit: false,
            cookie_jar: None,
            stores_cookies: true,
            before_send: None,
            tcp_profile: None,
            protocol_policy: Protocol::Auto,
            #[cfg(feature = "http3")]
            protocol_explicit: false,
            config_error: None,
            default_headers: Vec::new(),
            credential: None,
            refresh_token: None,
            cache: 0,
            http_identity: None,
            tls_trust: Tls::default(),
            default_retry: crate::core::retry::Retry::unset(),
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
        if let Err(err) = config.check_headers() {
            self.reject(err);
        }
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

    /// Enables a private in-memory HTTP cache with a byte budget; `None` or zero disables caching, which is the default.
    pub fn cache(mut self, bytes: impl Into<Option<usize>>) -> Self {
        self.cache = bytes.into().unwrap_or(0);
        self
    }

    /// Sets the default retry policy, which makes no retries by default.
    pub fn retry(mut self, retry: impl Into<crate::core::retry::Retry>) -> Self {
        self.default_retry = retry.into();
        self
    }

    /// Sets the maximum number of pool entries kept across all hosts, and fails the build on zero.
    pub fn max_connections(mut self, n: usize) -> Self {
        if n == 0 {
            self.reject(Error::config("max_connections must be at least 1"));
        }
        self.pool_config.max_connections = n;
        self
    }

    /// Sets the most connections per host, and fails the build on zero; HTTP/2 adds one when all are full, and browser profiles stay on one unless set.
    pub fn max_connections_per_host(mut self, n: usize) -> Self {
        if n == 0 {
            self.reject(Error::config("max_connections_per_host must be at least 1"));
        }
        self.pool_config.max_h1_conns_per_host = n;
        self.pool_config.max_h2_conns_per_host = Some(n);
        self
    }

    /// Sets how long an idle pooled connection stays open.
    pub fn pool_idle_timeout(mut self, timeout: Duration) -> Self {
        self.pool_config.idle_timeout = timeout;
        self
    }

    /// Stops reusing a connection this long after it opened, so traffic follows a DNS or load balancer change; no limit by default, and `None` removes the limit.
    pub fn pool_max_lifetime(mut self, lifetime: impl Into<Option<Duration>>) -> Self {
        self.pool_config.max_lifetime = lifetime.into();
        self
    }

    /// Sets whether connections are kept open for reuse.
    pub fn keep_alive(mut self, on: bool) -> Self {
        self.pool_config.keepalive = on;
        self
    }

    /// Sets the idle time after which an HTTP/2 connection sends a ping, or `None` for no pings.
    pub fn http2_ping_after_idle(mut self, idle: impl Into<Option<Duration>>) -> Self {
        self.pool_config.h2_ping_after_idle = idle.into();
        self
    }

    /// Sets how long an HTTP/2 ping waits for its acknowledgement.
    pub fn http2_ping_timeout(mut self, timeout: Duration) -> Self {
        self.pool_config.h2_ping_timeout = timeout;
        self
    }

    /// Sets whether a session built from `Session::to_builder` shares its parent's connection pool.
    pub fn share_connections(mut self, share: bool) -> Self {
        if let Some(parent) = &mut self.parent
            && !share
        {
            parent.separate_pool = true;
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
            self.reject(Error::config(
                "browser(..) replaces the fingerprint layers set before it; call browser(..) first",
            ));
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
            self.reject(Error::config(
                "hello needs a browser session; call browser(..) first",
            ));
            return self;
        };
        match Browser::pair(self.http_identity.unwrap_or(tls), hello) {
            Ok((http, hello)) => {
                self.browser = Some(hello);
                self.http_identity = Some(http);
            }
            Err(error) => self.reject(error),
        }
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

    fn fail(mut self, error: Error) -> Self {
        self.reject(error);
        self
    }

    fn reject(&mut self, error: Error) {
        if self.config_error.is_some() {
            return;
        }
        let error = if error.cause() == Kind::Config {
            error
        } else {
            Error::config(error.to_string()).with_source(error)
        };
        self.config_error = Some(Arc::new(error));
    }

    pub(super) fn loaded_profile(&self) -> Option<&Arc<BrowserProfile>> {
        self.profile.as_ref()
    }

    pub(super) fn impersonates(&self) -> bool {
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
            protocol: self.protocol_policy,
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

    /// Sets the cookie jar the session reads and stores cookies in; `None` turns cookie storage off, so `Set-Cookie` is ignored.
    pub fn cookies(mut self, jar: impl Into<Option<Jar>>) -> Self {
        self.cookie_jar = jar.into();
        self.stores_cookies = self.cookie_jar.is_some();
        self
    }

    /// Sets the TCP fingerprint used for new connections.
    pub fn tcp_profile(mut self, profile: impl Into<TcpProfile>) -> Self {
        self.tcp_profile = Some(profile.into());
        self
    }

    /// Sets the DNS resolver used for new connections.
    pub fn dns_resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.doh_endpoint = None;
        self.dns_config = std::mem::take(&mut self.dns_config).resolver(resolver);
        self
    }

    /// Resolves hostnames and HTTPS/ECH records over an RFC 8484 DNS-over-HTTPS `https://` endpoint.
    pub fn doh(mut self, endpoint: impl IntoUrl) -> Self {
        match endpoint.into_url() {
            Ok(url) if url.scheme() == "https" && url.host_str().is_some() => {
                self.doh_endpoint = Some(url);
            }
            Ok(_) => self.reject(Error::config(
                "doh endpoint must be an https:// URL with a host",
            )),
            Err(err) => self.reject(err),
        }
        self
    }

    /// Pins `host` to the given socket addresses instead of resolving it through DNS.
    pub fn resolve(
        mut self,
        host: impl AsRef<str>,
        addrs: impl IntoIterator<Item = SocketAddr>,
    ) -> Self {
        self.dns_config = std::mem::take(&mut self.dns_config).resolve_host(host, addrs);
        self
    }

    /// Sets the TLS trust settings for server certificates.
    pub fn tls(mut self, trust: Tls) -> Self {
        self.tls_trust = trust;
        self
    }

    /// Sets the content encodings the session offers in `Accept-Encoding`.
    pub fn accept_encoding(mut self, codings: impl IntoIterator<Item = Encoding>) -> Self {
        self.compression.offer(codings);
        self
    }

    /// Sets the maximum decoded response body size in bytes, 100 MiB by default; `None` removes the cap.
    pub fn max_body_size(mut self, bytes: impl Into<Option<usize>>) -> Self {
        self.compression
            .set_max_body_size(bytes.into().unwrap_or(crate::wire::NO_SIZE_CAP));
        self
    }

    /// Sets the maximum number of error response body bytes kept in an `Error`, 64 KiB by default; `None` keeps the body up to `max_body_size`.
    pub fn max_error_body_size(mut self, bytes: impl Into<Option<usize>>) -> Self {
        self.compression.max_error_body = bytes.into().unwrap_or(crate::wire::NO_SIZE_CAP);
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
            self.reject(Error::config(error));
        }
        self.languages = Some(tags);
    }

    /// Sets the per-host concurrency and pacing limits.
    pub fn host_limits(mut self, limits: HostLimits) -> Self {
        if let Err(error) = limits.validate() {
            self.reject(error);
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
            Err(error) => self.reject(error),
        }
        self
    }

    /// Adds response status codes that count as a block, and fails the build on a code outside 100 to 999.
    pub fn block_statuses(mut self, statuses: impl IntoIterator<Item = u16>) -> Self {
        match Rules::statuses(statuses) {
            Ok(rules) => self.block_rules.extend(rules),
            Err(error) => self.reject(error),
        }
        self
    }

    /// Sets the base URL that relative request URLs resolve against.
    pub fn base_url(mut self, url: impl IntoUrl) -> Self {
        match super::helpers::base_url(url) {
            Ok(url) => self.base_url = Some(url),
            Err(error) => {
                self.base_url = None;
                self.reject(error);
            }
        }
        self
    }

    /// Runs `hook` on every HTTP request before each attempt (not on redirect hops or WebSocket handshakes) before session defaults, cookies, and browser headers are applied: edits to method, URL, headers and body are sent while edits to extensions and HTTP version are ignored, and returning an error fails the request.
    pub fn before_send<F>(mut self, hook: F) -> Self
    where
        F: Fn(&mut http::Request<Body>) -> Result<()> + Send + Sync + 'static,
    {
        let hook: BeforeSend = match self.before_send.take() {
            Some(first) => Arc::new(move |request: &mut http::Request<Body>| {
                first(request)?;
                hook(request)
            }),
            None => Arc::new(hook),
        };
        self.before_send = Some(hook);
        self
    }

    /// Sets the `User-Agent` header sent on every request, replacing the browser profile's value.
    pub fn user_agent(mut self, value: impl Into<String>) -> Self {
        self.remove_default(USER_AGENT);
        self.header(USER_AGENT, value.into())
    }

    /// Adds one default header sent on every request, keeping earlier values of the same name, and fails the build when the name or value is invalid.
    pub fn header(
        self,
        name: impl TryInto<http::HeaderName>,
        value: impl TryInto<http::HeaderValue>,
    ) -> Self {
        self.headers([(name, value)])
    }

    /// Adds default headers sent on every request.
    pub fn headers<I, K, V>(mut self, headers: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: TryInto<http::HeaderName>,
        V: TryInto<http::HeaderValue>,
    {
        for (name, value) in headers {
            match crate::core::headers::utf8_pair(name, value, "header") {
                Ok((name, value)) => self.push_default(name, value),
                Err(msg) => self.reject(Error::config(msg)),
            }
        }
        self
    }

    pub(crate) fn blocking_io(mut self) -> Self {
        self.pool_config.blocking = true;
        self
    }

    pub(super) fn invalid_header(&mut self, name: &str) {
        self.reject(Error::config(format!(
            "invalid header `{}`",
            name.escape_debug()
        )));
    }

    pub(super) fn into_builtin(self) -> Session {
        self.build_with(InvalidEnvProxy::FailRequests)
            .expect("bundled profile data is statically valid; default trust loading only warns")
    }

    /// Builds the session and fails with `Kind::Config` when a setting is invalid.
    pub fn build(self) -> Result<Session> {
        self.build_with(InvalidEnvProxy::FailBuild)
    }

    /// Builds a `blocking::Session`, whose calls run on the calling thread, and fails with `Kind::Config` when a setting is invalid.
    pub fn build_blocking(self) -> Result<crate::blocking::Session> {
        crate::blocking::Session::new(self)
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
        let (connector, doh) = match shared {
            Some(pair) => pair,
            None => self
                .pool_inputs()
                .connector(&derived.profile, &tcp_profile)?,
        };
        let mut session = self.into_session(derived, connector, doh);
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
}
