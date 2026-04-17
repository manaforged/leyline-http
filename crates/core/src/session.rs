//! Session — the primary Leyline client.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, LazyLock};

use leyline_cookies::CookieJar;
use leyline_h2::H2Config;
use leyline_pool::Pool;
use leyline_profile::{Browser, Platform, Preset, ProfileRegistry};
use leyline_tcp::TcpProfile;
use leyline_tls::FingerprintConnector;

static PROFILES: LazyLock<ProfileRegistry> = LazyLock::new(ProfileRegistry::builtin);

use crate::body::Body;
use crate::error::{Error, Result};
use crate::headers::HeaderList;
use crate::request::RequestBuilder;
use crate::response::Response;

/// Protocol selection policy for requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolPolicy {
    /// Use H1 for `http://`, H2 for `https://`, and fall back to H1 when ALPN
    /// does not negotiate H2.
    Auto,
    /// Force HTTP/1.1. Uses plaintext H1 for `http://` and TLS H1 for `https://`.
    Http1,
    /// Force HTTP/2 over TLS.
    Http2,
    /// Force HTTP/3 over QUIC.
    Http3,
    /// Prefer H3 and fall back to H2/H1. This is currently sequential, not a
    /// true parallel Chrome-style race.
    Race,
}

/// Session builder — configure browser, platform, proxy, timeout, cookies.
pub struct SessionBuilder {
    browser: Browser,
    platform: Platform,
    proxy: Option<String>,
    timeout: std::time::Duration,
    max_redirects: usize,
    cookie_jar: Option<CookieJar>,
    tcp_profile: Option<TcpProfile>,
    protocol_policy: ProtocolPolicy,
    grease_seed: Option<Vec<u8>>,
    accept_invalid_certs: bool,
}

impl SessionBuilder {
    fn new() -> Self {
        Self {
            browser: Browser::default(),
            platform: Platform::default(),
            proxy: None,
            timeout: std::time::Duration::from_secs(30),
            max_redirects: 10,
            cookie_jar: None,
            tcp_profile: None,
            protocol_policy: ProtocolPolicy::Auto,
            grease_seed: None,
            accept_invalid_certs: false,
        }
    }

    /// Set a proxy URL (http:// with CONNECT tunnel).
    pub fn proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }

    /// Set request timeout (default: 30s).
    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the browser TLS profile.
    pub fn browser(mut self, browser: Browser) -> Self {
        self.browser = browser;
        self
    }

    /// Set the target platform.
    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self
    }

    /// Set maximum number of redirects to follow.
    pub fn max_redirects(mut self, n: usize) -> Self {
        self.max_redirects = n;
        self
    }

    /// Provide a pre-populated cookie jar.
    pub fn cookie_jar(mut self, jar: CookieJar) -> Self {
        self.cookie_jar = Some(jar);
        self
    }

    /// Set a custom TCP fingerprint profile.
    pub fn tcp_profile(mut self, profile: TcpProfile) -> Self {
        self.tcp_profile = Some(profile);
        self
    }

    /// Force HTTP/3 over QUIC.
    ///
    /// ```rust,ignore
    /// let session = Session::builder()
    ///     .browser(Browser::Chrome147)
    ///     .http3()
    ///     .build()?;
    /// ```
    pub fn http3(mut self) -> Self {
        self.protocol_policy = ProtocolPolicy::Http3;
        self
    }

    /// Deprecated: use [`http3`](Self::http3) instead — kept for source
    /// compatibility with 2.0.0-alpha.1.
    #[deprecated(
        since = "2.0.0-alpha.2",
        note = "use `.http3()` for consistency with `.http1()` / `.http2()`"
    )]
    pub fn h3(self) -> Self {
        self.http3()
    }

    /// Force HTTP/1.1.
    pub fn http1(mut self) -> Self {
        self.protocol_policy = ProtocolPolicy::Http1;
        self
    }

    /// Force HTTP/2.
    pub fn http2(mut self) -> Self {
        self.protocol_policy = ProtocolPolicy::Http2;
        self
    }

    /// Prefer HTTP/3 and fall back to HTTP/2/HTTP/1.1.
    ///
    /// This is a sequential compatibility policy today. It reserves the API
    /// shape for a future true parallel H2/H3 race.
    pub fn race(mut self) -> Self {
        self.protocol_policy = ProtocolPolicy::Race;
        self
    }

    /// Set the protocol selection policy.
    pub fn protocol_policy(mut self, policy: ProtocolPolicy) -> Self {
        self.protocol_policy = policy;
        self
    }

    /// Set a deterministic GREASE seed. Ensures the same GREASE values
    /// are used across connections, creating a stable fingerprint per
    /// identity. Without this, GREASE is random per connection.
    ///
    /// ```rust,ignore
    /// let session = Session::builder()
    ///     .grease_seed(b"user_42")
    ///     .build()?;
    /// ```
    pub fn grease_seed(mut self, seed: impl Into<Vec<u8>>) -> Self {
        self.grease_seed = Some(seed.into());
        self
    }

    /// Disable peer certificate verification. **Dangerous** — any
    /// man-in-the-middle between the client and the target can serve
    /// arbitrary content without detection. Intended only for the
    /// `leyline` CLI's `-k/--insecure` flag and controlled test
    /// fixtures against self-signed local servers.
    ///
    /// The method is named with a `danger_` prefix so it is grep-able
    /// in audit reviews — if you see this called in production code,
    /// that is itself a finding.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Build the session.
    pub fn build(self) -> Result<Session> {
        // HTTP/3 forbids proxies today — fail fast at build time instead of
        // deferring to first request.
        if self.proxy.is_some() && matches!(self.protocol_policy, ProtocolPolicy::Http3) {
            return Err(Error::Config(
                "HTTP/3 over proxies is not implemented; drop `.http3()` or `.proxy(...)`".into(),
            ));
        }

        let profile = PROFILES
            .get_browser(self.browser)
            .ok_or_else(|| Error::Config(format!("no profile for {}", self.browser)))?;

        let identity_key = self.platform.identity_key();
        let identity = profile
            .identity_for(identity_key)
            .ok_or_else(|| {
                Error::Config(format!("no {} identity for {}", identity_key, self.browser))
            })?
            .clone();

        let tcp_profile = self
            .tcp_profile
            .unwrap_or_else(|| self.platform.tcp_profile());

        let cookie_jar = self.cookie_jar.unwrap_or_default();

        // Build TLS connector from profile.
        let mut connector =
            FingerprintConnector::new(profile, tcp_profile, self.grease_seed.as_deref())
                .map_err(Error::Tls)?;
        if self.accept_invalid_certs {
            connector.set_accept_invalid_certs(true);
        }

        // Build H2 config from profile.
        let h2_config = H2Config::from_profile(&profile.h2);

        // Pre-compute audit data from profile.
        let extension_ids = leyline_audit::chrome_extension_ids(&profile.tls);
        let ja4 = {
            let input = leyline_audit::Ja4Input {
                ciphers: &profile.tls.ciphers,
                sigalgs: &profile.tls.sigalgs,
                curves: &profile.tls.curves,
                extension_ids: &extension_ids,
                tls_version: "1.3",
                has_sni: true,
                alpn: "h2",
            };
            leyline_audit::compute_ja4(&input)
        };
        let ja3 = {
            let input = leyline_audit::Ja3Input {
                ciphers: &profile.tls.ciphers,
                curves: &profile.tls.curves,
                extension_ids: &extension_ids,
                tls_record_version: 771, // TLS 1.2 record layer
            };
            leyline_audit::compute_ja3(&input)
        };
        let h2_fp = h2_config.akamai_fingerprint();
        let is_windows = self.platform == Platform::Windows;
        let ja4t = leyline_audit::compute_ja4t(
            tcp_profile.window_size,
            tcp_profile.mss as u16,
            tcp_profile.window_scale as u8,
            is_windows,
        );

        Ok(Session {
            browser: self.browser,
            platform: self.platform,
            user_agent: identity.user_agent,
            sec_ch_ua: identity.sec_ch_ua,
            accept_language: identity
                .accept_language
                .unwrap_or_else(|| "en-US,en;q=0.9".to_string()),
            proxy: self.proxy,
            timeout: self.timeout,
            max_redirects: self.max_redirects,
            cookie_jar,
            connector,
            h2_config,
            pool: Arc::new(Pool::new()),
            audit_tls: AuditTlsCache {
                ja4,
                ja3,
                h2_fingerprint: h2_fp,
                ja4t,
            },
            protocol_policy: self.protocol_policy,
            h3_config: match profile.meta.family.as_str() {
                "chromium" => leyline_quic::H3Config::chrome(),
                "firefox" => leyline_quic::H3Config::firefox(),
                "safari" | "webkit" => leyline_quic::H3Config::safari(),
                _ => leyline_quic::H3Config::chrome(),
            },
            profile,
        })
    }
}

/// A Leyline session — browser-fingerprinted HTTP client with cookies.
pub struct Session {
    browser: Browser,
    platform: Platform,
    user_agent: String,
    sec_ch_ua: String,
    accept_language: String,
    proxy: Option<String>,
    timeout: std::time::Duration,
    max_redirects: usize,
    cookie_jar: CookieJar,
    connector: FingerprintConnector,
    h2_config: H2Config,
    pool: Arc<Pool>,
    /// Cached audit data computed from the profile (TLS + TCP parts).
    audit_tls: AuditTlsCache,
    /// Request protocol selection policy.
    protocol_policy: ProtocolPolicy,
    /// H3 config (transport params + QPACK + SETTINGS).
    h3_config: leyline_quic::H3Config,
    /// Reference to the static browser profile — passed through to the H3
    /// path so QUIC ClientHello is built from the same factory as H2.
    profile: &'static leyline_profile::BrowserProfile,
}

/// Pre-computed TLS/TCP audit data from the profile.
#[derive(Debug)]
struct AuditTlsCache {
    ja4: String,
    ja3: String,
    h2_fingerprint: String,
    ja4t: String,
}

impl Session {
    /// Create a new session builder.
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    /// Shortcut to the latest bundled Chrome profile (currently Chrome 147
    /// on Windows). Bumps silently when a new Chrome profile is added —
    /// pin [`Browser::Chrome147`] via the builder if you need a specific
    /// version across releases.
    pub fn chrome_latest() -> Result<Self> {
        Self::builder().build()
    }

    /// Shortcut to the latest bundled Firefox profile (currently Firefox
    /// 148 on Windows). Bumps silently on new releases — pin via the
    /// builder for stability.
    pub fn firefox_latest() -> Result<Self> {
        Self::builder().browser(Browser::Firefox148).build()
    }

    /// Shortcut to the latest bundled Safari profile (currently Safari
    /// 18 on macOS). Bumps silently on new releases — pin via the
    /// builder for stability.
    pub fn safari_latest() -> Result<Self> {
        Self::builder()
            .browser(Browser::Safari18)
            .platform(Platform::MacOS)
            .build()
    }

    /// Deprecated alias for [`Session::chrome_latest`].
    #[deprecated(
        since = "2.0.0-alpha.1",
        note = "use `Session::chrome_latest` (renamed to make silent-upgrade behaviour obvious) or pin a specific `Browser::ChromeN` via `Session::builder`."
    )]
    pub fn chrome() -> Result<Self> {
        Self::chrome_latest()
    }

    /// Deprecated alias for [`Session::firefox_latest`].
    #[deprecated(
        since = "2.0.0-alpha.1",
        note = "use `Session::firefox_latest` or pin a specific `Browser::FirefoxN` via `Session::builder`."
    )]
    pub fn firefox() -> Result<Self> {
        Self::firefox_latest()
    }

    /// Deprecated alias for [`Session::safari_latest`].
    #[deprecated(
        since = "2.0.0-alpha.1",
        note = "use `Session::safari_latest` or pin a specific `Browser::SafariN` via `Session::builder`."
    )]
    pub fn safari() -> Result<Self> {
        Self::safari_latest()
    }

    /// Access the cookie jar.
    pub fn cookies(&self) -> &CookieJar {
        &self.cookie_jar
    }

    /// The browser profile in use.
    pub fn browser(&self) -> Browser {
        self.browser
    }

    /// The platform in use.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    // ─── Fluent request builders ───────────────────────────────────
    //
    // Every verb returns a `RequestBuilder` that you chain on and end with
    // `.send().await`. This is the one and only way to issue a request —
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

    // ─── Convenience shortcuts ─────────────────────────────────────

    /// GET with the Navigate preset applied — the request looks like a
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

    // ─── WebSocket ──────────────────────────────────────────────────

    /// Connect to a WebSocket URL with TLS fingerprinting.
    ///
    /// Path selection is two-tier:
    ///
    /// 1. **HTTP/2 extended CONNECT** (RFC 8441) is attempted first.
    ///    It piggybacks on whatever pooled H2 connection the session
    ///    already maintains to the destination (or opens a fresh one
    ///    with the standard ALPN list). If the peer advertises
    ///    `SETTINGS_ENABLE_CONNECT_PROTOCOL = 1` the WebSocket runs as
    ///    a bidirectional stream inside the existing H2 connection,
    ///    sharing its TLS session and TCP connection.
    /// 2. **HTTP/1.1 Upgrade** is used as a fallback when either the
    ///    peer doesn't advertise extended CONNECT, the ALPN doesn't
    ///    negotiate `h2`, or the H2 attempt errors. A fresh TLS
    ///    connection with `http/1.1` ALPN is negotiated and
    ///    tokio-tungstenite drives the classic RFC 6455 handshake.
    ///
    /// In both cases the returned [`WsConnection`](
    /// crate::websocket::WsConnection) looks identical to the caller.
    /// Use [`websocket_http1`](Self::websocket_http1) to force the
    /// legacy H1 path — useful for testing servers that speak both.
    ///
    /// ```rust,ignore
    /// let mut ws = session.websocket("wss://echo.example.com/ws").await?;
    /// ws.send("hello").await?;
    /// if let Some(msg) = ws.recv().await? {
    ///     println!("{}", msg);
    /// }
    /// ws.close().await?;
    /// ```
    pub async fn websocket(&self, url: &str) -> Result<crate::websocket::WsConnection> {
        let origin = ws_origin(url)?;

        // Try H2 first. The pool helper handles the TLS handshake +
        // ALPN check; if the connection already existed we just clone
        // its handle. Failures that look like "peer didn't enable
        // CONNECT protocol" fall through to the H1 upgrade path.
        match crate::websocket::WsConnection::connect_h2(
            &self.pool,
            &self.connector,
            &self.h2_config,
            url,
            self.proxy.as_deref(),
            &self.user_agent,
            &origin,
        )
        .await
        {
            Ok(conn) => return Ok(conn),
            Err(e) if crate::websocket::WsConnection::is_h2_fallback_trigger(&e) => {
                tracing::debug!(
                    error = %e,
                    "H2 extended CONNECT not available, falling back to H1 Upgrade"
                );
            }
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "H2 WebSocket path failed, falling back to H1 Upgrade"
                );
            }
        }

        self.websocket_http1(url).await
    }

    /// Force the HTTP/1.1 Upgrade WebSocket path, skipping the
    /// HTTP/2 extended CONNECT probe. Useful against servers that
    /// speak both protocols but whose H2 WebSocket implementation is
    /// known-broken, or for deterministic test setups.
    pub async fn websocket_http1(&self, url: &str) -> Result<crate::websocket::WsConnection> {
        let origin = ws_origin(url)?;
        crate::websocket::WsConnection::connect_h1(
            &self.connector,
            url,
            self.proxy.as_deref(),
            &self.user_agent,
            &origin,
        )
        .await
    }

    /// Dispatch a standalone [`crate::Request`] value. Used by
    /// adapters that can't borrow the session (e.g. the `tower::Service`
    /// implementation in `leyline-tower`).
    ///
    /// For most code, prefer the fluent [`Self::get`] / [`Self::post`]
    /// / [`Self::request`] builders — they're borrow-cheap and avoid
    /// an extra allocation of the `Request` value.
    pub async fn execute_request(&self, req: crate::Request) -> Result<Response> {
        let headers = if req.headers.is_empty() {
            None
        } else {
            Some(req.headers)
        };
        self.execute_with_timeout(
            &req.method, &req.url, None, req.body, headers, req.timeout, false,
        )
        .await
    }

    // ─── Core execution ─────────────────────────────────────────────

    /// Execute a request with an optional per-request timeout override.
    /// When `override_timeout` is `None`, the session-level timeout applies.
    /// Handles redirects, decompression, cookies.
    pub(crate) async fn execute_with_timeout(
        &self,
        method: &str,
        raw_url: &str,
        preset: Option<Preset>,
        body: Body,
        extra_headers: Option<HeaderList>,
        override_timeout: Option<std::time::Duration>,
        stream_response: bool,
    ) -> Result<Response> {
        let timeout = override_timeout.unwrap_or(self.timeout);
        // Box the inner future to move its state to the heap. Without this,
        // the combined RequestBuilder → execute_with_timeout → execute_inner
        // state machine is large enough to blow the default 2 MB thread
        // stack when a test awaits two requests sequentially.
        let inner: std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Response>> + Send + '_>,
        > = Box::pin(self.execute_inner(
            method,
            raw_url,
            preset,
            body,
            extra_headers,
            stream_response,
        ));
        match tokio::time::timeout(timeout, inner).await {
            Ok(result) => result,
            Err(_) => Err(Error::Timeout),
        }
    }

    #[tracing::instrument(
        name = "session.execute",
        level = "debug",
        skip_all,
        fields(http.method = method, http.url = raw_url)
    )]
    async fn execute_inner(
        &self,
        method: &str,
        raw_url: &str,
        preset: Option<Preset>,
        body: Body,
        extra_headers: Option<HeaderList>,
        stream_response: bool,
    ) -> Result<Response> {
        let mut current_url = url::Url::parse(raw_url)?;
        let original_origin = url_origin(&current_url);
        let mut current_method = method.to_string();
        // Carry the body through the redirect loop. A streaming body is
        // placed in `current_body` for the first hop; on a method- or
        // body-preserving redirect (307/308) we cannot replay a stream,
        // so a cross-redirect stream becomes an explicit error. Buffered
        // bodies replay fine because `Body::Bytes` is `Clone`-like.
        let mut current_body = body;
        let mut redirect_chain = Vec::new();
        let mut all_cookies = HashMap::new();

        for _ in 0..=self.max_redirects {
            let origin = url_origin(&current_url);
            let referer = if redirect_chain.is_empty() {
                format!("{}/", origin)
            } else {
                redirect_chain
                    .last()
                    .cloned()
                    .unwrap_or_else(|| format!("{}/", origin))
            };

            // Build headers.
            let mut headers: Vec<(String, String)> = if let Some(preset) = preset {
                let ctx = leyline_profile::preset::HeaderContext {
                    user_agent: &self.user_agent,
                    sec_ch_ua: &self.sec_ch_ua,
                    sec_ch_ua_mobile: self.platform.mobile_flag(),
                    sec_ch_ua_platform: self.platform.sec_ch_platform(),
                    accept_language: &self.accept_language,
                    origin: &origin,
                    referer: &referer,
                };
                preset.build_headers(&ctx)
            } else {
                vec![
                    ("user-agent".into(), self.user_agent.clone()),
                    ("accept".into(), "*/*".to_string()),
                    (
                        "accept-encoding".into(),
                        "gzip, deflate, br, zstd".to_string(),
                    ),
                    ("accept-language".into(), self.accept_language.clone()),
                ]
            };

            // Extra headers: include on first request, and on same-origin redirects.
            // Strip sensitive headers on cross-origin redirects.
            if let Some(ref extra) = extra_headers {
                let same_origin = origin == original_origin;
                for (k, v) in extra.iter() {
                    if !redirect_chain.is_empty() && !same_origin {
                        // Cross-origin redirect — strip sensitive headers.
                        let lower = k.to_lowercase();
                        if lower == "authorization"
                            || lower == "proxy-authorization"
                            || lower == "cookie"
                        {
                            continue;
                        }
                    }
                    headers.push((k.clone(), v.clone()));
                }
            }

            // Content-Length for requests with a known-length body.
            // For length-unknown streams we leave it out and let the
            // transport pick `Transfer-Encoding: chunked` (H1) or native
            // framing (H2/H3).
            if let Some(len) = current_body.len_hint() {
                if !matches!(current_body, Body::Empty) || len > 0 {
                    headers.push(("content-length".into(), len.to_string()));
                }
            }

            // Cookies.
            if let Some(cookie_val) = self.cookie_jar.cookie_header(&current_url) {
                headers.push(("cookie".into(), cookie_val));
            }

            let audit_headers = headers.clone();

            // Take the body for this hop. Streams are one-shot; we replace
            // `current_body` with `Body::Empty` so a follow-up redirect
            // sees there's nothing to replay (and fails loudly).
            let hop_body = std::mem::take(&mut current_body);
            let hop_body_was_stream = hop_body.is_stream();

            // Send via the configured protocol policy.
            let transport_resp = self
                .send_with_policy(
                    &current_method,
                    &current_url,
                    headers,
                    hop_body,
                    stream_response,
                )
                .await?;
            let status = transport_resp.status;
            let resp_headers = transport_resp.headers;
            let resp_body_shape = transport_resp.body;
            let final_url = transport_resp.final_url;
            let response_version = transport_resp.version;
            let tls_alpn = transport_resp.tls_alpn;
            let peer_cert_der = transport_resp.peer_cert_der;
            let tls_version = transport_resp.tls_version;
            let tls_cipher = transport_resp.tls_cipher;

            // Store cookies from response and accumulate across redirect chain.
            let set_cookies: Vec<&str> = resp_headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                .map(|(_, v)| v.as_str())
                .collect();
            if !set_cookies.is_empty() {
                self.cookie_jar
                    .store_response_cookies(&set_cookies, &current_url);
                for sc in &set_cookies {
                    if let Some(eq) = sc.find('=') {
                        let name = sc[..eq].trim();
                        let rest = &sc[eq + 1..];
                        let value = rest.split(';').next().unwrap_or("").trim();
                        all_cookies.insert(name.to_string(), value.to_string());
                    }
                }
            }

            // Check for redirect.
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if let Some(location) = resp_headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                    .map(|(_, v)| v.clone())
                {
                    // Drain and discard the intermediate response body.
                    drop(resp_body_shape);
                    redirect_chain.push(current_url.to_string());
                    current_url = current_url.join(&location)?;

                    // 301/302/303: switch to GET, drop body.
                    // 307/308: preserve method and body. A streaming
                    // request body cannot be replayed — fail clearly.
                    if matches!(status, 301 | 302 | 303) {
                        current_method = "GET".to_string();
                        current_body = Body::Empty;
                    } else if hop_body_was_stream {
                        return Err(Error::Http(format!(
                            "cannot follow {status} redirect: streaming request bodies are \
                             not replayable. Either buffer the body before sending or set \
                             max_redirects(0)."
                        )));
                    }
                    continue;
                }
            }

            // If the caller opted into streaming, deliver as-is WITHOUT
            // decompression. Otherwise materialise and decompress as today.
            let (final_body, final_headers) = match resp_body_shape {
                crate::transport::TransportBody::Streaming(bs) if stream_response => (
                    crate::response::ResponseBody::Streaming(bs),
                    resp_headers,
                ),
                crate::transport::TransportBody::Streaming(bs) => {
                    // Transport returned a stream but caller wanted
                    // buffering. Drain it fully here, then run normal
                    // decompression.
                    let buf = drain_stream_into_vec(bs).await?;
                    let content_encoding = resp_headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
                        .map(|(_, v)| v.to_lowercase());
                    let buf = decompress_body(buf, content_encoding.as_deref())?;
                    let resp_headers: Vec<(String, String)> = if content_encoding.is_some() {
                        resp_headers
                            .into_iter()
                            .filter(|(k, _)| {
                                !k.eq_ignore_ascii_case("content-encoding")
                                    && !k.eq_ignore_ascii_case("content-length")
                            })
                            .collect()
                    } else {
                        resp_headers
                    };
                    (crate::response::ResponseBody::Buffered(buf), resp_headers)
                }
                crate::transport::TransportBody::Buffered(buf) => {
                    if stream_response {
                        // Transport buffered (H1 / H3 path). Preserve
                        // content-encoding and hand the buffer over as a
                        // single-chunk stream so the API is uniform.
                        (
                            crate::response::ResponseBody::Streaming(
                                crate::body_stream::BodyStream::from_bytes(bytes::Bytes::from(buf)),
                            ),
                            resp_headers,
                        )
                    } else {
                        let content_encoding = resp_headers
                            .iter()
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
                            .map(|(_, v)| v.to_lowercase());
                        let buf = decompress_body(buf, content_encoding.as_deref())?;
                        let resp_headers: Vec<(String, String)> = if content_encoding.is_some() {
                            resp_headers
                                .into_iter()
                                .filter(|(k, _)| {
                                    !k.eq_ignore_ascii_case("content-encoding")
                                        && !k.eq_ignore_ascii_case("content-length")
                                })
                                .collect()
                        } else {
                            resp_headers
                        };
                        (crate::response::ResponseBody::Buffered(buf), resp_headers)
                    }
                }
            };

            return Ok(Response {
                status,
                headers: final_headers,
                body: final_body,
                cookies: all_cookies,
                url: final_url,
                redirect_chain,
                version: response_version,
                trailers: Vec::new(),
                request_headers: audit_headers.clone(),
                tls_alpn,
                tls_peer_certificate: peer_cert_der,
                tls_version,
                tls_cipher,
                audit_data: Some(leyline_audit::AuditData {
                    ja4: self.audit_tls.ja4.clone(),
                    ja3: self.audit_tls.ja3.clone(),
                    h2_fingerprint: self.audit_tls.h2_fingerprint.clone(),
                    ja4t: self.audit_tls.ja4t.clone(),
                    ja4h: {
                        let input = leyline_audit::Ja4hInput {
                            method: &current_method,
                            http_version: response_version.ja4h_token(),
                            headers: &audit_headers,
                        };
                        leyline_audit::compute_ja4h(&input)
                    },
                }),
            });
        }

        Err(Error::Http(format!(
            "too many redirects (max {})",
            self.max_redirects
        )))
    }

    async fn send_with_policy(
        &self,
        method: &str,
        url: &url::Url,
        headers: Vec<(String, String)>,
        body: Body,
        stream_response: bool,
    ) -> Result<crate::transport::TransportResponse> {
        match self.protocol_policy {
            ProtocolPolicy::Auto => {
                crate::transport::send_request_auto(
                    &self.pool,
                    &self.connector,
                    &self.h2_config,
                    method,
                    url,
                    headers,
                    body,
                    self.proxy.as_deref(),
                    stream_response,
                )
                .await
            }
            ProtocolPolicy::Http1 => {
                crate::transport::send_request_h1(
                    &self.connector,
                    method,
                    url,
                    headers,
                    body,
                    self.proxy.as_deref(),
                    stream_response,
                )
                .await
            }
            ProtocolPolicy::Http2 => {
                crate::transport::send_request_h2(
                    &self.pool,
                    &self.connector,
                    &self.h2_config,
                    method,
                    url,
                    headers,
                    body,
                    self.proxy.as_deref(),
                    stream_response,
                )
                .await
            }
            ProtocolPolicy::Http3 => {
                if self.proxy.is_some() {
                    return Err(Error::Config(
                        "HTTP/3 over proxies is not implemented; use Auto or Http2".into(),
                    ));
                }
                crate::transport::send_request_h3(
                    &self.h3_config,
                    self.profile,
                    method,
                    url,
                    headers,
                    body,
                    stream_response,
                )
                .await
            }
            ProtocolPolicy::Race => {
                // Race doesn't interact well with streaming bodies — we
                // can only try H3 first if we have a buffered body to
                // keep for the fallback. Streaming bodies run straight
                // through the Auto path.
                if body.is_stream() || stream_response {
                    return crate::transport::send_request_auto(
                        &self.pool,
                        &self.connector,
                        &self.h2_config,
                        method,
                        url,
                        headers,
                        body,
                        self.proxy.as_deref(),
                        stream_response,
                    )
                    .await;
                }
                if self.proxy.is_none() && url.scheme() == "https" {
                    // We have a buffered body — clone for the retry.
                    let retained = match &body {
                        Body::Empty => Body::Empty,
                        Body::Bytes(b) => Body::Bytes(b.clone()),
                        Body::Stream { .. } => unreachable!(),
                    };
                    match crate::transport::send_request_h3(
                        &self.h3_config,
                        self.profile,
                        method,
                        url,
                        headers.clone(),
                        retained,
                        stream_response,
                    )
                    .await
                    {
                        Ok(resp) => return Ok(resp),
                        Err(_) => {}
                    }
                }
                crate::transport::send_request_auto(
                    &self.pool,
                    &self.connector,
                    &self.h2_config,
                    method,
                    url,
                    headers,
                    body,
                    self.proxy.as_deref(),
                    stream_response,
                )
                .await
            }
        }
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

/// Build a WebSocket `Origin` header value from a `ws://` or `wss://`
/// URL by mapping the scheme to `http`/`https`. Used by the WebSocket
/// entry points to keep Origin consistent between H1 and H2 paths.
fn ws_origin(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url)?;
    let scheme = match parsed.scheme() {
        "wss" => "https",
        "ws" => "http",
        other => other,
    };
    Ok(format!("{}://{}", scheme, parsed.host_str().unwrap_or("")))
}

/// Extract origin (scheme://host:port) from a URL for same-origin comparison.
fn url_origin(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or("");
    match url.port() {
        Some(port) => format!("{}://{}:{}", url.scheme(), host, port),
        None => format!("{}://{}", url.scheme(), host),
    }
}

/// Decompress response body based on content-encoding header.
/// Handles multi-encoding (e.g., "gzip, br") by applying in reverse order.
fn decompress_body(body: Vec<u8>, encoding: Option<&str>) -> Result<Vec<u8>> {
    let encoding = match encoding {
        Some(e) => e,
        None => return Ok(body),
    };

    // Split on comma for multi-encoding, apply in reverse order.
    // "gzip, br" means gzip was applied first and br second; decode br then gzip.
    let encodings: Vec<&str> = encoding.split(',').map(|s| s.trim()).collect();
    let mut data = body;

    for enc in encodings.iter().rev() {
        data = decompress_single(data, enc)?;
    }

    Ok(data)
}

/// Max decompressed body size (100 MB, same as wire limit).
const MAX_DECOMPRESSED: usize = 100 * 1024 * 1024;

async fn drain_stream_into_vec(mut bs: crate::body_stream::BodyStream) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    while let Some(chunk) = bs.next().await {
        let chunk = chunk.map_err(Error::Io)?;
        if out.len() + chunk.len() > MAX_DECOMPRESSED {
            return Err(Error::Http(format!(
                "response body exceeds {MAX_DECOMPRESSED} bytes"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn decompress_single(body: Vec<u8>, encoding: &str) -> Result<Vec<u8>> {
    match encoding {
        "gzip" | "x-gzip" => {
            let mut decoder = flate2::read::GzDecoder::new(&body[..]);
            read_limited(&mut decoder, "gzip")
        }
        "br" => {
            let mut decoder = brotli::Decompressor::new(&body[..], 4096);
            read_limited(&mut decoder, "brotli")
        }
        "zstd" => {
            let mut decoder =
                zstd::Decoder::new(&body[..]).map_err(|e| Error::Http(format!("zstd: {e}")))?;
            read_limited(&mut decoder, "zstd")
        }
        "deflate" => {
            // HTTP `Content-Encoding: deflate` is notoriously ambiguous: some
            // servers send raw DEFLATE, most (IIS, nginx, httpbin) send zlib-
            // wrapped DEFLATE. Real Chrome tries zlib first and falls back to
            // raw. Detect zlib by its magic byte (CMF): high nibble is the
            // compression method (8 = deflate), so 0x78 is the common CMF.
            let looks_like_zlib = body.first() == Some(&0x78);
            if looks_like_zlib {
                let mut decoder = flate2::read::ZlibDecoder::new(&body[..]);
                match read_limited(&mut decoder, "deflate") {
                    Ok(v) => return Ok(v),
                    Err(_) => {
                        // Fall through to raw DEFLATE.
                    }
                }
            }
            let mut decoder = flate2::read::DeflateDecoder::new(&body[..]);
            read_limited(&mut decoder, "deflate")
        }
        "identity" | "" => Ok(body),
        _ => Ok(body),
    }
}

/// Read from a decoder with a size limit (decompression bomb protection).
fn read_limited(reader: &mut impl Read, name: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| Error::Http(format!("{name}: {e}")))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() > MAX_DECOMPRESSED {
            return Err(Error::Http(format!(
                "{name}: decompressed size exceeds {MAX_DECOMPRESSED} bytes"
            )));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::HttpVersion;
    use std::io::Write;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn decompress_multi_encoding_in_reverse_order() {
        let body = b"browser-shaped bytes";

        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(body).unwrap();
        let gzip_body = gzip.finish().unwrap();

        let mut br = brotli::CompressorReader::new(&gzip_body[..], 4096, 5, 22);
        let mut encoded = Vec::new();
        br.read_to_end(&mut encoded).unwrap();

        let decoded = decompress_body(encoded, Some("gzip, br")).unwrap();
        assert_eq!(decoded, body);
    }

    #[tokio::test]
    async fn plaintext_http_uses_h1_and_preserves_duplicate_headers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut tmp = [0u8; 1024];
            loop {
                let n = socket.read(&mut tmp).await.unwrap();
                assert!(n > 0, "client closed before request headers");
                req.extend_from_slice(&tmp[..n]);
                if req.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }

            let text = String::from_utf8_lossy(&req);
            assert!(text.starts_with("GET /wire?q=1 HTTP/1.1\r\n"), "{text}");
            assert!(text.contains(&format!("\r\nHost: {addr}\r\n")), "{text}");
            assert!(text.contains("\r\nUser-Agent: "), "{text}");
            assert!(text.contains("\r\nAccept-Encoding: "), "{text}");
            assert!(text.contains("\r\nConnection: keep-alive\r\n"), "{text}");
            let first = text.find("x-dup: one").unwrap();
            let second = text.find("x-dup: two").unwrap();
            assert!(first < second, "{text}");

            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nset-cookie: a=1\r\nset-cookie: b=2\r\n\r\nok",
                )
                .await
                .unwrap();
        });

        let session = Session::chrome().unwrap();
        let resp = session
            .get(&format!("http://{addr}/wire?q=1"))
            .append_header("x-dup", "one")
            .append_header("x-dup", "two")
            .send()
            .await
            .unwrap();

        assert_eq!(resp.version(), HttpVersion::Http1_1);
        assert_eq!(resp.text(), "ok");
        assert_eq!(resp.header_all("set-cookie"), vec!["a=1", "b=2"]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn json_builder_returns_error_instead_of_panicking() {
        struct BadJson;

        impl serde::Serialize for BadJson {
            fn serialize<S>(&self, _serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("intentional test error"))
            }
        }

        let session = Session::chrome().unwrap();
        let err = session
            .post("http://127.0.0.1:9/no-network")
            .json(&BadJson)
            .send()
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Json(_)));
    }
}
