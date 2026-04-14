//! Session — the primary Leyline client.

use std::collections::HashMap;
use std::time::Duration;

use leyline_cookies::CookieJar;
use leyline_h2::H2Config;
use leyline_profile::{Browser, Platform, Preset, ProfileRegistry};
use leyline_tcp::TcpProfile;
use leyline_tls::FingerprintConnector;

use crate::error::{Error, Result};
use crate::request::RequestBuilder;
use crate::response::Response;

/// Session builder — configure browser, platform, proxy, timeout, etc.
pub struct SessionBuilder {
    browser: Browser,
    platform: Platform,
    proxy: Option<String>,
    timeout: Duration,
    max_redirects: usize,
    cookie_jar: Option<CookieJar>,
    tcp_profile: Option<TcpProfile>,
    grease_seed: Option<Vec<u8>>,
}

impl SessionBuilder {
    fn new() -> Self {
        Self {
            browser: Browser::default(),
            platform: Platform::default(),
            proxy: None,
            timeout: Duration::from_secs(30),
            max_redirects: 10,
            cookie_jar: None,
            tcp_profile: None,
            grease_seed: None,
        }
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

    /// Set a proxy URL (http://, https://, socks5://).
    pub fn proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }

    /// Set request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
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

    /// Set a GREASE seed for deterministic per-identity fingerprints.
    pub fn grease_seed(mut self, seed: &[u8]) -> Self {
        self.grease_seed = Some(seed.to_vec());
        self
    }

    /// Build the session.
    pub fn build(self) -> Result<Session> {
        let registry = ProfileRegistry::builtin();
        let profile = registry
            .get_browser(self.browser)
            .ok_or_else(|| Error::Config(format!("no profile for {}", self.browser)))?;

        let identity_key = self.platform.identity_key();
        let identity = profile
            .identity_for(identity_key)
            .ok_or_else(|| {
                Error::Config(format!(
                    "no {} identity for {}",
                    identity_key, self.browser
                ))
            })?
            .clone();

        let tcp_profile = self
            .tcp_profile
            .unwrap_or_else(|| self.platform.tcp_profile());

        let cookie_jar = self.cookie_jar.unwrap_or_default();

        // Build TLS connector from profile.
        let connector = FingerprintConnector::new(profile, tcp_profile)
            .map_err(|e| Error::Tls(e.to_string()))?;

        // Build H2 config from profile.
        let h2_config = H2Config::from_profile(&profile.h2);

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
            tcp_profile,
            grease_seed: self.grease_seed,
            connector,
            h2_config,
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
    timeout: Duration,
    max_redirects: usize,
    cookie_jar: CookieJar,
    tcp_profile: TcpProfile,
    grease_seed: Option<Vec<u8>>,
    connector: FingerprintConnector,
    h2_config: H2Config,
}

impl Session {
    /// Create a new session builder.
    pub fn builder() -> SessionBuilder {
        SessionBuilder::new()
    }

    /// Shortcut: latest Chrome, Windows.
    pub fn chrome() -> Result<Self> {
        Self::builder().build()
    }

    /// Shortcut: Firefox 148, Windows.
    pub fn firefox() -> Result<Self> {
        Self::builder().browser(Browser::Firefox148).build()
    }

    /// Shortcut: Safari 18, macOS.
    pub fn safari() -> Result<Self> {
        Self::builder()
            .browser(Browser::Safari18)
            .platform(Platform::MacOS)
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

    /// The platform in use.
    pub fn platform(&self) -> Platform {
        self.platform
    }

    /// The TCP fingerprint profile.
    pub fn tcp_profile(&self) -> &TcpProfile {
        &self.tcp_profile
    }

    // ─── Convenience methods ────────────────────────────────────────

    /// GET with Navigate preset — document fetch.
    pub async fn navigate(&self, url: &str) -> Result<Response> {
        self.execute("GET", url, Some(Preset::Navigate), None, None)
            .await
    }

    /// POST with XHR preset — JSON body.
    pub async fn post_json(
        &self,
        url: &str,
        body: &impl serde::Serialize,
    ) -> Result<Response> {
        let json = serde_json::to_vec(body)?;
        self.execute("POST", url, Some(Preset::Xhr), Some(json), None)
            .await
    }

    /// POST with Form preset — URL-encoded body.
    pub async fn post_form(&self, url: &str, data: &str) -> Result<Response> {
        self.execute(
            "POST",
            url,
            Some(Preset::Form),
            Some(data.as_bytes().to_vec()),
            None,
        )
        .await
    }

    /// Raw GET with no preset.
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.execute("GET", url, None, None, None).await
    }

    /// Create a fluent request builder.
    pub fn request(&self, method: &str, url: &str) -> RequestBuilder<'_> {
        RequestBuilder::new(self, method, url)
    }

    // ─── Core execution ─────────────────────────────────────────────

    /// Execute a request. This is the main entry point.
    pub(crate) async fn execute(
        &self,
        method: &str,
        raw_url: &str,
        preset: Option<Preset>,
        body: Option<Vec<u8>>,
        extra_headers: Option<HashMap<String, String>>,
    ) -> Result<Response> {
        let parsed = url::Url::parse(raw_url)?;
        let origin = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
        let referer = format!("{}/", origin);

        // Build ordered headers from preset or minimal defaults.
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
                ("accept-encoding".into(), "gzip, deflate, br, zstd".to_string()),
                ("accept-language".into(), self.accept_language.clone()),
            ]
        };

        // Merge extra headers.
        if let Some(extra) = extra_headers {
            for (k, v) in extra {
                headers.push((k, v));
            }
        }

        // Attach cookies.
        if let Some(cookie_val) = self.cookie_jar.cookie_header(&parsed) {
            headers.push(("cookie".into(), cookie_val));
        }

        // Send request via transport.
        let (status, resp_headers, resp_body, final_url) = crate::transport::send_request(
            &self.connector,
            &self.h2_config,
            method,
            &parsed,
            headers,
            body,
        )
        .await?;

        // Store response cookies.
        let set_cookies: Vec<&str> = resp_headers
            .iter()
            .filter(|(k, _)| k.to_lowercase() == "set-cookie")
            .map(|(_, v)| v.as_str())
            .collect();
        if !set_cookies.is_empty() {
            self.cookie_jar
                .store_response_cookies(&set_cookies, &parsed);
        }

        // Build cookies map from jar.
        let cookies = HashMap::new(); // TODO: populate from jar

        Ok(Response {
            status,
            headers: resp_headers,
            body: resp_body,
            cookies,
            url: final_url,
            redirect_chain: Vec::new(), // TODO: redirect following
            tls_peer_certificate: None, // TODO: extract from TLS stream
        })
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
