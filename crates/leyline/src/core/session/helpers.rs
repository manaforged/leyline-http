use crate::cookies::CookieJar;
use crate::profile::{Browser, ChromiumBrand, Platform, Preset};

use super::{Session, SessionBuilder};
use crate::core::request::RequestBuilder;
use crate::core::response::Response;
use crate::core::Result;

impl Session {
    /// Create a new session builder.
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    /// Shortcut to the latest bundled Chrome profile (currently Chrome 147
    /// on Windows). Bumps silently when a new Chrome profile is added —
    /// pin [`Browser::Chrome147`] via the builder if you need a specific
    /// version across releases.
    pub fn chrome_latest() -> Result<Self> {
        Self::builder().build()
    }

    /// Shortcut to the latest bundled Firefox profile (currently Firefox
    /// 148 on Windows). Bumps silently on new releases — pin via the
    /// builder for stability.
    pub fn firefox_latest() -> Result<Self> {
        Self::builder().browser(Browser::Firefox148).build()
    }

    /// Shortcut to the latest bundled Safari profile (currently Safari
    /// 18 on macOS). Bumps silently on new releases — pin via the
    /// builder for stability.
    pub fn safari_latest() -> Result<Self> {
        Self::builder()
            .browser(Browser::Safari18)
            .platform(Platform::MacOS)
            .build()
    }

    /// Microsoft Edge on the latest Chromium profile.
    pub fn edge_latest() -> Result<Self> {
        Self::builder().brand(ChromiumBrand::Edge).build()
    }

    /// Brave on the latest Chromium profile.
    pub fn brave_latest() -> Result<Self> {
        Self::builder().brand(ChromiumBrand::Brave).build()
    }

    /// Opera on the latest Chromium profile we've verified against a
    /// live capture (Chrome 145 / Opera 129).
    pub fn opera_latest() -> Result<Self> {
        Self::builder()
            .browser(Browser::Chrome145)
            .brand(ChromiumBrand::Opera)
            .build()
    }

    /// Access the cookie jar.
    pub fn cookies(&self) -> &CookieJar {
        &self.cookie_jar
    }

    /// The browser profile in use.
    pub fn browser(&self) -> Browser {
        self.browser
    }

    /// The Chromium-family brand overlay applied, or
    /// [`ChromiumBrand::Chrome`] for stock Chrome / non-Chromium
    /// profiles.
    pub fn brand(&self) -> ChromiumBrand {
        self.brand
    }

    /// The platform in use.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// The configured default request timeout. Used by the retry loop
    /// to compute the overall wall-clock deadline across attempts so a
    /// caller's `timeout` bound is never exceeded by backoff.
    pub fn default_timeout(&self) -> std::time::Duration {
        self.timeout
    }

    /// Observability snapshot of the underlying connection pool.
    ///
    /// Cumulative counters plus the instantaneous entry count. Poll
    /// from any thread; counters are atomic.
    pub fn pool_stats(&self) -> crate::pool::PoolStats {
        self.pool.stats()
    }

    // Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬ Fluent request builders Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬
    //
    // Every verb returns a `RequestBuilder` that you chain on and end with
    // `.send().await`. This is the one and only way to issue a request —
    // no parallel `get_request` / `fetch` / bare-async-verb API.

    /// Start a request with any HTTP method.
    ///
    /// ```rust,ignore
    /// session.request("PATCH", url).json(&body).send().await?;
    /// ```
    pub fn request(&self, method: &str, url: &str) -> RequestBuilder<'_> {
        RequestBuilder::new(self, method, url)
    }

    /// Start a GET request.
    ///
    /// ```rust,ignore
    /// let resp = session.get(url).send().await?;
    /// ```
    pub fn get(&self, url: &str) -> RequestBuilder<'_> {
        RequestBuilder::new(self, "GET", url)
    }

    /// Start a POST request.
    pub fn post(&self, url: &str) -> RequestBuilder<'_> {
        RequestBuilder::new(self, "POST", url)
    }

    /// Start a PUT request.
    pub fn put(&self, url: &str) -> RequestBuilder<'_> {
        RequestBuilder::new(self, "PUT", url)
    }

    /// Start a PATCH request.
    pub fn patch(&self, url: &str) -> RequestBuilder<'_> {
        RequestBuilder::new(self, "PATCH", url)
    }

    /// Start a DELETE request.
    pub fn delete(&self, url: &str) -> RequestBuilder<'_> {
        RequestBuilder::new(self, "DELETE", url)
    }

    /// Start a HEAD request.
    pub fn head(&self, url: &str) -> RequestBuilder<'_> {
        RequestBuilder::new(self, "HEAD", url)
    }

    // Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬ Convenience shortcuts Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬Ã¢”â‚¬

    /// GET with the Navigate preset applied — the request looks like a
    /// browser document-fetch (Sec-Fetch-Mode: navigate, etc.). Returns a
    /// fully-executed `Response`; use [`get`](Self::get) if you want a
    /// builder you can chain on.
    pub async fn navigate(&self, url: &str) -> Result<Response> {
        self.get(url).preset(Preset::Navigate).send().await
    }

    /// POST a JSON body with the XHR preset applied.
    pub async fn post_json(&self, url: &str, body: &impl serde::Serialize) -> Result<Response> {
        self.post(url).preset(Preset::Xhr).json(body).send().await
    }

    /// POST URL-encoded form data with the Form preset applied.
    pub async fn post_form(&self, url: &str, params: &[(&str, &str)]) -> Result<Response> {
        self.post(url)
            .preset(Preset::Form)
            .form(params)
            .send()
            .await
    }

    /// POST a pre-encoded form string with the Form preset applied.
    pub async fn post_form_str(&self, url: &str, data: &str) -> Result<Response> {
        self.post(url)
            .preset(Preset::Form)
            .form_str(data)
            .send()
            .await
    }

    /// Dispatch a standalone [`crate::Request`] value. Used by
    /// adapters that can't borrow the session (e.g. the `tower::Service`
    /// implementation in `leyline-tower`).
    ///
    /// For most code, prefer the fluent [`Self::get`] / [`Self::post`]
    /// / [`Self::request`] builders — they're borrow-cheap and avoid
    /// an extra allocation of the `Request` value.
    pub async fn execute_request(&self, req: crate::Request) -> Result<Response> {
        // Route through the fluent builder so retry / digest / stream /
        // timeout options on `Request` carry the same semantics as
        // `session.get(...).retry(...).send()`. Without this, a tower
        // middleware holding a `Request` would silently lose every
        // feature added after the raw `execute_with_timeout` path.
        let mut rb = self.request(&req.method, &req.url);
        for (name, value) in req.headers.iter() {
            rb = rb.header(name, value);
        }
        match req.body {
            crate::Body::Empty => {}
            other => rb = rb.body(other),
        }
        if let Some(t) = req.timeout {
            rb = rb.timeout(t);
        }
        if let Some(policy) = req.retry_policy {
            rb = rb.retry(policy);
        }
        if req.allow_non_idempotent_retry {
            rb = rb.allow_non_idempotent_retry(true);
        }
        if let Some(auth) = req.digest_auth {
            rb = rb.digest_auth(auth);
        }
        if req.stream_response {
            rb = rb.stream();
        }
        rb.send().await
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("browser", &self.browser)
            .field("platform", &self.platform)
            .field("proxy", &self.proxy)
            .field("timeout", &self.timeout)
            .field("max_redirects", &self.max_redirects)
            .field("protocol_policy", &self.protocol_policy)
            .field("ja4", &self.audit_tls.ja4)
            .finish()
    }
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Session({}, {}, proxy={})",
            self.browser,
            self.platform,
            self.proxy.as_deref().unwrap_or("none")
        )
    }
}
