//! Public configuration structs for Leyline sessions.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use crate::tls::{Resolver, SystemResolver};

/// A validated proxy URL.
///
/// Leyline accepts `http://`, `socks5://`, and `socks5h://` proxy URLs.
/// SOCKS URLs require the `socks` feature at connection time. An `https://`
/// proxy (TLS to the proxy itself) is rejected: leyline does not yet perform a
/// TLS handshake to the proxy, so honoring it would send the CONNECT request —
/// including any credentials — in cleartext. This type lets callers validate
/// and carry proxy strings without accidentally feeding an empty host or
/// unsupported scheme into the connection layer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProxyUrl(String);

impl ProxyUrl {
    /// Validate a proxy URL.
    pub fn parse(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        let raw = raw.as_ref().trim();
        let parsed = Self::parse_inner(raw)?;
        match parsed.scheme() {
            "http" | "socks5" | "socks5h" => {}
            "https" => {
                return Err(crate::core::Error::Config(
                    "https:// proxies are not supported: leyline does not yet TLS-handshake to the \
                     proxy, so CONNECT would be sent in cleartext. Use http:// or socks5://."
                        .into(),
                ));
            }
            other => {
                return Err(crate::core::Error::Config(format!(
                    "unsupported proxy scheme {other:?}; expected http, socks5, or socks5h"
                )));
            }
        }
        if parsed.host_str().is_none() {
            return Err(crate::core::Error::Config(
                "proxy URL must include a host".into(),
            ));
        }
        Ok(Self(raw.to_string()))
    }

    fn parse_inner(raw: &str) -> crate::core::Result<url::Url> {
        url::Url::parse(raw)
            .map_err(|e| crate::core::Error::Config(format!("invalid proxy URL: {e}")))
    }

    fn parse_scheme(raw: impl AsRef<str>, expected: &'static str) -> crate::core::Result<Self> {
        let raw = raw.as_ref().trim();
        let parsed = Self::parse_inner(raw)?;
        if parsed.scheme() != expected {
            return Err(crate::core::Error::Config(format!(
                "expected {expected} proxy URL, got {:?}",
                parsed.scheme()
            )));
        }
        if parsed.host_str().is_none() {
            return Err(crate::core::Error::Config(
                "proxy URL must include a host".into(),
            ));
        }
        Ok(Self(raw.to_string()))
    }

    /// Build an HTTP proxy URL.
    pub fn http(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "http")
    }

    /// Build a SOCKS5 proxy URL.
    pub fn socks5(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "socks5")
    }

    /// Build a SOCKS5H proxy URL where DNS resolution happens at the proxy.
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
    /// `true` when the matcher was set via [`Self::no_proxy`] (deliberate
    /// caller config) rather than inherited from the `NO_PROXY` env var.
    /// Env-derived patterns only gate env-derived proxies — they must
    /// never silently turn an explicitly-proxied request DIRECT.
    no_proxy_explicit: bool,
    use_env: bool,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("rules", &self.rules)
            .field("no_proxy", &self.no_proxy)
            .field("no_proxy_explicit", &self.no_proxy_explicit)
            .field("use_env", &self.use_env)
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

    /// Replace the session-default all-scheme proxy. Any existing
    /// all-scheme rule is removed before the new one is appended;
    /// scheme-specific rules are preserved and keep outranking the
    /// catch-all (`proxy_for` is first-match). Session-level setters
    /// (`SessionBuilder::proxy`, `Session::with_proxy`) must use this
    /// instead of `with_rule`: appending leaves the older all-scheme
    /// rule winning first-match and the new proxy silently ignored.
    pub(crate) fn set_default_proxy(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.retain(|r| r.scheme != ProxyRuleScheme::All);
        self.rules.push(ProxyRule::all(proxy_url));
        self
    }

    /// Add a proxy URL that applies to all supported schemes.
    pub fn all(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.push(ProxyRule::all(proxy_url));
        self
    }

    /// Replace the no-proxy matcher. A matcher set here is *explicit*:
    /// it bypasses any configured proxy, including per-request overrides.
    /// (The env-inherited `NO_PROXY` default only gates env-derived
    /// proxies — see `Self::proxy_for`.)
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

    /// First configured proxy URL, used by legacy single-proxy plumbing.
    pub(crate) fn first_proxy(&self) -> Option<&str> {
        self.rules.first().map(|r| r.url.as_str())
    }

    /// Select a proxy URL for a request.
    ///
    /// Resolution order:
    ///   1. `request_override` - caller passed `.proxy(...)` on the
    ///      RequestBuilder; that's an explicit per-request choice and
    ///      wins over any rule or session default. This is what enables
    ///      per-request rotation: each call can pick its own egress
    ///      regardless of the session's bound proxy.
    ///   2. Configured `rules` matching the URL scheme.
    ///   3. `session_default` - the session's `.proxy(...)` value, used
    ///      only when no rule matched.
    ///
    /// No-proxy gating is scoped by provenance: a matcher set via
    /// [`Self::no_proxy`] bypasses any of the three; the env-inherited
    /// `NO_PROXY` default only bypasses proxies that were themselves
    /// discovered from the environment (`session_proxy_from_env`). A
    /// stray `NO_PROXY` on the box must never silently turn an
    /// explicitly-proxied request DIRECT — that is a
    /// real-IP leak, not a convenience.
    pub(crate) fn proxy_for<'a>(
        &'a self,
        url: &url::Url,
        request_override: Option<&'a str>,
        session_default: Option<&'a str>,
        session_proxy_from_env: bool,
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
            .map(|rule| rule.url.as_str())
            .or(session_default)?;
        // When the env proxy was injected at session build, it is the
        // only rule and the session default (injection is skipped as
        // soon as any explicit proxy exists) — so one flag covers both.
        if (self.no_proxy_explicit || session_proxy_from_env) && self.no_proxy.matches(host) {
            return None;
        }
        Some(winner)
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
    /// Per-chunk idle timeout for streaming response bodies (and the drain of
    /// a streamed body that the caller buffers). Resets after each chunk, so a
    /// steady stream never trips it; a stalled connection errors with
    /// `TimedOut` well before the request-wide `total` timeout.
    pub read: Option<Duration>,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            total: Duration::from_secs(300),
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
    /// Maximum simultaneous HTTP/1.1 connections per destination
    /// `(host, port, proxy)`. Defaults to 256 (throughput-favouring; H1 is the
    /// rare ALPN fallback) — set to 6 to mirror a browser's per-host socket
    /// limit. The HTTP/2 path (one multiplexed connection) is unaffected.
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
}

/// Socket-level direct-connect options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketConfig {
    /// Bind every direct socket to this local address.
    pub local_address: Option<IpAddr>,
    /// IPv4-specific local bind address.
    pub local_ipv4: Option<Ipv4Addr>,
    /// IPv6-specific local bind address.
    pub local_ipv6: Option<Ipv6Addr>,
    /// Override TCP_NODELAY after the browser TCP profile is applied.
    pub tcp_nodelay: Option<bool>,
    /// TCP keepalive idle time. Default 60s — kernel sends a
    /// keepalive probe after the socket has been idle this long.
    /// Cheap (one packet, no syscall on our side) and keeps NAT /
    /// load-balancer flow tables from pruning the connection
    /// during idle gaps that happen between long-lived requests
    /// (long-lived / session-persistent pooled workloads).
    /// Set to `None` to disable kernel keepalive entirely.
    pub tcp_keepalive: Option<Duration>,
    /// TCP keepalive interval between probes once idle expires.
    /// Default 30s.
    pub tcp_keepalive_interval: Option<Duration>,
    /// TCP keepalive probe count before the kernel drops the
    /// connection. Default 3 probes (matches Linux default).
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
            // Kernel-level keepalive on by default — long-lived
            // sessions (session-persistent pooled workloads) need their
            // sockets to survive idle gaps without app-level pings,
            // and one keepalive probe every 60s is invisibly cheap.
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
    pub url: &'a url::Url,
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
    let stripped = host.trim().trim_end_matches('.');
    // Strip a matching `[` `]` pair around IPv6 literals — `url::Host`
    // rejects bracketed input, and NO_PROXY accepts both `[::1]` and
    // `::1` forms. Only strip when both brackets are present.
    let stripped = stripped
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(stripped);
    let lowered = stripped.to_ascii_lowercase();
    // `url::Host` canonicalises IP literals (collapsing `::` runs) and
    // IDN domains, so `::1`/`fe80::1` survive instead of being mangled
    // by a naive `split(':')`.
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
    // Strip a `:port` suffix while leaving bare IPv6 literals (two or
    // more colons, unbracketed) untouched — `[::1]:8080` and
    // `example.com:443` lose the port; `::1` does not.
    if pat.starts_with('[') {
        if let Some(end) = pat.find("]:") {
            let suffix = &pat[end + 2..];
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                pat.truncate(end + 1); // keep the trailing `]`
            }
        }
    } else if pat.matches(':').count() == 1 {
        if let Some(idx) = pat.rfind(':') {
            if pat[idx + 1..].bytes().all(|b| b.is_ascii_digit()) {
                pat.truncate(idx);
            }
        }
    }
    let pat = normalize_host(&pat);
    let needle = pat.strip_prefix('.').unwrap_or(&pat);
    host == needle || host.ends_with(&format!(".{needle}"))
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
    fn no_proxy_ipv6_literal_matches_bare_host() {
        // Naive `split(':')` truncation collapsed `::1` to an empty
        // string, silently breaking loopback bypass for IPv6. The
        // bracketed and bare pattern forms must match a bracketless
        // IPv6 URL host.
        assert!(NoProxy::from_string("::1").unwrap().matches("::1"));
        assert!(NoProxy::from_string("[::1]").unwrap().matches("::1"));
        assert!(NoProxy::from_string("[::1]:8080").unwrap().matches("::1"));
        assert!(NoProxy::from_string("fe80::1").unwrap().matches("fe80::1"));
        assert!(NoProxy::from_string("2001:db8::1,192.0.2.0")
            .unwrap()
            .matches("2001:db8::1"));
    }

    #[test]
    fn no_proxy_ipv6_bracketed_host_matches_pattern() {
        // `url::Host::parse` rejects brackets; the normaliser strips
        // them so `[::1]` on either side compares equal to `::1`.
        assert!(NoProxy::from_string("::1").unwrap().matches("[::1]"));
        assert!(NoProxy::from_string("[::1]").unwrap().matches("[::1]"));
    }

    #[test]
    fn no_proxy_ipv4_port_stripping_still_works() {
        assert!(NoProxy::from_string("192.0.2.1:8080")
            .unwrap()
            .matches("192.0.2.1"));
        assert!(NoProxy::from_string("example.com:443")
            .unwrap()
            .matches("example.com"));
        assert!(NoProxy::from_string(".example.com:443")
            .unwrap()
            .matches("sub.example.com"));
    }

    #[test]
    fn no_proxy_does_not_match_unrelated_ipv6() {
        // Critical negative: `::1` must NOT match `::2`. Guard against
        // bare IPv6 patterns collapsing to a value that matches every
        // IPv6 host.
        assert!(!NoProxy::from_string("::1").unwrap().matches("::2"));
        assert!(!NoProxy::from_string("2001:db8::1")
            .unwrap()
            .matches("2001:db8::2"));
    }

    // ── no-proxy provenance gates ──────────────
    //
    // Env-inherited NO_PROXY may only bypass env-discovered proxies. A
    // stray NO_PROXY on the box silently turning explicitly-proxied
    // traffic DIRECT is a real-IP leak, not a convenience.

    /// A config whose `no_proxy` came from the environment (not the
    /// `.no_proxy()` builder).
    fn cfg_with_env_no_proxy(patterns: &str) -> ProxyConfig {
        ProxyConfig {
            rules: Vec::new(),
            no_proxy: NoProxy::from_string(patterns).unwrap(),
            no_proxy_explicit: false,
            use_env: true,
        }
    }

    #[test]
    fn env_no_proxy_never_bypasses_explicit_proxies() {
        let cfg = cfg_with_env_no_proxy("target.test");
        let url = url::Url::parse("https://target.test/x").unwrap();
        assert_eq!(
            cfg.proxy_for(&url, Some("http://req:1"), None, false),
            Some("http://req:1"),
            "env NO_PROXY bypassed a per-request proxy override"
        );
        assert_eq!(
            cfg.proxy_for(&url, None, Some("http://sess:1"), false),
            Some("http://sess:1"),
            "env NO_PROXY bypassed an explicit session proxy"
        );
        let cfg = cfg_with_env_no_proxy("target.test").all("http://rule:1");
        assert_eq!(
            cfg.proxy_for(&url, None, None, false),
            Some("http://rule:1"),
            "env NO_PROXY bypassed an explicit proxy rule"
        );
    }

    #[test]
    fn env_no_proxy_bypasses_env_derived_proxy() {
        let cfg = cfg_with_env_no_proxy("target.test");
        let url = url::Url::parse("https://target.test/x").unwrap();
        assert_eq!(cfg.proxy_for(&url, None, Some("http://env:1"), true), None);
        // Non-matching hosts still go through the env proxy.
        let other = url::Url::parse("https://other.test/x").unwrap();
        assert_eq!(
            cfg.proxy_for(&other, None, Some("http://env:1"), true),
            Some("http://env:1")
        );
    }

    #[test]
    fn explicit_no_proxy_bypasses_all_proxies() {
        // Set via the builder method → deliberate config → gates
        // everything, per-request overrides included (curl --noproxy
        // semantics, and the pre-fix behaviour for explicit users).
        let cfg = ProxyConfig::new().no_proxy(NoProxy::from_string("target.test").unwrap());
        let url = url::Url::parse("https://target.test/x").unwrap();
        assert_eq!(cfg.proxy_for(&url, Some("http://req:1"), None, false), None);
        assert_eq!(
            cfg.proxy_for(&url, None, Some("http://sess:1"), false),
            None
        );
        assert_eq!(
            cfg.all("http://rule:1").proxy_for(&url, None, None, false),
            None
        );
    }

    #[test]
    fn compression_none_disables_known_codecs() {
        let cfg = CompressionConfig::none();
        assert!(!cfg.allows("gzip"));
        assert!(!cfg.allows("br"));
        assert!(cfg.allows("identity"));
    }

    #[test]
    fn default_timeout_matches_browser_scale_patience() {
        assert_eq!(TimeoutConfig::default().total, Duration::from_secs(300));
    }

    #[test]
    fn proxy_url_validates_supported_schemes_and_hosts() {
        assert_eq!(
            ProxyUrl::parse(" http://proxy.example:8080 ")
                .unwrap()
                .as_str(),
            "http://proxy.example:8080"
        );
        assert!(ProxyUrl::parse("ftp://proxy.example:21").is_err());
        assert!(ProxyUrl::parse("http://").is_err());
        // https:// proxies are rejected at construction: leyline cannot yet
        // TLS-handshake to the proxy, so honoring one would leak CONNECT in
        // cleartext (see connect_through_proxy).
        assert!(ProxyUrl::parse("https://proxy.example:8443").is_err());
    }

    #[test]
    fn proxy_url_constructors_enforce_scheme() {
        assert!(ProxyUrl::http("http://proxy.example:8080").is_ok());
        assert!(ProxyUrl::socks5("socks5://proxy.example:1080").is_ok());
        assert!(ProxyUrl::socks5h("socks5h://proxy.example:1080").is_ok());
        assert!(ProxyUrl::http("socks5://proxy.example:1080").is_err());
        assert!(ProxyUrl::socks5("http://proxy.example:8080").is_err());
    }
}
