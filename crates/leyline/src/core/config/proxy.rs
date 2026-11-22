use std::borrow::Cow;
use std::sync::Arc;
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

type Chooser = dyn Fn(&url::Url) -> Option<Proxy> + Send + Sync;

#[derive(Clone)]
pub(super) struct Choose(Arc<Chooser>);

impl std::panic::UnwindSafe for Choose {}

impl std::panic::RefUnwindSafe for Choose {}

#[doc = include_str!("../../../docs/proxy.md")]
#[derive(Clone)]
#[non_exhaustive]
#[must_use]
pub struct Proxy {
    pub(super) rules: Vec<ProxyRule>,
    pub(super) headers: Vec<(String, String)>,
    pub(super) header_error: Option<String>,
    pub(super) no_proxy: NoProxy,
    pub(super) no_proxy_explicit: bool,
    pub(super) use_env: bool,
    pub(super) from_env: bool,
    pub(super) rejected: Option<RejectedProxy>,
    pub(crate) pool: Option<ProxyPool>,
    pub(super) custom: Option<Choose>,
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
            .field("headers", &ProxyUrl::masked_headers(&self.headers))
            .field("no_proxy", &self.no_proxy)
            .field("no_proxy_explicit", &self.no_proxy_explicit)
            .field("use_env", &self.use_env)
            .field("from_env", &self.from_env)
            .field("rejected", &self.rejected)
            .field("pool", &self.pool)
            .field("custom", &self.custom.is_some())
            .finish()
    }
}

impl Default for Proxy {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            headers: Vec::new(),
            header_error: None,
            no_proxy: NoProxy::default(),
            no_proxy_explicit: false,
            use_env: true,
            from_env: false,
            rejected: None,
            pool: None,
            custom: None,
        }
    }
}

const FORBIDDEN_PROXY_HEADERS: [&str; 3] = ["host", "content-length", "transfer-encoding"];

impl Proxy {
    /// Returns the default settings: environment proxies on and no explicit rules.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets one proxy URL for every scheme, with any username and password inside it as in `http://user:pass@host:8080`, replacing any earlier all-scheme proxy.
    pub fn all(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.retain(|r| r.scheme != ProxyRuleScheme::All);
        self.rules.push(ProxyRule::new(
            ProxyRuleScheme::All,
            proxy_url,
            &self.headers,
        ));
        self.from_env = false;
        self
    }

    /// Adds a proxy for `http` URLs.
    pub fn http(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.push(ProxyRule::new(
            ProxyRuleScheme::Http,
            proxy_url,
            &self.headers,
        ));
        self
    }

    /// Adds a proxy for `https` URLs.
    pub fn https(mut self, proxy_url: impl Into<String>) -> Self {
        self.rules.push(ProxyRule::new(
            ProxyRuleScheme::Https,
            proxy_url,
            &self.headers,
        ));
        self
    }

    /// Adds one custom header sent to HTTP and HTTPS proxies on `CONNECT` and plain-HTTP proxy requests.
    pub fn header(
        self,
        name: impl TryInto<http::HeaderName>,
        value: impl TryInto<http::HeaderValue>,
    ) -> Self {
        self.headers([(name, value)])
    }

    /// Adds custom headers sent to HTTP and HTTPS proxies on `CONNECT` and plain-HTTP proxy requests.
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
                (Ok(name), Ok(value)) => {
                    let key = name.as_str();
                    if FORBIDDEN_PROXY_HEADERS.contains(&key) {
                        self.record_header_error(format!(
                            "proxy header `{key}` cannot be overridden"
                        ));
                        continue;
                    }
                    let Ok(val) = std::str::from_utf8(value.as_bytes()) else {
                        self.record_header_error(format!(
                            "invalid proxy header `{}`",
                            key.escape_debug()
                        ));
                        continue;
                    };
                    let (k, v) = (key.to_owned(), val.to_owned());
                    self.push_validated_headers(std::slice::from_ref(&(k, v)));
                }
                (Ok(name), Err(_)) => self.record_header_error(format!(
                    "invalid proxy header `{}`",
                    name.as_str().escape_debug()
                )),
                (Err(err), _) => self.record_header_error(err.to_string()),
            }
        }
        self
    }

    pub(crate) fn push_validated_headers(&mut self, headers: &[(String, String)]) {
        for (k, v) in headers {
            for rule in &mut self.rules {
                if let Ok(proxy) = &mut rule.parsed {
                    proxy.push_header(k.clone(), v.clone());
                }
            }
            self.headers.push((k.clone(), v.clone()));
        }
        if let Some(pool) = self.pool.take() {
            self.pool = Some(pool.with_headers(headers));
        }
    }

    fn record_header_error(&mut self, msg: String) {
        if self.header_error.is_none() {
            self.header_error = Some(msg);
        }
    }

    pub(crate) fn check_headers(&self) -> crate::error::Result<()> {
        if let Some(err) = &self.header_error {
            return Err(Error::config(err.clone()));
        }
        Ok(())
    }

    pub(crate) fn same_for_doh(&self, other: &Self) -> bool {
        self.use_env == other.use_env
            && self.no_proxy_explicit == other.no_proxy_explicit
            && self.headers == other.headers
            && self.rules.len() == other.rules.len()
            && self
                .rules
                .iter()
                .zip(&other.rules)
                .all(|(a, b)| a.scheme == b.scheme && a.url() == b.url())
            && match (&self.custom, &other.custom) {
                (Some(a), Some(b)) => Arc::ptr_eq(&a.0, &b.0),
                (None, None) => true,
                _ => false,
            }
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

    /// Calls `choose` once with the URL of each request and each redirect hop: `None` sends it with no proxy, and `Some` must hold one proxy, so a `Proxy::list`, `Proxy::identified` or `Proxy::custom` fails the request with `Kind::Config`; of the other setters only `no_proxy` applies.
    pub fn custom<F>(choose: F) -> Self
    where
        F: Fn(&url::Url) -> Option<Proxy> + Send + Sync + 'static,
    {
        Self {
            custom: Some(Choose(Arc::new(choose))),
            ..Self::new()
        }
    }

    pub(crate) fn pooled(pool: ProxyPool) -> Self {
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

    pub(crate) fn shown(&self) -> Option<String> {
        self.primary().map(redact)
    }

    pub(crate) fn summary(&self, pool: Option<&ProxyPool>) -> Option<String> {
        match pool.or(self.pool.as_ref()) {
            Some(pool) => Some(format!("pool({})", pool.len())),
            None if self.is_custom() => Some("custom".to_owned()),
            None => self.shown(),
        }
    }

    pub(crate) fn is_single(&self) -> bool {
        self.pool.is_none() && self.custom.is_none()
    }

    pub(crate) fn is_pool(&self) -> bool {
        self.pool.is_some()
    }

    pub(crate) fn primary_url(&self) -> Option<&ProxyUrl> {
        self.primary_rule()
            .and_then(|rule| rule.parsed.as_ref().ok())
    }

    fn primary_rule(&self) -> Option<&ProxyRule> {
        if self.is_custom() {
            return None;
        }
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
        self.use_env && !self.is_custom()
    }

    pub(crate) fn is_custom(&self) -> bool {
        self.custom.is_some()
    }

    pub(crate) fn proxy_for(
        &self,
        url: &url::Url,
    ) -> crate::error::Result<Option<Cow<'_, ProxyUrl>>> {
        let Some(choose) = &self.custom else {
            return Ok(self.rule_for(url)?.map(Cow::Borrowed));
        };
        if self.bypasses(url.host_str().unwrap_or("")) {
            return Ok(None);
        }
        let Some(chosen) = (choose.0)(url) else {
            return Ok(None);
        };
        if !chosen.is_single() {
            return Err(Error::config(
                "a Proxy::custom closure must return one proxy, not a Proxy::list, \
                 Proxy::identified or Proxy::custom",
            ));
        }
        Ok(chosen.rule_for(url)?.cloned().map(Cow::Owned))
    }

    fn rule_for(&self, url: &url::Url) -> crate::error::Result<Option<&ProxyUrl>> {
        self.check_headers()?;
        if let Some(rejected) = self.rejected {
            return Err(rejected.error(Kind::Proxy));
        }
        let host = url.host_str().unwrap_or("");
        let scheme = Scheme::of(url);
        let Some(winner) = self.rules.iter().find(|rule| rule.matches(scheme)) else {
            return Ok(None);
        };
        if self.bypasses(host) {
            return Ok(None);
        }
        winner.proxy().map(Some)
    }

    fn bypasses(&self, host: &str) -> bool {
        (self.no_proxy_explicit || self.from_env) && self.no_proxy.matches(host)
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
    pub(super) fn new(
        scheme: ProxyRuleScheme,
        proxy_url: impl Into<String>,
        headers: &[(String, String)],
    ) -> Self {
        let raw = proxy_url.into();
        let parsed = ProxyUrl::parse(&raw)
            .map(|p| {
                if headers.is_empty() {
                    p
                } else {
                    p.with_headers(headers.to_vec())
                }
            })
            .ok()
            .ok_or(raw);
        Self { scheme, parsed }
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
