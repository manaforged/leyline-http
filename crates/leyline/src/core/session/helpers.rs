use http::{Method, Uri};

use crate::cookie::Jar;
use crate::profile::{Browser, ChromiumBrand, Platform};

use super::{Identity, Session, SessionBuilder};
use crate::core::request::RequestBuilder;
use crate::core::{Request, Response, Result};

impl Session {
    /// Create a new session builder.
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    /// Bare session. Panics if `SessionBuilder::build` fails.
    pub fn new() -> Self {
        Self::builder()
            .build()
            .expect("bare session profile is always valid")
    }

    /// Latest Chrome on Windows. Under the `http3` feature the session races HTTP/3 against HTTP/2 for origins already known to speak HTTP/3 and uses `Auto` elsewhere.
    pub fn chrome() -> Self {
        Self::builder()
            .chrome()
            .build()
            .expect("built-in Chrome profile is always valid")
    }

    /// Firefox [`Browser::default_firefox`] (currently Firefox 154 on Windows).
    pub fn firefox() -> Self {
        Self::builder()
            .firefox()
            .build()
            .expect("built-in Firefox profile is always valid")
    }

    /// The latest bundled Safari profile (currently Safari 26 on macOS).
    pub fn safari() -> Self {
        Self::builder()
            .safari()
            .build()
            .expect("built-in Safari profile is always valid")
    }

    /// Microsoft Edge overlay on [`Browser::default_browser`] (Chrome 152). Under the `http3` feature the session races HTTP/3 against HTTP/2 for origins already known to speak HTTP/3 and uses `Auto` elsewhere.
    pub fn edge() -> Self {
        Self::builder()
            .edge()
            .build()
            .expect("built-in Edge overlay is always valid")
    }

    /// Brave on the latest verified Chromium profile (currently Brave 146). Under the `http3` feature the session races HTTP/3 against HTTP/2 for origins already known to speak HTTP/3 and uses `Auto` elsewhere.
    pub fn brave() -> Self {
        Self::builder()
            .brave()
            .build()
            .expect("built-in Brave profile is always valid")
    }

    /// Opera overlay on [`Browser::default_browser`] (Chrome 152 / Opera 136). Under the `http3` feature the session races HTTP/3 against HTTP/2 for origins already known to speak HTTP/3 and uses `Auto` elsewhere.
    pub fn opera() -> Self {
        Self::builder()
            .opera()
            .build()
            .expect("built-in Opera overlay is always valid")
    }

    /// Vivaldi overlay on Chrome 147, the last major with a recorded Vivaldi build string. Under the `http3` feature the session races HTTP/3 against HTTP/2 for origins already known to speak HTTP/3 and uses `Auto` elsewhere.
    pub fn vivaldi() -> Self {
        Self::builder()
            .vivaldi()
            .build()
            .expect("built-in Vivaldi overlay is always valid")
    }

    /// A session for an explicit browser and platform in one call.
    pub fn profile(browser: Browser, platform: Platform) -> Result<Self> {
        Self::builder().profile(browser, platform).build()
    }

    /// Access the cookie jar.
    pub fn cookies(&self) -> &Jar {
        &self.inner.cookie_jar
    }

    /// Derive a new session from this one that shares the TLS connector, H2/H3 config, and connection pool, but uses an independent cookie jar.
    pub fn with_cookie_jar(&self, cookie_jar: Jar) -> Self {
        let mut s = self.clone();
        std::sync::Arc::make_mut(&mut s.inner).cookie_jar = cookie_jar;
        s
    }

    /// Derive a new session that keeps cookies, TLS, pool, and identity, and only swaps the proxy. Empty or invalid URLs are accepted here and fail per request.
    pub fn with_proxy(&self, proxy_url: &str) -> Self {
        let mut s = self.clone();
        let inner = std::sync::Arc::make_mut(&mut s.inner);
        inner.proxy_config = inner.proxy_config.clone().set_default_proxy(proxy_url);
        s
    }

    /// The impersonated browser, or `None` for a bare (non-impersonating) session — the default when no `.browser(...)` was set.
    pub fn browser(&self) -> Option<Browser> {
        self.inner.browser
    }

    /// The locked presentation, or `None` for a bare session.
    #[must_use]
    pub fn identity(&self) -> Option<Identity> {
        self.inner.identity
    }

    /// Chromium-family overlay (`Edge`, `Opera`, `Vivaldi`), `Some(Chrome)` for stock Chrome, or `None` for every other session.
    pub fn brand(&self) -> Option<ChromiumBrand> {
        match self.inner.brand {
            ChromiumBrand::Chrome => match self.inner.browser {
                Some(browser) if browser.family() == "chrome" => Some(ChromiumBrand::Chrome),
                _ => None,
            },
            other => Some(other),
        }
    }

    /// The platform in use.
    pub fn platform(&self) -> Platform {
        self.inner.platform
    }

    /// Request protocol policy for this session.
    pub fn protocol_policy(&self) -> crate::core::ProtocolPolicy {
        self.inner.protocol_policy
    }

    /// The configured default request timeout.
    pub fn default_timeout(&self) -> std::time::Duration {
        self.inner.timeouts.total
    }

    /// The configured post-send response timeout, if any ([`TimeoutConfig::response_header`](crate::TimeoutConfig::response_header)).
    pub fn response_header_timeout(&self) -> Option<std::time::Duration> {
        self.inner.timeouts.response_header
    }

    /// The session-wide default retry policy, inherited by every request that does not override it via [`crate::RequestBuilder::retry`].
    pub(crate) fn default_retry(&self) -> &crate::core::retry::RetryPolicy {
        &self.inner.default_retry
    }

    /// Observability snapshot of the underlying connection pool.
    pub fn pool_stats(&self) -> crate::PoolStats {
        self.inner.pool.stats()
    }

    /// Start a request with any HTTP method. An unparsable URL surfaces as an error from `send`.
    pub fn request(&self, method: Method, url: impl TryInto<Uri>) -> RequestBuilder {
        match url.try_into() {
            Ok(url) => RequestBuilder::new(self, method, &url.to_string()),
            Err(_) => RequestBuilder::invalid(self, method),
        }
    }

    /// Dispatch an owned [`Request`].
    pub async fn execute(&self, req: Request) -> Result<Response> {
        let mut builder = RequestBuilder::new(self, req.method, &req.url.to_string());
        for (name, value) in req.headers.iter() {
            builder = builder.append_header(name.clone(), value.clone());
        }
        builder = builder.body(req.body);
        if let Some(preset) = req.preset {
            builder = builder.preset(preset);
        }
        if let Some(timeout) = req.timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(policy) = req.retry_policy {
            builder = builder.retry(policy);
        }
        builder = builder.allow_non_idempotent_retry(req.allow_non_idempotent_retry);
        if let Some(auth) = req.digest_auth {
            builder = builder.digest_auth(auth);
        }
        if req.stream_response {
            builder = builder.stream();
        }
        builder.send().await
    }

    /// Start a GET request.
    pub fn get(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::GET, url)
    }

    /// Start a POST request.
    pub fn post(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::POST, url)
    }

    /// Start a PUT request.
    pub fn put(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::PUT, url)
    }

    /// Start a PATCH request.
    pub fn patch(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::PATCH, url)
    }

    /// Start a DELETE request.
    pub fn delete(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::DELETE, url)
    }

    /// Start a HEAD request.
    pub fn head(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, Method::HEAD, url)
    }
}

impl Default for Session {
    /// The default session is **bare** — see [`Session::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
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
            .field("protocol_policy", &self.inner.protocol_policy)
            .field("ja4", &self.inner.audit_tls.ja4)
            .finish()
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
