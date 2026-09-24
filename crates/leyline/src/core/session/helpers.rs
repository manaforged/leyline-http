use http::{Method, Uri};

use crate::cookie::Jar;
#[cfg(test)]
use crate::profile::ChromiumBrand;
use crate::profile::{Browser, Platform, Preset};

#[cfg(test)]
use super::Identity;
use super::{Session, SessionBuilder};
use crate::core::request::RequestBuilder;
use crate::core::retry::RetryPolicy;
use crate::core::{Body, Response, Result, TimeoutConfig};

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

    pub(crate) fn browser(&self) -> Option<Browser> {
        self.inner.browser
    }

    #[cfg(test)]
    pub(crate) fn identity(&self) -> Option<Identity> {
        self.inner.identity
    }

    #[cfg(test)]
    pub(crate) fn brand(&self) -> Option<ChromiumBrand> {
        match self.inner.brand {
            ChromiumBrand::Chrome => match self.inner.browser {
                Some(browser) if browser.family() == "chrome" => Some(ChromiumBrand::Chrome),
                _ => None,
            },
            other => Some(other),
        }
    }

    #[cfg(test)]
    pub(crate) fn platform(&self) -> Platform {
        self.inner.platform
    }

    #[cfg(test)]
    pub(crate) fn protocol_policy(&self) -> crate::core::ProtocolPolicy {
        self.inner.protocol_policy
    }

    pub(crate) fn default_retry(&self) -> &crate::core::retry::RetryPolicy {
        &self.inner.default_retry
    }

    pub fn pool_stats(&self) -> crate::PoolStats {
        self.inner.pool.stats()
    }

    pub async fn preconnect(&self, url: &str, proxy: Option<&str>) -> Result<()> {
        let url = url::Url::parse(url).map_err(crate::core::Error::from_url_parse)?;
        if url.scheme() != "https" || self.inner.protocol_policy == super::ProtocolPolicy::Http1 {
            return Ok(());
        }
        let host = url.host_str().unwrap_or("");
        let port = url.port_or_known_default().unwrap_or(443);
        let proxy = self.inner.proxy_config.proxy_for(&url, proxy);
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

    pub fn request(&self, method: Method, url: impl TryInto<Uri>) -> RequestBuilder {
        match url.try_into() {
            Ok(url) => RequestBuilder::new(self, method, &url.to_string()),
            Err(_) => RequestBuilder::invalid(self, method),
        }
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
                &self.inner.proxy_config.primary().map(crate::util::redact),
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
                    .map(crate::util::redact)
                    .unwrap_or_else(|| "none".into())
            ),
            None => write!(
                f,
                "Session(bare, {}, proxy={})",
                self.inner.platform,
                self.inner
                    .proxy_config
                    .primary()
                    .map(crate::util::redact)
                    .unwrap_or_else(|| "none".into())
            ),
        }
    }
}
