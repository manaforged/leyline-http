use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use crate::core::Kind;
use crate::tls::{Resolver, SystemResolver};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProxyUrl(String);

impl std::fmt::Debug for ProxyUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProxyUrl").field(&redact(&self.0)).finish()
    }
}

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

    pub fn http(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "http")
    }

    pub fn https(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "https")
    }

    pub fn socks5(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "socks5")
    }

    pub fn socks5h(raw: impl AsRef<str>) -> crate::core::Result<Self> {
        Self::parse_scheme(raw, "socks5h")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

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

#[derive(Clone)]
pub struct ProxyConfig {
    rules: Vec<ProxyRule>,
    no_proxy: NoProxy,
    no_proxy_explicit: bool,
    use_env: bool,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_rule(mut self, rule: ProxyRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub(crate) fn set_default_proxy(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.retain(|r| r.scheme != ProxyRuleScheme::All);
        self.rules.push(ProxyRule::all(proxy_url));
        self.from_env = false;
        self
    }

    pub(crate) fn set_from_env(mut self) -> Self {
        self.from_env = true;
        self
    }

    pub(crate) fn rules(&self) -> &[ProxyRule] {
        &self.rules
    }

    pub(crate) fn primary(&self) -> Option<&str> {
        self.rules
            .iter()
            .find(|r| r.scheme == ProxyRuleScheme::All)
            .or_else(|| self.rules.first())
            .map(|r| r.url.as_str())
    }

    pub fn all(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.push(ProxyRule::all(proxy_url));
        self
    }

    pub fn no_proxy(mut self, no_proxy: NoProxy) -> Self {
        self.no_proxy = no_proxy;
        self.no_proxy_explicit = true;
        self
    }

    pub fn without_env(mut self) -> Self {
        self.use_env = false;
        self
    }

    pub fn uses_env(&self) -> bool {
        self.use_env
    }

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
    pub fn all(proxy_url: impl Into<String>) -> Self {
        Self {
            scheme: ProxyRuleScheme::All,
            url: proxy_url.into(),
        }
    }

    pub fn http(proxy_url: impl Into<String>) -> Self {
        Self {
            scheme: ProxyRuleScheme::Http,
            url: proxy_url.into(),
        }
    }

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoProxy {
    patterns: Vec<String>,
}

impl NoProxy {
    pub fn from_string(raw: &str) -> Option<Self> {
        let patterns: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        (!patterns.is_empty()).then_some(Self { patterns })
    }

    pub fn from_env() -> Option<Self> {
        std::env::var("NO_PROXY")
            .ok()
            .or_else(|| std::env::var("no_proxy").ok())
            .and_then(|raw| Self::from_string(&raw))
    }

    pub fn new<I, S>(patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            patterns: patterns.into_iter().map(Into::into).collect(),
        }
    }

    pub fn matches(&self, host: &str) -> bool {
        let host = normalize_host(host);
        self.patterns.iter().any(|raw| pattern_matches(&host, raw))
    }
}

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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn resolver(mut self, resolver: Arc<dyn Resolver>) -> Self {
        self.resolver = resolver;
        self
    }

    pub fn resolve_host(mut self, host: impl AsRef<str>, addr: SocketAddr) -> Self {
        self.overrides
            .insert(normalize_host(host.as_ref()), vec![addr]);
        self
    }

    pub fn resolve_host_to_addrs<I>(mut self, host: impl AsRef<str>, addrs: I) -> Self
    where
        I: IntoIterator<Item = SocketAddr>,
    {
        self.overrides
            .insert(normalize_host(host.as_ref()), addrs.into_iter().collect());
        self
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TimeoutConfig {
    pub total: Duration,
    pub connect: Option<Duration>,
    pub read: Option<Duration>,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total(mut self, d: Duration) -> Self {
        self.total = d;
        self
    }

    pub fn connect(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.connect = d.into();
        self
    }

    pub fn read(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.read = d.into();
        self
    }

    pub fn response_header(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.response_header = d.into();
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolConfig {
    pub idle_timeout: Duration,
    pub max_connections: usize,
    pub max_h1_conns_per_host: usize,
    pub keepalive: bool,
    pub h2_ping_after_idle: Option<Duration>,
    pub h2_ping_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            idle_timeout: crate::pool::DEFAULT_IDLE_TIMEOUT,
            max_connections: crate::pool::DEFAULT_MAX_CONNECTIONS,
            max_h1_conns_per_host: crate::pool::DEFAULT_MAX_H1_CONNS_PER_HOST,
            keepalive: true,
            h2_ping_after_idle: crate::pool::DEFAULT_H2_PING_AFTER_IDLE,
            h2_ping_timeout: crate::pool::DEFAULT_H2_PING_TIMEOUT,
        }
    }
}

impl PoolConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn idle_timeout(mut self, d: Duration) -> Self {
        self.idle_timeout = d;
        self
    }

    pub fn max_connections(mut self, n: usize) -> Self {
        self.max_connections = n;
        self
    }

    pub fn max_h1_conns_per_host(mut self, n: usize) -> Self {
        self.max_h1_conns_per_host = n;
        self
    }

    pub fn keepalive(mut self, on: bool) -> Self {
        self.keepalive = on;
        self
    }

    pub fn h2_ping_after_idle(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.h2_ping_after_idle = d.into();
        self
    }

    pub fn h2_ping_timeout(mut self, d: Duration) -> Self {
        self.h2_ping_timeout = d;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SocketConfig {
    pub local_address: Option<IpAddr>,
    pub local_ipv4: Option<Ipv4Addr>,
    pub local_ipv6: Option<Ipv6Addr>,
    pub tcp_nodelay: Option<bool>,
    pub tcp_keepalive: Option<Duration>,
    pub tcp_keepalive_interval: Option<Duration>,
    pub tcp_keepalive_retries: Option<u32>,
    pub tcp_user_timeout: Option<Duration>,
    pub send_buffer_size: Option<usize>,
    pub recv_buffer_size: Option<usize>,
    pub interface: Option<String>,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn local_address(mut self, addr: impl Into<Option<IpAddr>>) -> Self {
        self.local_address = addr.into();
        self
    }

    pub fn local_ipv4(mut self, addr: impl Into<Option<Ipv4Addr>>) -> Self {
        self.local_ipv4 = addr.into();
        self
    }

    pub fn local_ipv6(mut self, addr: impl Into<Option<Ipv6Addr>>) -> Self {
        self.local_ipv6 = addr.into();
        self
    }

    pub fn tcp_nodelay(mut self, on: impl Into<Option<bool>>) -> Self {
        self.tcp_nodelay = on.into();
        self
    }

    pub fn tcp_keepalive(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_keepalive = d.into();
        self
    }

    pub fn tcp_keepalive_interval(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_keepalive_interval = d.into();
        self
    }

    pub fn tcp_keepalive_retries(mut self, n: impl Into<Option<u32>>) -> Self {
        self.tcp_keepalive_retries = n.into();
        self
    }

    pub fn tcp_user_timeout(mut self, d: impl Into<Option<Duration>>) -> Self {
        self.tcp_user_timeout = d.into();
        self
    }

    pub fn send_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.send_buffer_size = n.into();
        self
    }

    pub fn recv_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.recv_buffer_size = n.into();
        self
    }

    pub fn interface(mut self, name: impl Into<String>) -> Self {
        self.interface = Some(name.into());
        self
    }

    pub fn strict(mut self, on: bool) -> Self {
        self.strict = on;
        self
    }
}

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
    pub fn limited(max: usize) -> Self {
        Self {
            kind: RedirectKind::Limited(max),
        }
    }

    pub fn none() -> Self {
        Self {
            kind: RedirectKind::None,
        }
    }

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

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct RedirectAttempt<'a> {
    pub status: u16,
    pub url: &'a http::Uri,
    pub location: Option<&'a str>,
    pub previous: &'a [String],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RedirectAction {
    Follow,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompressionConfig {
    pub gzip: bool,
    pub brotli: bool,
    pub deflate: bool,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn none() -> Self {
        Self {
            gzip: false,
            brotli: false,
            deflate: false,
            zstd: false,
        }
    }

    pub fn gzip(mut self, on: bool) -> Self {
        self.gzip = on;
        self
    }

    pub fn brotli(mut self, on: bool) -> Self {
        self.brotli = on;
        self
    }

    pub fn deflate(mut self, on: bool) -> Self {
        self.deflate = on;
        self
    }

    pub fn zstd(mut self, on: bool) -> Self {
        self.zstd = on;
        self
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebSocketConfig {
    pub prefer_http2: bool,
    pub max_frame_size: Option<usize>,
    pub max_message_size: Option<usize>,
    pub read_buffer_size: Option<usize>,
    pub write_buffer_size: Option<usize>,
    pub max_write_buffer_size: Option<usize>,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prefer_http2(mut self, on: bool) -> Self {
        self.prefer_http2 = on;
        self
    }

    pub fn max_frame_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_frame_size = n.into();
        self
    }

    pub fn max_message_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_message_size = n.into();
        self
    }

    pub fn read_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.read_buffer_size = n.into();
        self
    }

    pub fn write_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.write_buffer_size = n.into();
        self
    }

    pub fn max_write_buffer_size(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_write_buffer_size = n.into();
        self
    }

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
