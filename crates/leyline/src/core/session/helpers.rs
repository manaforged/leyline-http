use http::Method;

use crate::cookie::Jar;
use crate::profile::{Browser, Platform, Preset};

use super::{Session, SessionBuilder};
use crate::core::request::RequestBuilder;
use crate::core::retry::RetryPolicy;
use crate::core::{Body, IntoUrl, ProxyConfig, RedirectPolicy, Response, Result, TimeoutConfig};

impl Session {
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    pub fn new() -> Self {
        let browser = Browser::default_browser();
        let builder = Self::builder().browser(browser).platform(Platform::Windows);
        #[cfg(feature = "http3")]
        let builder = if browser.profile().h3.as_ref().is_some_and(|h3| h3.race) {
            builder.protocol(super::ProtocolPolicy::Race)
        } else {
            builder
        };
        builder.into_builtin()
    }

    pub fn cookies(&self) -> &Jar {
        &self.inner.cookie_jar
    }

    pub fn with_proxy(&self, config: impl Into<ProxyConfig>) -> Self {
        let config = config.into();
        self.derive(|inner| inner.proxy_config = config)
    }

    pub fn fresh_pool(&self) -> Self {
        self.derive(|inner| {
            inner.pool = std::sync::Arc::new(inner.pool.fresh());
            inner.connector = inner.connector.with_fresh_session_cache();
        })
    }

    pub fn with_cookie_jar(&self, jar: Jar) -> Self {
        self.derive(|inner| inner.cookie_jar = jar)
    }

    pub fn with_redirect(&self, policy: RedirectPolicy) -> Self {
        self.derive(|inner| inner.redirect_policy = policy)
    }

    fn derive(&self, change: impl FnOnce(&mut super::SessionInner)) -> Self {
        let mut s = self.clone();
        change(std::sync::Arc::make_mut(&mut s.inner));
        s
    }

    #[cfg(test)]
    pub(crate) fn browser(&self) -> Option<Browser> {
        self.inner.browser
    }

    pub(crate) fn impersonates(&self) -> bool {
        self.inner.impersonates
    }

    pub(crate) fn default_retry(&self) -> &crate::core::retry::RetryPolicy {
        &self.inner.default_retry
    }

    pub fn pool_stats(&self) -> crate::PoolStats {
        self.inner.pool.stats()
    }

    pub async fn preconnect(&self, url: impl IntoUrl) -> Result<()> {
        let url = url.into_url()?;
        if url.scheme() != "https" || self.inner.protocol_policy == super::ProtocolPolicy::Http1 {
            return Ok(());
        }
        let host = url.host_str().unwrap_or("");
        let port = url.port_or_known_default().unwrap_or(443);
        let proxy = self.proxy_for(&url, None)?;
        let opened = crate::pool::checkout_handle(
            &self.inner.pool,
            &self.inner.connector,
            &self.inner.h2_config,
            host,
            port,
            proxy,
        )
        .await;
        match opened {
            Ok(_) => Ok(()),
            Err(error) if error.alpn().is_some() => {
                self.inner.pool.note_h1_only(host, port, proxy);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub fn request(&self, method: Method, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, method, url)
    }

    pub async fn execute(&self, req: http::Request<Body>) -> Result<Response> {
        let (mut parts, body) = req.into_parts();
        let mut builder = RequestBuilder::new(self, parts.method, &parts.uri.to_string());
        for (name, value) in &parts.headers {
            builder = builder.header(name.clone(), value.clone());
        }
        builder = builder.body(body);
        if let Some(preset) = parts.extensions.remove::<Preset>() {
            builder = builder.preset(preset);
        }
        if let Some(timeouts) = parts.extensions.remove::<TimeoutConfig>() {
            builder = builder.timeout(timeouts);
        }
        if let Some(policy) = parts.extensions.remove::<RetryPolicy>() {
            builder = builder.retry(policy);
        }
        if let Some(policy) = parts.extensions.remove::<RedirectPolicy>() {
            builder = builder.redirect(policy);
        }
        builder.send().await
    }

    pub fn get(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::GET, url)
    }

    pub fn post(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::POST, url)
    }

    pub fn put(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::PUT, url)
    }

    pub fn patch(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::PATCH, url)
    }

    pub fn delete(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::DELETE, url)
    }

    pub fn head(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::HEAD, url)
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
                &self.inner.proxy_config.primary().map(crate::util::redact),
            )
            .field("timeout", &self.inner.timeouts.total_limit())
            .field("protocol_policy", &self.inner.protocol_policy);
        if let Some(audit) = &self.inner.audit_tls {
            debug.field("ja4", &audit.ja4);
        }
        debug.finish()
    }
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self.inner.browser {
            Some(browser) => browser.to_string(),
            None if self.inner.impersonates => self.inner.profile.meta.name.clone(),
            None => "bare".to_string(),
        };
        write!(
            f,
            "Session({label}, {}, proxy={})",
            self.inner.platform,
            self.inner
                .proxy_config
                .primary()
                .map(crate::util::redact)
                .unwrap_or_else(|| "none".into())
        )
    }
}
