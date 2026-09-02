//! Public configuration structs for Leyline sessions.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use crate::core::Kind;
use crate::tls::{Resolver, SystemResolver};

/// A validated proxy URL.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProxyUrl(String);

impl std::fmt::Debug for ProxyUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProxyUrl").field(&redact(&self.0)).finish()
    }
}

/// Replace the password in a proxy URL's userinfo with `***` for logs and `Debug` output.
pub(crate) fn redact(raw: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw) else {
        return raw.to_string();
    };
    if parsed.password().is_none() {
        return raw.to_string();
    }
    if parsed.set_password(Some("***")).is_err() {
        return raw.to_string();
    }
    parsed.to_string()
}

impl ProxyUrl {
    /// Validate a proxy URL.
    pub fn parse(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        let raw = raw.as_ref().trim();
        let parsed = Self::parse_inner(raw)?;
        match parsed.scheme() {
            "http" | "https" | "socks5" | "socks5h" => {}
            other => {
                return Err(crate::core::Error::new(Kind::Config).with_message(format!(
                    "unsupported proxy scheme {other:?}; expected http, https, socks5, or socks5h"
                )));
            }
        }
        if parsed.host_str().is_none() {
            return Err(
                crate::core::Error::new(Kind::Config).with_message("proxy URL must include a host")
            );
        }
        Ok(Self(raw.to_string()))
    }

    fn parse_inner(raw: &str) -> crate::core::Result<url::Url> {
        url::Url::parse(raw).map_err(|e| {
            crate::core::Error::new(Kind::Config).with_message(format!("invalid proxy URL: {e}"))
        })
    }

    fn parse_scheme(raw: impl AsRef<str>, expected: &'static str) -> crate::core::Result<Self> {
        let raw = raw.as_ref().trim();
        let parsed = Self::parse_inner(raw)?;
        if parsed.scheme() != expected {
            return Err(crate::core::Error::new(Kind::Config).with_message(format!(
                "expected {expected} proxy URL, got {:?}",
                parsed.scheme()
            )));
        }
        if parsed.host_str().is_none() {
            return Err(
                crate::core::Error::new(Kind::Config).with_message("proxy URL must include a host")
            );
        }
        Ok(Self(raw.to_string()))
    }

    /// Build an HTTP proxy URL.
    pub fn http(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "http")
    }

    /// Build an HTTPS proxy URL (TLS to the proxy, then CONNECT).
    pub fn https(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "https")
    }

    /// Build a SOCKS5 proxy URL. Hostnames are sent to the proxy for resolution, the same as `socks5h`.
    pub fn socks5(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "socks5")
    }

    /// Build a SOCKS5H proxy URL. Behaves the same as `socks5`: the proxy resolves hostnames.
    pub fn socks5h(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "socks5h")
    }

    /// Borrow as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consume into the validated string.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for ProxyUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<ProxyUrl> for String {
    fn from(value: ProxyUrl) -> Self {
        value.0
    }
}

/// Proxy configuration for a session.
#[derive(Clone)]
pub struct ProxyConfig {
    rules: Vec<ProxyRule>,
    no_proxy: NoProxy,
    /// `true` when the matcher was set via [`Self::no_proxy`] (deliberate caller config) rather than inherited from the `NO_PROXY` env var.
    no_proxy_explicit: bool,
    use_env: bool,
    /// `true` when the default proxy came from `HTTPS_PROXY`/`HTTP_PROXY` at session build, so `NO_PROXY` applies to it.
    from_env: bool,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("rules", &self.rules)
            .field("no_proxy", &self.no_proxy)
            .field("no_proxy_explicit", &self.no_proxy_explicit)
            .field("use_env", &self.use_env)
            .field("from_env", &self.from_env)
            .finish()
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            no_proxy: NoProxy::from_env().unwrap_or_default(),
            no_proxy_explicit: false,
            use_env: true,
            from_env: false,
        }
    }
}

impl ProxyConfig {
    /// Create proxy config that honours environment proxies.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a proxy rule.
    pub fn with_rule(mut self, rule: ProxyRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Replace the session-default all-scheme proxy.
    pub(crate) fn set_default_proxy(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.retain(|r| r.scheme != ProxyRuleScheme::All);
        self.rules.push(ProxyRule::all(proxy_url));
        self.from_env = false;
        self
    }

    /// Mark the default proxy as discovered from the environment.
    pub(crate) fn set_from_env(mut self) -> Self {
        self.from_env = true;
        self
    }

    /// Every configured rule, for validation at session build.
    pub(crate) fn rules(&self) -> &[ProxyRule] {
        &self.rules
    }

    /// The all-scheme proxy when one is set, else the first rule's URL.
    pub(crate) fn primary(&self) -> Option<&str> {
        self.rules
            .iter()
            .find(|r| r.scheme == ProxyRuleScheme::All)
            .or_else(|| self.rules.first())
            .map(|r| r.url.as_str())
    }

    /// Add a proxy URL that applies to all supported schemes.
    pub fn all(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.push(ProxyRule::all(proxy_url));
        self
    }

    /// Replace the no-proxy matcher.
    pub fn no_proxy(mut self, no_proxy: NoProxy) -> Self {
        self.no_proxy = no_proxy;
        self.no_proxy_explicit = true;
        self
    }

    /// Disable environment proxy discovery.
    pub fn without_env(mut self) -> Self {
        self.use_env = false;
        self
    }

    /// Whether environment proxy discovery is enabled.
    pub fn uses_env(&self) -> bool {
        self.use_env
    }

    /// Select a proxy URL for a request.
    pub(crate) fn proxy_for<'a>(
        &'a self,
        url: &url::Url,
        request_override: Option<&'a str>,
    ) -> Option<&'a str> {
        let host = url.host_str().unwrap_or("");
        if let Some(p) = request_override {
            if self.no_proxy_explicit && self.no_proxy.matches(host) {
                return None;
            }
            return Some(p);
        }
        let winner = self
            .rules
            .iter()
            .find(|rule| rule.matches(url.scheme()))
            .map(|rule| rule.url.as_str())?;
        if (self.no_proxy_explicit || self.from_env) && self.no_proxy.matches(host) {
            return None;
        }
        Some(winner)
    }
}

/// A single proxy routing rule.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProxyRule {
    scheme: ProxyRuleScheme,
    url: String,
}

impl std::fmt::Debug for ProxyRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyRule")
            .field("scheme", &self.scheme)
            .field("url", &redact(&self.url))
            .finish()
    }
}

impl ProxyRule {
    /// Route all supported request schemes through `proxy_url`.
    pub fn all(proxy_url: impl Into<String>) -> Self {
        Self {
            scheme: ProxyRuleScheme::All,
            url: proxy_url.into(),
        }
    }

    /// Route HTTP requests through `proxy_url`.
    pub fn http(proxy_url: impl Into<String>) -> Self {
        Self {
            scheme: ProxyRuleScheme::Http,
            url: proxy_url.into(),
        }
    }

    /// Route HTTPS requests through `proxy_url`.
    pub fn https(proxy_url: impl Into<String>) -> Self {
        Self {
            scheme: ProxyRuleScheme::Https,
            url: proxy_url.into(),
        }
    }

    fn matches(&self, scheme: &str) -> bool {
        matches!(
            (self.scheme, scheme),
            (ProxyRuleScheme::All, _)
                | (ProxyRuleScheme::Http, "http")
                | (ProxyRuleScheme::Https, "https")
        )
    }

    /// Proxy URL carried by this rule.
    pub fn url(&self) -> &str {
        &self.url
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyRuleScheme {
    All,
    Http,
    Https,
}

/// Host matcher for proxy bypass rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoProxy {
    patterns: Vec<String>,
}

impl NoProxy {
    /// Build from a comma-separated no-proxy list.
    pub fn from_string(raw: &str) -> Option<Self> {
        let patterns: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        (!patterns.is_empty()).then_some(Self { patterns })
    }

    /// Build from `NO_PROXY` / `no_proxy`.
    pub fn from_env() -> Option<Self> {
        std::env::var("NO_PROXY")
            .ok()
            .or_else(|| std::env::var("no_proxy").ok())
            .and_then(|raw| Self::from_string(&raw))
    }

    /// Create a matcher from exact/domain patterns.
    pub fn new<I, S>(patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            patterns: patterns.into_iter().map(Into::into).collect(),
        }
    }

    /// Return true when `host` should bypass proxies.
    pub fn matches(&self, host: &str) -> bool {
        let host = normalize_host(host);
        self.patterns.iter().any(|raw| pattern_matches(&host, raw))
    }
}

/// DNS configuration for direct connections.
#[derive(Clone)]
pub struct DnsConfig {
    resolver: Arc<dyn Resolver>,
    overrides: HashMap<String, Vec<SocketAddr>>,
}

impl std::fmt::Debug for DnsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DnsConfig")
            .field("overrides", &self.overrides)
            .finish_non_exhaustive()
    }
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            resolver: Arc::new(SystemResolver),
            overrides: HashMap::new(),
        }
    }
}

impl DnsConfig {
    /// Create DNS config with the system resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Use a custom resolver.
    pub fn resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    /// Resolve a host to a single socket address.
    pub fn resolve_host(mut self, host: impl AsRef<str>, addr: SocketAddr) -> Self {
        self.overrides
            .insert(normalize_host(host.as_ref()), vec![addr]);
        self
    }

    /// Resolve a host to multiple socket addresses.
    pub fn resolve_host_to_addrs<I>(mut self, host: impl AsRef<str>, addrs: I) -> Self
    where
        I: IntoIterator<Item = SocketAddr>,
    {
        self.overrides
            .insert(normalize_host(host.as_ref()), addrs.into_iter().collect());
        self
    }

    /// Convert to a resolver object.
    pub(crate) fn into_resolver(self) -> Arc<dyn Resolver> {
        if self.overrides.is_empty() {
            self.resolver
        } else {
            Arc::new(LayeredResolver {
                resolver: self.resolver,
                overrides: self.overrides,
            })
        }
    }
}

struct LayeredResolver {
    resolver: Arc<dyn Resolver>,
    overrides: HashMap<String, Vec<SocketAddr>>,
}

impl Resolver for LayeredResolver {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> crate::tls::ResolveFuture<'a> {
        let key = normalize_host(host);
        if let Some(addrs) = self.overrides.get(&key) {
            let addrs = addrs
                .iter()
                .map(|addr| SocketAddr::new(addr.ip(), port))
                .collect::<Vec<_>>();
            return Box::pin(async move { Ok(addrs) });
        }
        self.resolver.resolve(host, port)
    }
}

/// Request timeout configuration. Start from [`TimeoutConfig::default`] and set one field per call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TimeoutConfig {
    /// Wall-clock cap on one `send`, covering every redirect hop, retry, backoff sleep, and buffered body read; on expiry the call returns a `Kind::Timeout` error.
    pub total: Duration,
    /// Cap on DNS + TCP + TLS setup for one new `https` connection, fired before the request is written; pooled reuse and plaintext `http` connects are not covered.
    pub connect: Option<Duration>,
    /// Idle cap between chunks of a streamed response body, fired only on a request that called `stream`; a buffered body is read inside the `response_header` and `total` windows instead.
    pub read: Option<Duration>,
    /// Cap on the wait from request-sent until the transport response resolves, per redirect hop; a buffered response resolves only after its body is read.
    pub response_header: Option<Duration>,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            total: Duration::from_secs(300),
            connect: Some(Duration::from_secs(10)),
            read: None,
            response_header: None,
        }
    }
}

impl TimeoutConfig {
    /// Create default timeout config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the wall-clock cap on one `send`.
    pub fn total(mut self, d: Duration) -> Self {
        self.total = d;
        self
    }

    /// Set the connect-setup cap. `None` disables it.
    pub fn connect(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.connect = d.into();
        self
    }

    /// Set the idle cap between streamed body chunks. `None` disables it.
    pub fn read(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.read = d.into();
        self
    }

    /// Set the request-sent to response cap, per redirect hop. `None` disables it.
    pub fn response_header(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.response_header = d.into();
        self
    }
}

/// Connection pool configuration. Start from [`PoolConfig::default`] and set one field per call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolConfig {
    /// Idle eviction timeout.
    pub idle_timeout: Duration,
    /// Maximum pooled entries.
    pub max_connections: usize,
    /// Maximum simultaneous HTTP/1.1 connections per destination `(host, port, proxy)`.
    pub max_h1_conns_per_host: usize,
    /// Whether keepalive pooling is enabled.
    pub keepalive: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            idle_timeout: crate::pool::DEFAULT_IDLE_TIMEOUT,
            max_connections: crate::pool::DEFAULT_MAX_CONNECTIONS,
            max_h1_conns_per_host: crate::pool::DEFAULT_MAX_H1_CONNS_PER_HOST,
            keepalive: true,
        }
    }
}

impl PoolConfig {
    /// Create default pool config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the idle eviction timeout.
    pub fn idle_timeout(mut self, d: Duration) -> Self {
        self.idle_timeout = d;
        self
    }

    /// Set the maximum number of pooled entries.
    pub fn max_connections(mut self, n: usize) -> Self {
        self.max_connections = n;
        self
    }

    /// Set the maximum simultaneous HTTP/1.1 connections per destination.
    pub fn max_h1_conns_per_host(mut self, n: usize) -> Self {
        self.max_h1_conns_per_host = n;
        self
    }

    /// Enable or disable keepalive pooling.
    pub fn keepalive(mut self, on: bool) -> Self {
        self.keepalive = on;
        self
    }
}

/// Socket-level direct-connect options. Start from [`SocketConfig::default`] and set one field per call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SocketConfig {
    /// Bind every direct socket to this local address.
    pub local_address: Option<IpAddr>,
    /// IPv4-specific local bind address.
    pub local_ipv4: Option<Ipv4Addr>,
    /// IPv6-specific local bind address.
    pub local_ipv6: Option<Ipv6Addr>,
    /// Override TCP_NODELAY after the browser TCP profile is applied.
    pub tcp_nodelay: Option<bool>,
    /// TCP keepalive idle time.
    pub tcp_keepalive: Option<Duration>,
    /// TCP keepalive interval between probes once idle expires.
    pub tcp_keepalive_interval: Option<Duration>,
    /// TCP keepalive probe count before the kernel drops the connection.
    pub tcp_keepalive_retries: Option<u32>,
    /// TCP user timeout.
    pub tcp_user_timeout: Option<Duration>,
    /// Socket send buffer size.
    pub send_buffer_size: Option<usize>,
    /// Socket receive buffer size.
    pub recv_buffer_size: Option<usize>,
    /// Platform network interface name.
    pub interface: Option<String>,
    /// Treat socket-option failures as hard errors.
    pub strict: bool,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            local_address: None,
            local_ipv4: None,
            local_ipv6: None,
            tcp_nodelay: None,
            tcp_keepalive: Some(Duration::from_secs(60)),
            tcp_keepalive_interval: Some(Duration::from_secs(30)),
            tcp_keepalive_retries: Some(3),
            tcp_user_timeout: None,
            send_buffer_size: None,
            recv_buffer_size: None,
            interface: None,
            strict: false,
        }
    }
}

impl SocketConfig {
    /// Create default socket config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind every direct socket to this local address.
    pub fn local_address(mut self, addr: impl Into<Option<IpAddr>>) -> Self {
        self.local_address = addr.into();
        self
    }

    /// Bind IPv4 sockets to this local address.
    pub fn local_ipv4(mut self, addr: impl Into<Option<Ipv4Addr>>) -> Self {
        self.local_ipv4 = addr.into();
        self
    }

    /// Bind IPv6 sockets to this local address.
    pub fn local_ipv6(mut self, addr: impl Into<Option<Ipv6Addr>>) -> Self {
        self.local_ipv6 = addr.into();
        self
    }

    /// Override `TCP_NODELAY` after the browser TCP profile is applied.
    pub fn tcp_nodelay(mut self, on: impl Into<Option<bool>>) -> Self {
        self.tcp_nodelay = on.into();
        self
    }

    /// Set the TCP keepalive idle time.
    pub fn tcp_keepalive(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_keepalive = d.into();
        self
    }

    /// Set the TCP keepalive probe interval.
    pub fn tcp_keepalive_interval(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_keepalive_interval = d.into();
        self
    }

    /// Set the TCP keepalive probe count.
    pub fn tcp_keepalive_retries(mut self, n: impl Into<Option<u32>>) -> Self {
        self.tcp_keepalive_retries = n.into();
        self
    }

    /// Set the TCP user timeout.
    pub fn tcp_user_timeout(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_user_timeout = d.into();
        self
    }

    /// Set the socket send buffer size.
    pub fn send_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.send_buffer_size = n.into();
        self
    }

    /// Set the socket receive buffer size.
    pub fn recv_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.recv_buffer_size = n.into();
        self
    }

    /// Bind to a platform network interface by name.
    pub fn interface(mut self, name: impl Into<String>) -> Self {
        self.interface = Some(name.into());
        self
    }

    /// Treat socket-option failures as hard errors.
    pub fn strict(mut self, on: bool) -> Self {
        self.strict = on;
        self
    }
}

/// Redirect follow policy.
#[derive(Clone)]
pub struct RedirectPolicy {
    kind: RedirectKind,
}

#[derive(Clone)]
enum RedirectKind {
    Limited(usize),
    None,
    Custom(Arc<dyn Fn(RedirectAttempt<'_>) -> RedirectAction + Send + Sync>),
}

impl std::fmt::Debug for RedirectPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            RedirectKind::Limited(n) => f.debug_tuple("RedirectPolicy::Limited").field(n).finish(),
            RedirectKind::None => f.write_str("RedirectPolicy::None"),
            RedirectKind::Custom(_) => f.write_str("RedirectPolicy::Custom(..)"),
        }
    }
}

impl Default for RedirectPolicy {
    fn default() -> Self {
        Self::limited(10)
    }
}

impl RedirectPolicy {
    /// Follow up to `max` redirects.
    pub fn limited(max: usize) -> Self {
        Self {
            kind: RedirectKind::Limited(max),
        }
    }

    /// Do not follow redirects.
    pub fn none() -> Self {
        Self {
            kind: RedirectKind::None,
        }
    }

    /// Use a custom redirect callback.
    pub fn custom<F>(f: F) -> Self
    where
        F: Fn(RedirectAttempt<'_>) -> RedirectAction + Send + Sync + 'static,
    {
        Self {
            kind: RedirectKind::Custom(Arc::new(f)),
        }
    }

    pub(crate) fn max_redirects_hint(&self) -> usize {
        match self.kind {
            RedirectKind::Limited(n) => n,
            RedirectKind::None => 0,
            RedirectKind::Custom(_) => 32,
        }
    }

    pub(crate) fn action(&self, attempt: RedirectAttempt<'_>) -> RedirectAction {
        match &self.kind {
            RedirectKind::Limited(max) => {
                if attempt.previous.len() < *max {
                    RedirectAction::Follow
                } else {
                    RedirectAction::Stop
                }
            }
            RedirectKind::None => RedirectAction::Stop,
            RedirectKind::Custom(f) => f(attempt),
        }
    }
}

/// Information passed to a custom redirect policy.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RedirectAttempt<'a> {
    /// Response status code.
    pub status: u16,
    /// Current request URL.
    pub url: &'a http::Uri,
    /// Location header value, if present.
    pub location: Option<&'a str>,
    /// Previously visited URLs.
    pub previous: &'a [String],
}

/// Decision returned by a redirect policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RedirectAction {
    /// Follow the redirect.
    Follow,
    /// Return the redirect response to the caller.
    Stop,
}

/// Response decompression configuration. Start from [`CompressionConfig::default`] and set one field per call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompressionConfig {
    /// Decode gzip.
    pub gzip: bool,
    /// Decode Brotli.
    pub brotli: bool,
    /// Decode deflate.
    pub deflate: bool,
    /// Decode zstd.
    pub zstd: bool,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            gzip: true,
            brotli: true,
            deflate: true,
            zstd: true,
        }
    }
}

impl CompressionConfig {
    /// Create default compression config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Disable all response decompression.
    pub fn none() -> Self {
        Self {
            gzip: false,
            brotli: false,
            deflate: false,
            zstd: false,
        }
    }

    /// Enable or disable gzip decoding.
    pub fn gzip(mut self, on: bool) -> Self {
        self.gzip = on;
        self
    }

    /// Enable or disable Brotli decoding.
    pub fn brotli(mut self, on: bool) -> Self {
        self.brotli = on;
        self
    }

    /// Enable or disable deflate decoding.
    pub fn deflate(mut self, on: bool) -> Self {
        self.deflate = on;
        self
    }

    /// Enable or disable zstd decoding.
    pub fn zstd(mut self, on: bool) -> Self {
        self.zstd = on;
        self
    }

    /// Return true if this encoding may be decoded.
    pub(crate) fn allows(&self, encoding: &str) -> bool {
        match encoding {
            "gzip" | "x-gzip" => self.gzip,
            "br" => self.brotli,
            "deflate" => self.deflate,
            "zstd" => self.zstd,
            "identity" | "" => true,
            _ => false,
        }
    }
}

/// WebSocket connection preferences. Start from [`WebSocketConfig::default`] and set one field per call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebSocketConfig {
    /// Prefer HTTP/2 extended CONNECT before H1 upgrade.
    pub prefer_http2: bool,
    /// Maximum frame size.
    pub max_frame_size: Option<usize>,
    /// Maximum message size.
    pub max_message_size: Option<usize>,
    /// Read buffer size.
    pub read_buffer_size: Option<usize>,
    /// Write buffer size.
    pub write_buffer_size: Option<usize>,
    /// Maximum queued write buffer size.
    pub max_write_buffer_size: Option<usize>,
    /// Accept unmasked frames.
    pub accept_unmasked_frames: bool,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            prefer_http2: true,
            max_frame_size: None,
            max_message_size: None,
            read_buffer_size: None,
            write_buffer_size: None,
            max_write_buffer_size: None,
            accept_unmasked_frames: false,
        }
    }
}

impl WebSocketConfig {
    /// Create default WebSocket config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Prefer HTTP/2 extended CONNECT before the HTTP/1.1 upgrade.
    pub fn prefer_http2(mut self, on: bool) -> Self {
        self.prefer_http2 = on;
        self
    }

    /// Set the maximum frame size.
    pub fn max_frame_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_frame_size = n.into();
        self
    }

    /// Set the maximum message size.
    pub fn max_message_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_message_size = n.into();
        self
    }

    /// Set the read buffer size.
    pub fn read_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.read_buffer_size = n.into();
        self
    }

    /// Set the write buffer size.
    pub fn write_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.write_buffer_size = n.into();
        self
    }

    /// Set the maximum queued write buffer size.
    pub fn max_write_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_write_buffer_size = n.into();
        self
    }

    /// Accept unmasked frames from the peer.
    pub fn accept_unmasked_frames(mut self, on: bool) -> Self {
        self.accept_unmasked_frames = on;
        self
    }
}

fn normalize_host(host: &str) -> String {
    let stripped = host.trim().trim_end_matches('.');
    let stripped = stripped
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(stripped);
    let lowered = stripped.to_ascii_lowercase();
    match url::Host::parse(&lowered) {
        Ok(url::Host::Domain(d)) => d,
        Ok(url::Host::Ipv4(a)) => a.to_string(),
        Ok(url::Host::Ipv6(a)) => a.to_string(),
        Err(_) => lowered,
    }
}

fn pattern_matches(host: &str, raw: &str) -> bool {
    let mut pat = raw.trim().to_string();
    if pat.is_empty() {
        return false;
    }
    if pat == "*" {
        return true;
    }
    if pat.starts_with('[') {
        if let Some(end) = pat.find("]:") {
            let suffix = &pat[end + 2..];
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                pat.truncate(end + 1);
            }
        }
    } else if pat.matches(':').count() == 1
        && let Some(idx) = pat.rfind(':')
        && pat[idx + 1..].bytes().all(|b| b.is_ascii_digit())
    {
        pat.truncate(idx);
    }
    let pat = normalize_host(&pat);
    let needle = pat.strip_prefix('.').unwrap_or(&pat);
    host == needle || host.ends_with(&format!(".{needle}"))
}

#[cfg(test)]
mod tests;
