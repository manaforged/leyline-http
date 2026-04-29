//! Public configuration structs for Leyline sessions.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use crate::tls::{Resolver, SystemResolver};

/// Proxy configuration for a session.
#[derive(Clone)]
pub struct ProxyConfig {
    rules: Vec<ProxyRule>,
    no_proxy: NoProxy,
    use_env: bool,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("rules", &self.rules)
            .field("no_proxy", &self.no_proxy)
            .field("use_env", &self.use_env)
            .finish()
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            no_proxy: NoProxy::from_env().unwrap_or_default(),
            use_env: true,
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

    /// Add a proxy URL that applies to all supported schemes.
    pub fn all(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.push(ProxyRule::all(proxy_url));
        self
    }

    /// Replace the no-proxy matcher.
    pub fn no_proxy(mut self, no_proxy: NoProxy) -> Self {
        self.no_proxy = no_proxy;
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

    /// First configured proxy URL, used by legacy single-proxy plumbing.
    pub(crate) fn first_proxy(&self) -> Option<&str> {
        self.rules.first().map(|r| r.url.as_str())
    }

    /// Select a proxy URL for a request.
    ///
    /// Resolution order (after NO_PROXY filtering):
    ///   1. `request_override` — caller passed `.proxy(...)` on the
    ///      RequestBuilder; that's an explicit per-request choice and
    ///      wins over any rule or session default. This is what enables
    ///      per-request rotation: each call can pick its own egress
    ///      regardless of the session's bound proxy.
    ///   2. Configured `rules` matching the URL scheme.
    ///   3. `session_default` — the session's `.proxy(...)` value, used
    ///      only when no rule matched.
    pub(crate) fn proxy_for<'a>(
        &'a self,
        url: &url::Url,
        request_override: Option<&'a str>,
        session_default: Option<&'a str>,
    ) -> Option<&'a str> {
        let host = url.host_str().unwrap_or("");
        if self.no_proxy.matches(host) {
            return None;
        }
        if let Some(p) = request_override {
            return Some(p);
        }
        self.rules
            .iter()
            .find(|rule| rule.matches(url.scheme()))
            .map(|rule| rule.url.as_str())
            .or(session_default)
    }
}

/// A single proxy routing rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRule {
    scheme: ProxyRuleScheme,
    url: String,
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

/// Request timeout configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutConfig {
    /// Request-wide timeout.
    pub total: Duration,
    /// DNS + TCP + TLS connect timeout.
    pub connect: Option<Duration>,
    /// Timeout for buffered response body reads.
    pub read: Option<Duration>,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            total: Duration::from_secs(30),
            connect: None,
            read: None,
        }
    }
}

impl TimeoutConfig {
    /// Create default timeout config.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Connection pool configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolConfig {
    /// Idle eviction timeout.
    pub idle_timeout: Duration,
    /// Maximum pooled entries.
    pub max_connections: usize,
    /// Whether keepalive pooling is enabled.
    pub keepalive: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            idle_timeout: crate::pool::DEFAULT_IDLE_TIMEOUT,
            max_connections: crate::pool::DEFAULT_MAX_CONNECTIONS,
            keepalive: true,
        }
    }
}

impl PoolConfig {
    /// Create default pool config.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Socket-level direct-connect options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
    /// TCP keepalive interval.
    pub tcp_keepalive_interval: Option<Duration>,
    /// TCP keepalive probe count.
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
pub struct RedirectAttempt<'a> {
    /// Response status code.
    pub status: u16,
    /// Current request URL.
    pub url: &'a url::Url,
    /// Location header value, if present.
    pub location: Option<&'a str>,
    /// Previously visited URLs.
    pub previous: &'a [String],
}

/// Decision returned by a redirect policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectAction {
    /// Follow the redirect.
    Follow,
    /// Return the redirect response to the caller.
    Stop,
}

/// Response decompression configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// WebSocket connection preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(':')
        .next()
        .unwrap_or(host)
        .trim_matches('.')
        .to_ascii_lowercase()
}

fn pattern_matches(host: &str, raw: &str) -> bool {
    let pattern = normalize_host(raw);
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('.') {
        return host == suffix || host.ends_with(&format!(".{suffix}"));
    }
    host == pattern || host.ends_with(&format!(".{pattern}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_proxy_matches_domains_and_literals() {
        let no_proxy = NoProxy::from_string(".example.com,127.0.0.1,[::1]:8080").unwrap();
        assert!(no_proxy.matches("api.example.com"));
        assert!(no_proxy.matches("127.0.0.1"));
        assert!(no_proxy.matches("[::1]"));
        assert!(!no_proxy.matches("other.test"));
    }

    #[test]
    fn compression_none_disables_known_codecs() {
        let cfg = CompressionConfig::none();
        assert!(!cfg.allows("gzip"));
        assert!(!cfg.allows("br"));
        assert!(cfg.allows("identity"));
    }
}
