use http::{Method, Uri};
use std::sync::Arc;

use crate::cookie::Jar;
use crate::profile::{Browser, ChromiumBrand, Platform};

use super::{Identity, Session, SessionBuilder};
use crate::core::request::RequestBuilder;
use crate::core::{RedirectPolicy, Request, Response, Result};

impl Session {
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    pub fn new() -> Self {
        Self::builder().into_builtin()
    }

    pub fn chrome() -> Self {
        Self::builder().chrome().into_builtin()
    }

    pub fn firefox() -> Self {
        Self::builder().firefox().into_builtin()
    }

    pub fn safari() -> Self {
        Self::builder().safari().into_builtin()
    }

    pub fn edge() -> Self {
        Self::builder().edge().into_builtin()
    }

    pub fn brave() -> Self {
        Self::builder().brave().into_builtin()
    }

    pub fn opera() -> Self {
        Self::builder().opera().into_builtin()
    }

    pub fn vivaldi() -> Self {
        Self::builder().vivaldi().into_builtin()
    }

    pub fn profile(browser: Browser, platform: Platform) -> Result<Self> {
        Self::builder().profile(browser, platform).build()
    }

    pub fn cookies(&self) -> &Jar {
        &self.inner.cookie_jar
    }

    pub fn with_cookie_jar(&self, cookie_jar: Jar) -> Self {
        let mut s = self.clone();
        std::sync::Arc::make_mut(&mut s.inner).cookie_jar = cookie_jar;
        s
    }

    pub fn with_proxy(&self, proxy_url: &str) -> Result<Self> {
        let proxy = crate::core::ProxyUrl::parse(proxy_url)?;
        let mut s = self.clone();
        let inner = std::sync::Arc::make_mut(&mut s.inner);
        if inner.proxy_config.primary() == Some(proxy.as_str()) {
            inner.pool = std::sync::Arc::new(inner.pool.fresh());
            inner.connector = inner.connector.with_fresh_session_cache();
        }
        inner.proxy_config = inner.proxy_config.clone().set_default_proxy(proxy.as_str());
        Ok(s)
    }

    pub fn with_redirect_policy(&self, policy: RedirectPolicy) -> Self {
        let mut session = self.clone();
        Arc::make_mut(&mut session.inner).redirect_policy = policy;
        session
    }

    pub fn browser(&self) -> Option<Browser> {
        self.inner.browser
    }

    #[must_use]
    pub fn identity(&self) -> Option<Identity> {
        self.inner.identity
    }

    pub fn brand(&self) -> Option<ChromiumBrand> {
        match self.inner.brand {
            ChromiumBrand::Chrome => match self.inner.browser {
                Some(browser) if browser.family() == "chrome" => Some(ChromiumBrand::Chrome),
                _ => None,
            },
            other => Some(other),
        }
    }

    pub fn platform(&self) -> Platform {
        self.inner.platform
    }

    pub fn protocol_policy(&self) -> crate::core::ProtocolPolicy {
        self.inner.protocol_policy
    }

    pub fn default_timeout(&self) -> std::time::Duration {
        self.inner.timeouts.total
    }

    pub fn response_header_timeout(&self) -> Option<std::time::Duration> {
        self.inner.timeouts.response_header
    }

    pub(crate) fn default_retry(&self) -> &crate::core::retry::RetryPolicy {
        &self.inner.default_retry
    }

    pub fn pool_stats(&self) -> crate::PoolStats {
        self.inner.pool.stats()
    }

    pub async fn preconnect(&self, url: &str) -> Result<()> {
        self.preconnect_via(url, None).await
    }

    pub async fn preconnect_via(&self, url: &str, proxy: Option<&str>) -> Result<()> {
        let url = url::Url::parse(url).map_err(crate::core::Error::from_url_parse)?;
        if url.scheme() != "https" || self.inner.protocol_policy == super::ProtocolPolicy::Http1 {
            return Ok(());
        }
        let host = url.host_str().unwrap_or("");
        let port = url.port_or_known_default().unwrap_or(443);
        let proxy = self.inner.proxy_config.proxy_for(&url, proxy);
        let connect = crate::pool::checkout_handle(
            &self.inner.pool,
            &self.inner.connector,
            &self.inner.h2_config,
            host,
            port,
            proxy,
        );
        let opened = match self.inner.timeouts.connect {
            Some(limit) => tokio::time::timeout(limit, connect).await.map_err(|_| {
                crate::Error::new(crate::Kind::Timeout).with_message(format!(
                    "preconnect to {host}:{port} timed out after {limit:?}"
                ))
            })?,
            None => connect.await,
        };
        match opened {
            Ok(_) => Ok(()),
            Err(error) if error.alpn().is_some() => {
                self.inner.pool.note_h1_only(host, port, proxy);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub fn request(&self, method: Method, url: impl TryInto<Uri>) -> RequestBuilder {
        match url.try_into() {
            Ok(url) => RequestBuilder::new(self, method, &url.to_string()),
            Err(_) => RequestBuilder::invalid(self, method),
        }
    }

    pub async fn execute(&self, req: Request) -> Result<Response> {
        let mut builder = RequestBuilder::new(self, req.method, &req.url.to_string());
        for (name, value) in req.headers.iter() {
            builder = builder.append_header(name.clone(), value.clone());
        }
        builder = builder.body(req.body);
        if let Some(preset) = req.preset {
            builder = builder.preset(preset);
        }
        if let Some(timeouts) = req.timeouts {
            builder = builder.timeouts(timeouts);
        } else if let Some(timeout) = req.timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(policy) = req.retry {
            builder = builder.retry(policy);
        }
        builder = builder.allow_non_idempotent_retry(req.allow_non_idempotent_retry);
        if let Some(auth) = req.digest_auth {
            builder = builder.digest_auth(auth);
        }
        if req.stream {
            builder = builder.stream();
        }
        builder.send().await
    }

    pub fn get(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::GET, url)
    }

    pub fn post(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::POST, url)
    }

    pub fn put(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::PUT, url)
    }

    pub fn patch(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::PATCH, url)
    }

    pub fn delete(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::DELETE, url)
    }

    pub fn head(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::HEAD, url)
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("Session");
        debug
            .field("browser", &self.inner.browser)
            .field("platform", &self.inner.platform)
            .field(
                "proxy",
                &self
                    .inner
                    .proxy_config
                    .primary()
                    .map(crate::core::config::redact),
            )
            .field("timeout", &self.inner.timeouts.total)
            .field("protocol_policy", &self.inner.protocol_policy);
        if let Some(audit) = &self.inner.audit_tls {
            debug.field("ja4", &audit.ja4);
        }
        debug.finish()
    }
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner.browser {
            Some(b) => write!(
                f,
                "Session({}, {}, proxy={})",
                b,
                self.inner.platform,
                self.inner
                    .proxy_config
                    .primary()
                    .map(crate::core::config::redact)
                    .unwrap_or_else(|| "none".into())
            ),
            None => write!(
                f,
                "Session(bare, {}, proxy={})",
                self.inner.platform,
                self.inner
                    .proxy_config
                    .primary()
                    .map(crate::core::config::redact)
                    .unwrap_or_else(|| "none".into())
            ),
        }
    }
}
