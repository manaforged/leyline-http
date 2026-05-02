use crate::cookie::Jar as CookieJar;
use crate::profile::{Browser, ChromiumBrand, Platform, Preset};

use super::{Session, SessionBuilder};
use crate::core::request::RequestBuilder;
use crate::core::response::Response;
use crate::core::{IntoParamPair, Result};

impl Session {
    /// Create a new session builder.
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    /// Build a default Chrome session.
    ///
    /// This mirrors the `Client::new()` shape while still
    /// returning `Result` because Leyline builds real TLS/profile state.
    pub fn new() -> Result<Self> {
        Self::chrome_latest()
    }

    /// Shortcut to the latest bundled Chrome profile (currently Chrome 147
    /// on Windows). Bumps silently when a new Chrome profile is added —
    /// pin [`Browser::Chrome147`] via the builder if you need a specific
    /// version across releases.
    pub fn chrome_latest() -> Result<Self> {
        Self::builder().build()
    }

    /// Alias for [`Session::chrome_latest`].
    pub fn chrome() -> Result<Self> {
        Self::chrome_latest()
    }

    /// Shortcut to the latest bundled Firefox profile (currently Firefox
    /// 150 on Windows). Bumps silently on new releases — pin via the
    /// builder for stability.
    pub fn firefox_latest() -> Result<Self> {
        Self::builder().browser(Browser::Firefox150).build()
    }

    /// Alias for [`Session::firefox_latest`].
    pub fn firefox() -> Result<Self> {
        Self::firefox_latest()
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

    /// Alias for [`Session::safari_latest`].
    pub fn safari() -> Result<Self> {
        Self::safari_latest()
    }

    /// Microsoft Edge on the latest Chromium profile.
    pub fn edge_latest() -> Result<Self> {
        Self::builder().brand(ChromiumBrand::Edge).build()
    }

    /// Alias for [`Session::edge_latest`].
    pub fn edge() -> Result<Self> {
        Self::edge_latest()
    }

    /// Brave on the latest verified Chromium profile (currently Brave
    /// 146 on macOS — see [`Browser::Brave146`]).
    pub fn brave_latest() -> Result<Self> {
        Self::builder()
            .browser(Browser::Brave146)
            .platform(Platform::MacOS)
            .build()
    }

    /// Alias for [`Session::brave_latest`].
    pub fn brave() -> Result<Self> {
        Self::brave_latest()
    }

    /// Opera on the latest Chromium profile we have a verified overlay
    /// for (currently Chrome 147 / Opera 131). Track the Opera anchor
    /// table in [`crate::profile::ChromiumBrand`] when bumping.
    pub fn opera_latest() -> Result<Self> {
        Self::builder().brand(ChromiumBrand::Opera).build()
    }

    /// Alias for [`Session::opera_latest`].
    pub fn opera() -> Result<Self> {
        Self::opera_latest()
    }

    /// Vivaldi on the latest Chromium profile we have a verified
    /// overlay for (currently Chrome 147 / Vivaldi 7.9). Vivaldi
    /// deliberately omits its own brand from `sec-ch-ua` by default;
    /// the overlay reflects that.
    pub fn vivaldi_latest() -> Result<Self> {
        Self::builder().brand(ChromiumBrand::Vivaldi).build()
    }

    /// Alias for [`Session::vivaldi_latest`].
    pub fn vivaldi() -> Result<Self> {
        Self::vivaldi_latest()
    }

    /// Build a session for an explicit browser and platform in one call.
    ///
    /// ```rust,ignore
    /// let session = leyline::Session::profile(
    ///     leyline::Browser::Firefox150,
    ///     leyline::Platform::Windows,
    /// )?;
    /// ```
    pub fn profile(browser: Browser, platform: Platform) -> Result<Self> {
        Self::builder().browser(browser).platform(platform).build()
    }

    /// Access the cookie jar.
    pub fn cookies(&self) -> &CookieJar {
        &self.cookie_jar
    }

    /// Derive a new session from this one that shares the TLS connector,
    /// H2/H3 config, and connection pool, but uses an independent cookie
    /// jar. The original session's jar is untouched. Use this when you want
    /// many short-lived cookie scopes while
    /// amortising TLS-handshake cost across them.
    pub fn with_cookie_jar(&self, cookie_jar: CookieJar) -> Self {
        let mut s = self.clone();
        s.cookie_jar = cookie_jar;
        s
    }

    /// Derive a new session from this one that preserves every piece of
    /// state — cookie jar, TLS connector, BoringSSL session cache, H2/H3
    /// config, browser/platform identity, header overlays — and only swaps
    /// the bound proxy. Use this when one identity needs to rotate egress
    /// IPs across many short-lived clones (e.g. per-chunk monitor probes
    /// or sticky-session refresh) without
    /// re-handshaking or losing cookie state.
    ///
    /// The pool keys connections by `(host, port, proxy)`, so the first
    /// request through each (clone, proxy) pair pays one TLS handshake;
    /// subsequent requests through the same proxy reuse the cached
    /// connection on the shared pool.
    ///
    /// For per-request rotation (override the bound proxy on a single
    /// request, e.g. retrying a 429 through a different exit), prefer
    /// [`RequestBuilder::proxy`](crate::RequestBuilder::proxy) — this
    /// method is for cases where a logical scope (one cycle, one chunk,
    /// one retry attempt) wants a stable proxy across many requests.
    pub fn with_proxy(&self, proxy_url: &str) -> Self {
        let mut s = self.clone();
        s.proxy = Some(proxy_url.to_string());
        s.proxy_config = s
            .proxy_config
            .clone()
            .with_rule(crate::core::ProxyRule::all(proxy_url));
        s
    }

    /// Derive a new session that preserves every piece of state and only
    /// changes the redirect-follow cap. Cheap clone (shares the pool /
    /// connector / TLS cache); intended for one-shot calls that want the
    /// raw 3xx response without leyline auto-following — e.g. an order
    /// placement POST where the success signal is the redirect target
    /// itself and the followed body would be a multi-MB confirmation
    /// page wasting bandwidth and time.
    #[must_use]
    pub fn with_max_redirects(&self, n: usize) -> Self {
        let mut s = self.clone();
        s.max_redirects = n;
        s.redirect_policy = crate::core::RedirectPolicy::limited(n);
        s
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
        self.timeouts.total
    }

    /// Observability snapshot of the underlying connection pool.
    ///
    /// Cumulative counters plus the instantaneous entry count. Poll
    /// from any thread; counters are atomic.
    pub fn pool_stats(&self) -> crate::pool::PoolStats {
        self.pool.stats()
    }

    /// True iff `self` and `other` share the same underlying connection
    /// pool (`Arc::ptr_eq` on the inner pool). Cheap pointer comparison;
    /// useful for tests asserting that derivation paths like
    /// [`Session::clone`], [`Session::with_cookie_jar`], and
    /// [`Session::with_proxy`] preserve the pool identity. Two
    /// independently `build()`-ed sessions never share — this check
    /// returns false for them.
    #[must_use]
    pub fn shares_pool(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.pool, &other.pool)
    }

    // Fluent request builders.
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

    // Convenience shortcuts.

    /// GET with the Navigate preset applied — the request looks like a
    /// browser document-fetch (Sec-Fetch-Mode: navigate, etc.). Returns a
    /// fully-executed `Response`; use [`get`](Self::get) if you want a
    /// builder you can chain on.
    pub async fn navigate(&self, url: &str) -> Result<Response> {
        self.get(url).preset(Preset::Navigate).send().await
    }

    /// GET with the Script preset applied.
    pub async fn get_script(&self, url: &str) -> Result<Response> {
        self.get(url).preset(Preset::Script).send().await
    }

    /// POST a JSON body with the XHR preset applied.
    pub async fn post_json(&self, url: &str, body: &impl serde::Serialize) -> Result<Response> {
        self.post(url).preset(Preset::Xhr).json(body).send().await
    }

    /// POST URL-encoded form data with the Form preset applied.
    pub async fn post_form<I, P>(&self, url: &str, params: I) -> Result<Response>
    where
        I: IntoIterator<Item = P>,
        P: IntoParamPair,
    {
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
