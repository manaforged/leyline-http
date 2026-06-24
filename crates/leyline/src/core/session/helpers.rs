use crate::cookie::Jar as CookieJar;
use crate::profile::{Browser, ChromiumBrand, Platform, Preset};

use super::{Session, SessionBuilder};
use crate::core::request::RequestBuilder;
use crate::core::response::Response;
use crate::core::{Body, IntoParamPair, Result};

impl Session {
    /// Create a new session builder.
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    // ── Infallible constructors ─────────────────────────────────
    //
    // The common entry points return `Session`, not `Result<Session>` —
    // built-in profiles are statically valid (enforced by the
    // `profile_validation` tests), so the only way these fail is an
    // unrecoverable environment fault (e.g. a corrupt OS trust store), which
    // they surface as a panic exactly like `reqwest::Client::new()`. For a
    // configuration that can genuinely fail (custom trust roots, etc.) use
    // the fallible [`Session::builder`]`.build()`.

    /// A default **bare** session — no browser impersonation, a plain
    /// `leyline/<version>` client on the host OS. Mirrors the
    /// `Client::new()` shape. Opt into a browser with [`Session::chrome`]
    /// or `Session::builder().browser(...)`.
    pub fn new() -> Self {
        Self::builder()
            .build()
            .expect("bare session profile is always valid")
    }

    /// The latest bundled Chrome profile (currently Chrome 148 on Windows).
    /// Bumps silently when a new Chrome profile is added — pin
    /// [`Browser::Chrome147`] via the builder for a fixed version.
    ///
    /// This does not enable fingerprint auditing — `resp.audit()` returns
    /// `None`. For JA4/H2 introspection, build via
    /// `Session::builder().chrome().audit(true).build()` instead.
    pub fn chrome() -> Self {
        Self::builder()
            .browser(Browser::default_browser())
            .build()
            .expect("built-in Chrome profile is always valid")
    }

    /// The latest bundled Firefox profile (currently Firefox 150 on Windows).
    pub fn firefox() -> Self {
        Self::builder()
            .browser(Browser::Firefox150)
            .build()
            .expect("built-in Firefox profile is always valid")
    }

    /// The latest bundled Safari profile (currently Safari 18 on macOS).
    pub fn safari() -> Self {
        Self::builder()
            .browser(Browser::Safari18)
            .platform(Platform::MacOS)
            .build()
            .expect("built-in Safari profile is always valid")
    }

    /// Microsoft Edge on the latest Chromium profile we have a verified
    /// overlay for (currently Chrome 147). Pinned to the verified sibling
    /// anchor rather than the global Chrome default (148).
    pub fn edge() -> Self {
        Self::builder()
            .browser(Browser::Chrome147)
            .brand(ChromiumBrand::Edge)
            .build()
            .expect("built-in Edge overlay is always valid")
    }

    /// Brave on the latest verified Chromium profile (currently Brave 146).
    pub fn brave() -> Self {
        Self::builder()
            .browser(Browser::Brave146)
            .platform(Platform::MacOS)
            .build()
            .expect("built-in Brave profile is always valid")
    }

    /// Opera on the latest verified Chromium overlay (Chrome 147 / Opera 131).
    pub fn opera() -> Self {
        Self::builder()
            .browser(Browser::Chrome147)
            .brand(ChromiumBrand::Opera)
            .build()
            .expect("built-in Opera overlay is always valid")
    }

    /// Vivaldi on the latest verified Chromium overlay (Chrome 147 / Vivaldi 7.9).
    pub fn vivaldi() -> Self {
        Self::builder()
            .browser(Browser::Chrome147)
            .brand(ChromiumBrand::Vivaldi)
            .build()
            .expect("built-in Vivaldi overlay is always valid")
    }

    /// A session for an explicit browser and platform in one call.
    pub fn profile(browser: Browser, platform: Platform) -> Self {
        Self::builder()
            .browser(browser)
            .platform(platform)
            .build()
            .expect("built-in profile is always valid")
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
        // `make_mut` clones the inner state once (this Arc is shared), then
        // mutates the unique copy — the original session is untouched. This
        // is a config-time derive, not the per-request hot path.
        std::sync::Arc::make_mut(&mut s.inner).cookie_jar = cookie_jar;
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
        let inner = std::sync::Arc::make_mut(&mut s.inner);
        inner.proxy = Some(proxy_url.to_string());
        inner.proxy_config = inner.proxy_config.clone().set_default_proxy(proxy_url);
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
        let inner = std::sync::Arc::make_mut(&mut s.inner);
        inner.max_redirects = n;
        inner.redirect_policy = crate::core::RedirectPolicy::limited(n);
        s
    }

    /// The impersonated browser, or `None` for a bare (non-impersonating)
    /// session — the default when no `.browser(...)` was set.
    pub fn browser(&self) -> Option<Browser> {
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

    /// The session-wide default retry policy, inherited by every request that
    /// does not override it via [`crate::RequestBuilder::retry`].
    pub(crate) fn default_retry(&self) -> &crate::core::retry::RetryPolicy {
        &self.default_retry
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
    pub fn request(&self, method: &str, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, method, url)
    }

    /// Start a GET request.
    ///
    /// ```rust,ignore
    /// let resp = session.get(url).send().await?;
    /// ```
    pub fn get(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, "GET", url)
    }

    /// Start a POST request.
    pub fn post(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, "POST", url)
    }

    /// Start a PUT request.
    pub fn put(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, "PUT", url)
    }

    /// Start a PATCH request.
    pub fn patch(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, "PATCH", url)
    }

    /// Start a DELETE request.
    pub fn delete(&self, url: &str) -> RequestBuilder {
        RequestBuilder::new(self, "DELETE", url)
    }

    /// Start a HEAD request.
    pub fn head(&self, url: &str) -> RequestBuilder {
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

    /// GET a sub-resource with the Script preset applied — the request
    /// looks like a browser `<script src=…>` / stylesheet load
    /// (Sec-Fetch-Dest: script, no-cors). Returns a fully-executed
    /// `Response`; use [`get`](Self::get) + `.preset(Preset::Script)` if
    /// you need to chain extra headers first.
    ///
    /// ```rust,ignore
    /// let js = session.get_script(url).await?.text();
    /// ```
    pub async fn get_script(&self, url: &str) -> Result<Response> {
        self.get(url).preset(Preset::Script).send().await
    }

    /// GET with the XHR preset applied — the request looks like a
    /// browser `fetch()` / `XMLHttpRequest` (Sec-Fetch-Mode: cors,
    /// Sec-Fetch-Dest: empty). Returns a fully-executed `Response`; use
    /// [`get`](Self::get) + `.preset(Preset::Xhr)` to chain headers first.
    pub async fn get_xhr(&self, url: &str) -> Result<Response> {
        self.get(url).preset(Preset::Xhr).send().await
    }

    /// POST a JSON body with the XHR preset applied.
    pub async fn post_json(&self, url: &str, body: &impl serde::Serialize) -> Result<Response> {
        self.post(url).preset(Preset::Xhr).json(body).send().await
    }

    /// POST a raw body with the XHR preset applied — the non-JSON
    /// `fetch()`/`XMLHttpRequest` case (e.g. a `text/plain`
    /// payload). Returns a fully-executed `Response`; use
    /// [`post`](Self::post) + `.preset(Preset::Xhr)` to chain a custom
    /// content-type / referer before sending.
    pub async fn post_xhr(&self, url: &str, body: impl Into<Body>) -> Result<Response> {
        self.post(url).preset(Preset::Xhr).body(body).send().await
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

impl Default for Session {
    /// The default session is **bare** — see [`Session::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("browser", &self.browser)
            .field("platform", &self.platform)
            .field("proxy", &self.proxy)
            .field("timeout", &self.timeouts.total)
            .field("max_redirects", &self.max_redirects)
            .field("protocol_policy", &self.protocol_policy)
            .field("ja4", &self.audit_tls.ja4)
            .finish()
    }
}

impl std::fmt::Display for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.browser {
            Some(b) => write!(
                f,
                "Session({}, {}, proxy={})",
                b,
                self.platform,
                self.proxy.as_deref().unwrap_or("none")
            ),
            None => write!(
                f,
                "Session(bare, {}, proxy={})",
                self.platform,
                self.proxy.as_deref().unwrap_or("none")
            ),
        }
    }
}
