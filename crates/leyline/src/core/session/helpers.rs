use std::future::Future;

use http::Method;

use crate::cookie::Jar;
use crate::profile::{Browser, RequestKind};

use super::{Session, SessionBuilder};
use crate::core::Redirect;
use crate::core::config::HostPass;
use crate::core::config::Proxy;
use crate::core::into_url::IntoUrl;
use crate::core::request::RequestBuilder;
use crate::core::response::Response;
use crate::core::retry::Retry;
use crate::deadline::Timeout;
use crate::error::{Error, Result};
use crate::net::ProxyUrl;
use crate::wire::Body;

impl Session {
    /// Returns a `SessionBuilder` with default settings.
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    /// Returns a session built with default settings.
    pub fn new() -> Self {
        Self::builder().into_builtin()
    }

    /// Returns a new `Tab` on this session with no page open.
    pub fn tab(&self) -> crate::core::Tab {
        crate::core::Tab::new(self.clone())
    }

    /// Returns the session cookie jar.
    pub fn cookies(&self) -> &Jar {
        self.inner.inputs.jar()
    }

    pub(crate) fn proxy_url(&self) -> Option<ProxyUrl> {
        self.inner.inputs.proxy_config.primary_url().cloned()
    }

    /// Returns the primary proxy URL with its credentials masked.
    #[must_use]
    pub fn proxy(&self) -> Option<&str> {
        self.inner.proxy_shown.as_deref()
    }

    pub(crate) fn base_url(&self) -> Option<&url::Url> {
        self.inner.inputs.base_url.as_ref()
    }

    pub(super) fn changed(&self, change: impl FnOnce(&mut super::SessionInner)) -> Self {
        let mut s = self.clone();
        change(std::sync::Arc::make_mut(&mut s.inner));
        s
    }

    /// Returns the session's current pool, per-host, and per-proxy counters.
    #[must_use]
    pub fn stats(&self) -> crate::core::Stats {
        crate::core::Stats::new(
            self.inner.pool.stats(),
            self.inner.host_limits.stats(),
            self.inner
                .inputs
                .proxy_pool
                .as_ref()
                .map(crate::core::proxy_pool::ProxyPool::stats)
                .unwrap_or_default(),
        )
    }

    #[cfg(feature = "html")]
    pub(crate) fn host_limits(&self) -> &crate::core::config::HostGates {
        &self.inner.host_limits
    }

    pub(crate) fn per_core(&self) -> Self {
        let mut session = self.clone();
        std::sync::Arc::make_mut(&mut session.inner).pool =
            std::sync::Arc::new(self.inner.pool.sibling());
        session
    }

    #[cfg(feature = "html")]
    pub(crate) fn upgraded(&self, url: &url::Url) -> url::Url {
        url::Url::clone(&self.hsts_upgrade(&std::sync::Arc::new(url.clone())))
    }

    #[cfg(feature = "html")]
    pub(crate) fn with_extra_gates(&self, gates: crate::core::config::HostGates) -> Self {
        self.changed(|inner| inner.extra_gates = Some(gates))
    }

    pub(crate) fn admit_host(&self, url: &url::Url) -> impl Future<Output = Option<HostPass>> {
        self.inner
            .host_limits
            .admit_with(self.inner.extra_gates.as_ref(), url)
    }

    pub(crate) fn observe_host(
        &self,
        url: &url::Url,
        status: http::StatusCode,
        wait: impl Fn() -> Option<std::time::Duration>,
    ) {
        for gates in std::iter::once(&self.inner.host_limits).chain(&self.inner.extra_gates) {
            gates.observe(url, status, &wait);
        }
    }

    pub(crate) fn proxy_pool(&self) -> Option<&crate::core::proxy_pool::ProxyPool> {
        self.inner.inputs.proxy_pool.as_ref()
    }

    pub(crate) fn without_proxy_pool(&self) -> Self {
        self.changed(|inner| {
            inner.inputs.proxy_pool = None;
        })
    }

    pub(crate) fn downgrade(&self) -> std::sync::Weak<super::SessionInner> {
        std::sync::Arc::downgrade(&self.inner)
    }

    pub(crate) fn upgrade(weak: &std::sync::Weak<super::SessionInner>) -> Option<Self> {
        weak.upgrade().map(|inner| Self { inner })
    }

    pub(crate) fn is_inner(&self, weak: &std::sync::Weak<super::SessionInner>) -> bool {
        std::ptr::eq(weak.as_ptr(), std::sync::Arc::as_ptr(&self.inner))
    }

    pub(crate) fn impersonates(&self) -> bool {
        self.inner.impersonates
    }

    pub(crate) fn default_retry(&self) -> &crate::core::retry::Retry {
        &self.inner.inputs.default_retry
    }

    pub(crate) fn body_rules(&self) -> crate::core::BodyRules {
        self.inner.inputs.compression
    }

    /// Opens a pooled connection to an HTTPS `url` ahead of the first request.
    pub async fn preconnect(&self, url: impl IntoUrl) -> Result<()> {
        let url = url.into_url()?;
        self.unless_shut_down(self.warm(url)).await
    }

    async fn warm(&self, url: url::Url) -> Result<()> {
        if crate::wire::Scheme::of(&url) != Some(crate::wire::Scheme::Https)
            || !self.inner.inputs.protocol_policy.dials_h2()
        {
            return Ok(());
        }
        let (host, port) = crate::wire::host_port(&url)?;
        let proxy = self.proxy_for(&url, None)?;
        crate::pool::checkout_handle(
            &self.inner.pool,
            &self.inner.connector,
            &self.inner.h2_config,
            host,
            port,
            proxy,
        )
        .await
        .map(drop)
    }

    /// Returns a request builder for `method` and `url`.
    pub fn request(&self, method: Method, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, method, url)
    }

    /// Sends an `http::Request` and applies the request options found in its extensions.
    pub async fn execute(&self, req: http::Request<Body>) -> Result<Response> {
        self.http_request(req).send().await
    }

    pub(crate) fn http_request(&self, req: http::Request<Body>) -> RequestBuilder {
        let (mut parts, body) = req.into_parts();
        let url = url::Url::parse(&parts.uri.to_string());
        let mut builder = RequestBuilder::new(
            self,
            parts.method,
            url.as_ref().ok().cloned().map(std::sync::Arc::new),
        );
        if let Err(err) = url {
            builder.fail(Error::from(err));
        }
        for (name, value) in &parts.headers {
            builder = builder.header(name.clone(), value.clone());
        }
        builder = builder.body(body);
        if let Some(preset) = parts.extensions.remove::<RequestKind>() {
            builder = builder.kind(preset);
        }
        if let Some(timeouts) = parts.extensions.remove::<Timeout>() {
            builder = builder.timeout(timeouts);
        }
        if let Some(policy) = parts.extensions.remove::<Retry>() {
            builder = builder.retry(policy);
        }
        if let Some(policy) = parts.extensions.remove::<Redirect>() {
            builder = builder.redirect(policy);
        }
        if let Some(proxy) = parts.extensions.remove::<Proxy>() {
            builder = builder.proxy(proxy);
        }
        builder
    }

    /// Returns a `GET` request builder for `url`.
    pub fn get(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::GET, url)
    }

    /// Returns a `POST` request builder for `url`.
    pub fn post(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::POST, url)
    }

    /// Returns a `PUT` request builder for `url`.
    pub fn put(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::PUT, url)
    }

    /// Returns a `PATCH` request builder for `url`.
    pub fn patch(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::PATCH, url)
    }

    /// Returns a `DELETE` request builder for `url`.
    pub fn delete(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::DELETE, url)
    }

    /// Returns a `HEAD` request builder for `url`.
    pub fn head(&self, url: impl IntoUrl) -> RequestBuilder {
        RequestBuilder::from_url(self, Method::HEAD, url)
    }
}

pub(super) fn base_url(url: impl IntoUrl) -> Result<url::Url> {
    let url = url.into_url()?;
    if url.cannot_be_a_base() {
        return Err(crate::error::Error::config(format!(
            "base URL {url} cannot be a base"
        )));
    }
    Ok(url)
}

pub(in crate::core::session) fn header_values(
    headers: &http::HeaderMap,
    name: http::HeaderName,
) -> impl Iterator<Item = &http::HeaderValue> {
    headers.get_all(name).iter()
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
            .field(
                "browser",
                &self
                    .inner
                    .browser
                    .map(|b| b.branded(self.inner.inputs.brand)),
            )
            .field("platform", &self.inner.platform)
            .field(
                "proxy",
                &self
                    .inner
                    .inputs
                    .proxy_config
                    .primary()
                    .map(crate::util::redact),
            )
            .field("timeout", &self.inner.inputs.timeouts.total_limit())
            .field("protocol_policy", &self.inner.inputs.protocol_policy);
        if let Some(audit) = &self.inner.audit_tls {
            debug.field("ja4", &audit.tcp().ja4());
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
        let config = &self.inner.inputs.proxy_config;
        let proxy = match config.primary() {
            _ if config.rejects() => "rejected".into(),
            Some(url) => crate::util::redact(url),
            None => "none".into(),
        };
        write!(
            f,
            "Session({label}, {}, proxy={proxy})",
            self.inner.platform
        )
    }
}

impl From<Browser> for Session {
    fn from(browser: Browser) -> Self {
        Self::builder().identity(browser).into_builtin()
    }
}
