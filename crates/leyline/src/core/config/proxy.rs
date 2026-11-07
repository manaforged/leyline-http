use std::time::Duration;

use crate::core::proxy_pool::ProxyPool;
use crate::error::Error;
use crate::error::Kind;
use crate::net::ProxyUrl;
use crate::profile::{Browser, Platform};
use crate::util::redact;
use crate::wire::Scheme;

use super::host::pattern_matches;
use crate::net::normalize_host;

#[doc = include_str!("../../../docs/proxy.md")]
#[derive(Clone)]
#[non_exhaustive]
#[must_use]
pub struct Proxy {
    pub(super) rules: Vec<ProxyRule>,
    pub(super) no_proxy: NoProxy,
    pub(super) no_proxy_explicit: bool,
    pub(super) use_env: bool,
    pub(super) from_env: bool,
    pub(super) rejected: Option<RejectedProxy>,
    pub(crate) pool: Option<ProxyPool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RejectedProxy {
    variable: &'static str,
}

impl RejectedProxy {
    fn error(self, kind: Kind) -> crate::error::Error {
        crate::error::Error::new(kind).with_message(format!(
            "the {} environment variable is not a valid proxy URL (expected an http, https, \
             socks5, or socks5h URL with a host); fix or unset it, or set a proxy with \
             `SessionBuilder::proxy`",
            self.variable
        ))
    }
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy")
            .field("rules", &self.rules)
            .field("no_proxy", &self.no_proxy)
            .field("no_proxy_explicit", &self.no_proxy_explicit)
            .field("use_env", &self.use_env)
            .field("from_env", &self.from_env)
            .field("rejected", &self.rejected)
            .field("pool", &self.pool)
            .finish()
    }
}

impl Default for Proxy {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            no_proxy: NoProxy::default(),
            no_proxy_explicit: false,
            use_env: true,
            from_env: false,
            rejected: None,
            pool: None,
        }
    }
}

impl Proxy {
    /// Returns the default settings: environment proxies on and no explicit rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets one proxy for every scheme, replacing any earlier all-scheme proxy.
    pub fn all(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.retain(|r| r.scheme != ProxyRuleScheme::All);
        self.rules
            .push(ProxyRule::new(ProxyRuleScheme::All, proxy_url));
        self.from_env = false;
        self
    }

    /// Adds a proxy for `http` URLs.
    pub fn http(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules
            .push(ProxyRule::new(ProxyRuleScheme::Http, proxy_url));
        self
    }

    /// Adds a proxy for `https` URLs.
    pub fn https(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules
            .push(ProxyRule::new(ProxyRuleScheme::Https, proxy_url));
        self
    }

    /// Sets the host patterns that bypass the proxy, replacing the `NO_PROXY` value.
    pub fn no_proxy<I, S>(mut self, patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.no_proxy = NoProxy::new(patterns);
        self.no_proxy_explicit = true;
        self
    }

    /// Sets whether proxies from environment variables apply, on by default.
    pub fn env(mut self, on: bool) -> Self {
        self.use_env = on;
        self
    }

    /// Returns a rotating pool over the given proxies.
    pub fn list<P: Into<Proxy>>(proxies: impl IntoIterator<Item = P>) -> Self {
        Self::pooled(ProxyPool::new(proxies))
    }

    /// Returns a pool where each proxy carries its own browser and platform identity.
    pub fn identified<P: Into<Proxy>>(
        proxies: impl IntoIterator<Item = (P, Browser, Platform)>,
    ) -> Self {
        Self::pooled(ProxyPool::identified(proxies))
    }

    /// Sets how long a pool keeps reusing one proxy, zero by default.
    pub fn sticky_for(self, duration: Duration) -> Self {
        self.map_pool(|pool| pool.sticky_for(duration))
    }

    /// Sets how many failures ban a pooled proxy, 3 by default.
    pub fn ban_after(self, failures: u32) -> Self {
        self.map_pool(|pool| pool.ban_after(failures))
    }

    /// Sets how long a banned pooled proxy stays out, 60 seconds by default.
    pub fn ban_for(self, duration: Duration) -> Self {
        self.map_pool(|pool| pool.ban_for(duration))
    }

    fn pooled(pool: ProxyPool) -> Self {
        Self {
            pool: Some(pool),
            ..Self::new()
        }
    }

    fn map_pool(mut self, change: impl FnOnce(ProxyPool) -> ProxyPool) -> Self {
        self.pool = self.pool.take().map(change);
        self
    }

    pub(crate) fn set_from_env(mut self) -> Self {
        self.from_env = true;
        self
    }

    pub(crate) fn env_no_proxy(mut self, raw: Option<&str>) -> Self {
        if !self.no_proxy_explicit {
            self.no_proxy = raw.and_then(NoProxy::from_string).unwrap_or_default();
        }
        self
    }

    pub(crate) fn reject_env(mut self, variable: &'static str) -> Self {
        self.rejected = Some(RejectedProxy { variable });
        self
    }

    pub(crate) fn rejects(&self) -> bool {
        self.rejected.is_some()
    }

    pub(crate) fn rejection(&self, kind: Kind) -> Option<crate::error::Error> {
        self.rejected.map(|rejected| rejected.error(kind))
    }

    pub(crate) fn rules(&self) -> &[ProxyRule] {
        &self.rules
    }

    pub(crate) fn primary(&self) -> Option<&str> {
        self.primary_rule().map(ProxyRule::url)
    }

    pub(crate) fn primary_url(&self) -> Option<&ProxyUrl> {
        self.primary_rule()
            .and_then(|rule| rule.parsed.as_ref().ok())
    }

    fn primary_rule(&self) -> Option<&ProxyRule> {
        self.rules
            .iter()
            .find(|r| r.scheme == ProxyRuleScheme::All)
            .or_else(|| self.rules.first())
    }

    #[cfg(feature = "http3")]
    pub(crate) fn proxies_every_url(&self) -> bool {
        let bypass =
            (self.no_proxy_explicit || self.from_env) && !self.no_proxy.patterns.is_empty();
        !bypass && self.rules.iter().any(|r| r.scheme == ProxyRuleScheme::All)
    }

    pub(crate) fn uses_env(&self) -> bool {
        self.use_env
    }

    pub(crate) fn proxy_for(&self, url: &url::Url) -> crate::error::Result<Option<&ProxyUrl>> {
        if let Some(rejected) = self.rejected {
            return Err(rejected.error(Kind::Proxy));
        }
        let host = url.host_str().unwrap_or("");
        let scheme = Scheme::of(url);
        let Some(winner) = self.rules.iter().find(|rule| rule.matches(scheme)) else {
            return Ok(None);
        };
        if (self.no_proxy_explicit || self.from_env) && self.no_proxy.matches(host) {
            return Ok(None);
        }
        winner.proxy().map(Some)
    }
}

impl From<&str> for Proxy {
    fn from(proxy_url: &str) -> Self {
        Self::new().all(proxy_url)
    }
}

impl From<&String> for Proxy {
    fn from(proxy_url: &String) -> Self {
        Self::new().all(proxy_url.as_str())
    }
}

impl From<String> for Proxy {
    fn from(proxy_url: String) -> Self {
        Self::new().all(proxy_url)
    }
}

impl From<ProxyUrl> for Proxy {
    fn from(proxy_url: ProxyUrl) -> Self {
        Self::new().all(proxy_url)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProxyRule {
    scheme: ProxyRuleScheme,
    parsed: std::result::Result<ProxyUrl, String>,
}

impl std::fmt::Debug for ProxyRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyRule")
            .field("scheme", &self.scheme)
            .field("url", &redact(self.url()))
            .finish()
    }
}

impl ProxyRule {
    pub(super) fn new(scheme: ProxyRuleScheme, proxy_url: impl Into<String>) -> Self {
        let raw = proxy_url.into();
        Self {
            scheme,
            parsed: ProxyUrl::parse(&raw).ok().ok_or(raw),
        }
    }

    fn matches(&self, scheme: Option<Scheme>) -> bool {
        matches!(
            (self.scheme, scheme),
            (ProxyRuleScheme::All, _)
                | (ProxyRuleScheme::Http, Some(Scheme::Http))
                | (ProxyRuleScheme::Https, Some(Scheme::Https))
        )
    }

    pub(crate) fn url(&self) -> &str {
        match &self.parsed {
            Ok(proxy) => proxy.as_str(),
            Err(raw) => raw,
        }
    }

    pub(crate) fn proxy(&self) -> crate::error::Result<&ProxyUrl> {
        self.parsed
            .as_ref()
            .map_err(|raw| match ProxyUrl::parse(raw) {
                Err(err) => err,
                Ok(_) => Error::config("invalid proxy URL"),
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProxyRuleScheme {
    All,
    Http,
    Https,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NoProxy {
    patterns: Vec<String>,
}

impl NoProxy {
    pub(crate) fn from_string(raw: &str) -> Option<Self> {
        let patterns: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        (!patterns.is_empty()).then_some(Self { patterns })
    }

    pub(crate) fn new<I, S>(patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            patterns: patterns.into_iter().map(Into::into).collect(),
        }
    }

    pub(crate) fn matches(&self, host: &str) -> bool {
        let host = normalize_host(host);
        self.patterns.iter().any(|raw| pattern_matches(&host, raw))
    }
}
