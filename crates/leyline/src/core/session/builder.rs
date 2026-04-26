use std::sync::{Arc, LazyLock};

use crate::cookie::Jar as CookieJar;
use crate::h2::H2Config;
use crate::pool::Pool;
use crate::profile::{Browser, ChromiumBrand, Platform, ProfileRegistry};
use crate::tcp::TcpProfile;
use crate::tls::FingerprintConnector;

use super::proxy::env_proxy;
use super::{AuditTlsCache, ProtocolPolicy, Session};
use crate::core::error::{Error, Result};

static PROFILES: LazyLock<ProfileRegistry> = LazyLock::new(ProfileRegistry::builtin);

/// Session builder - configure browser, platform, proxy, timeout, cookies.
pub struct SessionBuilder {
    browser: Browser,
    platform: Platform,
    brand: ChromiumBrand,
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
    pub(super) fn new() -> Self {
        Self {
            browser: Browser::default(),
            platform: Platform::default(),
            brand: ChromiumBrand::default(),
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

    /// Apply a Chromium-family identity overlay (Edge, Brave, Opera).
    ///
    /// Leaves TLS ClientHello and HTTP/2 SETTINGS untouched — those
    /// are byte-identical across Chromium siblings at a given
    /// Chromium version. What changes is a small set of HTTP
    /// identity headers:
    ///
    /// - `user-agent` suffix — `Edg/NNN` for Edge, `OPR/NNN` for
    ///   Opera, unchanged for Brave (Brave matches Chrome's UA by
    ///   design).
    /// - `sec-ch-ua` brand list — `"Microsoft Edge"`, `"Brave"`, or
    ///   `"Opera"` in place of `"Google Chrome"`. Opera additionally
    ///   uses the `"Not:A-Brand"` placeholder form.
    /// - Extra privacy headers — `dnt: 1` for Edge, `sec-gpc: 1`
    ///   for Brave.
    /// - Navigation `accept` — Brave drops `signed-exchange;v=b3`
    ///   because it disables signed exchanges by default.
    ///
    /// The brand setter is a no-op when applied to non-Chromium
    /// profiles (Firefox, Safari, OkHttp) — the overlay only
    /// affects Chrome profiles. Opera is typically based on
    /// `Chromium N-2`; pair with `Browser::Chrome145` for Opera 129.
    pub fn brand(mut self, brand: ChromiumBrand) -> Self {
        self.brand = brand;
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
    pub fn build(mut self) -> Result<Session> {
        // Honour standard proxy environment variables when no explicit
        // `proxy(..)` has been set. Matches reqwest / curl / Python
        // requests behaviour: a CLI user exporting `HTTPS_PROXY` gets
        // it picked up without re-plumbing the session. The
        // precedence order is:
        //   1. Explicit `SessionBuilder::proxy(..)` (highest).
        //   2. `HTTPS_PROXY` (upper or lower case).
        //   3. `HTTP_PROXY` (upper or lower case).
        // `NO_PROXY` is honoured per-request in the transport layer
        // — building the session with a proxy plus `NO_PROXY`
        // patterns means some hosts bypass it.
        if self.proxy.is_none() {
            if let Some(p) = env_proxy() {
                self.proxy = Some(p);
            }
        }
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
        let mut identity = profile
            .identity_for(identity_key)
            .ok_or_else(|| {
                Error::Config(format!("no {} identity for {}", identity_key, self.browser))
            })?
            .clone();

        // Apply the Chromium-sibling identity overlay if the caller
        // asked for one. Only meaningful on Chrome profiles;
        // `chromium_major()` returns None for Firefox / Safari /
        // OkHttp, which short-circuits the overlay to a no-op. An
        // unverified (brand, Chromium, platform) combination
        // returns an error rather than silently emitting headers
        // we haven't captured against a real browser.
        let mut brand_extra_headers: Vec<(String, String)> = Vec::new();
        let mut brand_navigate_accept: Option<String> = None;
        if self.brand != ChromiumBrand::Chrome {
            if let Some(chromium_major) = self.browser.chromium_major() {
                let overlay = self
                    .brand
                    .overlay(
                        chromium_major,
                        self.platform,
                        &identity.user_agent,
                        &identity.sec_ch_ua,
                    )
                    .map_err(|e| Error::Config(format!("{e}")))?;
                if let Some(overlay) = overlay {
                    identity.user_agent = overlay.user_agent;
                    identity.sec_ch_ua = overlay.sec_ch_ua;
                    brand_extra_headers = overlay.extra_headers;
                    brand_navigate_accept = overlay.navigate_accept;
                }
            }
        }

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

        // Build H2 config from profile, applying any per-platform
        // override (e.g. Chromium-on-macOS drops `unknown_setting8`).
        let resolved_h2 = profile.h2.resolve_for_platform(identity_key);
        let h2_config = H2Config::from_profile(&resolved_h2);

        // Pre-compute audit data from profile.
        let extension_ids = crate::audit::chrome_extension_ids(&profile.tls);
        let ja4 = {
            let input = crate::audit::Ja4Input {
                ciphers: &profile.tls.ciphers,
                sigalgs: &profile.tls.sigalgs,
                curves: &profile.tls.curves,
                extension_ids: &extension_ids,
                tls_version: "1.3",
                has_sni: true,
                alpn: "h2",
            };
            crate::audit::compute_ja4(&input)
        };
        let ja3 = {
            let input = crate::audit::Ja3Input {
                ciphers: &profile.tls.ciphers,
                curves: &profile.tls.curves,
                extension_ids: &extension_ids,
                tls_record_version: 771, // TLS 1.2 record layer
            };
            crate::audit::compute_ja3(&input)
        };
        let h2_fp = h2_config.akamai_fingerprint();
        let is_windows = self.platform == Platform::Windows;
        let ja4t = crate::audit::compute_ja4t(
            tcp_profile.window_size,
            tcp_profile.mss as u16,
            tcp_profile.window_scale as u8,
            is_windows,
        );

        Ok(Session {
            browser: self.browser,
            platform: self.platform,
            brand: self.brand,
            user_agent: identity.user_agent,
            sec_ch_ua: identity.sec_ch_ua,
            accept_language: identity
                .accept_language
                .unwrap_or_else(|| "en-US,en;q=0.9".to_string()),
            brand_extra_headers,
            brand_navigate_accept,
            identity_extra_headers: identity.extra_headers.clone(),
            identity_navigate_accept: identity.navigate_accept_override.clone(),
            identity_request_header_order: identity.request_header_order.clone(),
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
                "chromium" => crate::quic::H3Config::chrome(),
                "firefox" => crate::quic::H3Config::firefox(),
                "safari" | "webkit" => crate::quic::H3Config::safari(),
                _ => crate::quic::H3Config::chrome(),
            },
            profile,
        })
    }
}
