use crate::core::Kind;
use crate::util::redact;

use super::host::{normalize_host, pattern_matches};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProxyUrl(String);

impl std::fmt::Debug for ProxyUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ProxyUrl").field(&redact(&self.0)).finish()
    }
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

    pub(crate) fn configured(raw: &str) -> Self {
        Self(raw.to_owned())
    }

    fn parse_inner(raw: &str) -> crate::core::Result<url::Url> {
        url::Url::parse(raw).map_err(|e| {
            crate::core::Error::new(Kind::Config).with_message(format!("invalid proxy URL: {e}"))
        })
    }

    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProxyUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&redact(&self.0))
    }
}

impl From<ProxyUrl> for String {
    fn from(value: ProxyUrl) -> Self {
        value.0
    }
}

#[derive(Clone)]
#[non_exhaustive]
pub struct ProxyConfig {
    pub(super) rules: Vec<ProxyRule>,
    pub(super) no_proxy: NoProxy,
    pub(super) no_proxy_explicit: bool,
    pub(super) use_env: bool,
    pub(super) from_env: bool,
    pub(super) rejected: Option<RejectedProxy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RejectedProxy {
    variable: &'static str,
}

impl RejectedProxy {
    fn error(self, kind: Kind) -> crate::core::Error {
        crate::core::Error::new(kind).with_message(format!(
            "the {} environment variable is not a valid proxy URL (expected an http, https, \
             socks5, or socks5h URL with a host); fix or unset it, or set a proxy with \
             `SessionBuilder::proxy`",
            self.variable
        ))
    }
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("rules", &self.rules)
            .field("no_proxy", &self.no_proxy)
            .field("no_proxy_explicit", &self.no_proxy_explicit)
            .field("use_env", &self.use_env)
            .field("from_env", &self.from_env)
            .field("rejected", &self.rejected)
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
            rejected: None,
        }
    }
}

impl ProxyConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rule(mut self, rule: ProxyRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn no_proxy(mut self, no_proxy: NoProxy) -> Self {
        self.no_proxy = no_proxy;
        self.no_proxy_explicit = true;
        self
    }

    pub fn env(mut self, on: bool) -> Self {
        self.use_env = on;
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

    pub(crate) fn reject_env(mut self, variable: &'static str) -> Self {
        self.rejected = Some(RejectedProxy { variable });
        self
    }

    pub(crate) fn rejects(&self) -> bool {
        self.rejected.is_some()
    }

    pub(crate) fn rejection(&self, kind: Kind) -> Option<crate::core::Error> {
        self.rejected.map(|rejected| rejected.error(kind))
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

    #[cfg(feature = "http3")]
    pub(crate) fn proxies_every_url(&self) -> bool {
        let bypass =
            (self.no_proxy_explicit || self.from_env) && !self.no_proxy.patterns.is_empty();
        !bypass && self.rules.iter().any(|r| r.scheme == ProxyRuleScheme::All)
    }

    pub(crate) fn uses_env(&self) -> bool {
        self.use_env
    }

    pub(crate) fn proxy_for(&self, url: &url::Url) -> crate::core::Result<Option<&str>> {
        if let Some(rejected) = self.rejected {
            return Err(rejected.error(Kind::Proxy));
        }
        let host = url.host_str().unwrap_or("");
        let Some(winner) = self
            .rules
            .iter()
            .find(|rule| rule.matches(url.scheme()))
            .map(|rule| rule.url.as_str())
        else {
            return Ok(None);
        };
        if (self.no_proxy_explicit || self.from_env) && self.no_proxy.matches(host) {
            return Ok(None);
        }
        Ok(Some(winner))
    }
}

impl From<&str> for ProxyConfig {
    fn from(proxy_url: &str) -> Self {
        Self::new().set_default_proxy(proxy_url)
    }
}

impl From<&String> for ProxyConfig {
    fn from(proxy_url: &String) -> Self {
        Self::new().set_default_proxy(proxy_url.as_str())
    }
}

impl From<String> for ProxyConfig {
    fn from(proxy_url: String) -> Self {
        Self::new().set_default_proxy(proxy_url)
    }
}

impl From<ProxyUrl> for ProxyConfig {
    fn from(proxy_url: ProxyUrl) -> Self {
        Self::new().set_default_proxy(proxy_url)
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
        Self::for_scheme(ProxyRuleScheme::All, proxy_url)
    }

    pub fn http(proxy_url: impl Into<String>) -> Self {
        Self::for_scheme(ProxyRuleScheme::Http, proxy_url)
    }

    pub fn https(proxy_url: impl Into<String>) -> Self {
        Self::for_scheme(ProxyRuleScheme::Https, proxy_url)
    }

    fn for_scheme(scheme: ProxyRuleScheme, proxy_url: impl Into<String>) -> Self {
        Self {
            scheme,
            url: proxy_url.into(),
        }
    }

    fn matches(&self, scheme: &str) -> bool {
        self.scheme == ProxyRuleScheme::All
            || ProxyRuleScheme::for_url_scheme(scheme) == Some(self.scheme)
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyRuleScheme {
    All,
    Http,
    Https,
}

impl ProxyRuleScheme {
    fn for_url_scheme(scheme: &str) -> Option<Self> {
        match scheme {
            "http" | "ws" => Some(Self::Http),
            "https" | "wss" => Some(Self::Https),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct NoProxy {
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

    pub(crate) fn from_env() -> Option<Self> {
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

    pub(crate) fn matches(&self, host: &str) -> bool {
        let host = normalize_host(host);
        self.patterns.iter().any(|raw| pattern_matches(&host, raw))
    }
}
