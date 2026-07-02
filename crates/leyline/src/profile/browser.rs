//! Browser enum — maps to a profile in the registry.

/// Identifies a browser for TLS profile selection.
///
/// Each variant maps 1:1 to a TOML profile in `crates/leyline/profiles/`.
/// There are exactly
/// 16 profiles. Most Chromium siblings (Edge, Opera, Vivaldi) are NOT
/// separate variants — their TLS ClientHello is byte-identical to Chrome's.
/// Pick a `ChromeNNN` anchor and apply `ChromiumBrand::{Edge, Opera, Vivaldi}`
/// via `SessionBuilder::brand(..)` to swap HTTP identity headers without
/// changing the JA4 or Akamai fingerprint.
///
/// Brave is the exception: it diverges from Chrome on H2 SETTINGS, header
/// order, and several HTTP headers, so it ships as the first-class
/// [`Browser::Brave146`] variant rather than a brand overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Browser {
    /// Google Chrome 145 (Windows/macOS/Linux/Android).
    Chrome145,
    /// Google Chrome 146 (Windows/macOS/Linux/Android).
    Chrome146,
    /// Google Chrome 147 (Windows/macOS/Linux/Android).
    Chrome147,
    /// Google Chrome 148 (Windows/macOS/Linux/Android).
    ///
    /// Wire-identical to Chrome 147 (same JA4, ciphers, extension set, and
    /// Akamai-H2 fingerprint — verified against tls.peet.ws on 2026-06-09);
    /// the only divergence is the `Chrome/148` UA token and the `sec-ch-ua`
    /// brand list.
    Chrome148,
    /// Google Chrome 149 (Windows/macOS/Linux/Android) — current default and
    /// the current stable release. TLS/JA4 identical to 147 (verified
    /// against real Chrome 149 via tls.peet.ws); identity is the 149 UA +
    /// 3-brand sec-ch-ua.
    Chrome149,
    /// Google Chrome 150 (Windows/macOS/Linux/Android).
    ///
    /// Prepends the ML-DSA post-quantum signature schemes (mldsa44/65/87 =
    /// 0x0904/0905/0906) to the sigalgs list (JA4_3 `cb7bf5808d99`). Requires a
    /// BoringSSL revision with ML-DSA support (>= 3a9254f).
    Chrome150,
    /// Aloha 4.17 — Chromium-138-based privacy browser (Windows/macOS).
    Aloha138,
    /// Brave 1.x — Chromium-146-based privacy browser (macOS only today).
    Brave146,
    /// Mozilla Firefox 148 (Windows/macOS/Linux/Android).
    Firefox148,
    /// Mozilla Firefox 150 (Windows/macOS/Linux/Android) - current release.
    Firefox150,
    /// Mozilla Firefox 151 (Windows/macOS/Linux/Android) - available prerelease/canary capture.
    Firefox151,
    /// Safari 18 on macOS.
    Safari18,
    /// OkHttp 4.x as shipped on Android 10+.
    OkHttpAndroid10,
    /// OkHttp 4.x as shipped on Android 7-9 (TLS 1.2 only).
    OkHttpAndroid7,
    /// Safari on iOS 15.
    SafariIOS15,
    /// Safari on iOS 17.
    SafariIOS17,
    /// Safari on iOS 18.
    SafariIOS18,
}

/// Canonical profile count. Tests assert against this.
pub const PROFILE_COUNT: usize = 17;

/// All browser variants, for iteration.
pub const ALL_BROWSERS: [Browser; PROFILE_COUNT] = [
    Browser::Chrome145,
    Browser::Chrome146,
    Browser::Chrome147,
    Browser::Chrome148,
    Browser::Chrome149,
    Browser::Chrome150,
    Browser::Aloha138,
    Browser::Brave146,
    Browser::Firefox148,
    Browser::Firefox150,
    Browser::Firefox151,
    Browser::Safari18,
    Browser::OkHttpAndroid10,
    Browser::OkHttpAndroid7,
    Browser::SafariIOS15,
    Browser::SafariIOS17,
    Browser::SafariIOS18,
];

impl Browser {
    /// Profile lookup key: (browser_name, version).
    pub fn profile_key(&self) -> (&'static str, u32) {
        match self {
            Self::Chrome145 => ("chrome", 145),
            Self::Chrome146 => ("chrome", 146),
            Self::Chrome147 => ("chrome", 147),
            Self::Chrome148 => ("chrome", 148),
            Self::Chrome149 => ("chrome", 149),
            Self::Chrome150 => ("chrome", 150),
            Self::Aloha138 => ("aloha", 138),
            Self::Brave146 => ("brave", 146),
            Self::Firefox148 => ("firefox", 148),
            Self::Firefox150 => ("firefox", 150),
            Self::Firefox151 => ("firefox", 151),
            Self::Safari18 => ("safari", 18),
            Self::OkHttpAndroid10 => ("okhttp", 10),
            Self::OkHttpAndroid7 => ("okhttp", 7),
            Self::SafariIOS15 => ("safari-ios", 15),
            Self::SafariIOS17 => ("safari-ios", 17),
            Self::SafariIOS18 => ("safari-ios", 18),
        }
    }

    /// Whether this browser caps at TLS 1.2 (no TLS 1.3).
    pub fn max_tls_12(&self) -> bool {
        matches!(self, Self::OkHttpAndroid7)
    }

    /// The default browser for new sessions.
    ///
    /// Pinned to Chrome 149, the current stable release.
    /// Chrome 150 is not yet on the stable channel anywhere, so defaulting a
    /// bare `Session::chrome()` to a `Chrome/150` UA no real user runs would
    /// itself be a fingerprint tell.
    pub fn default_browser() -> Self {
        Self::Chrome149
    }

    /// Chromium major version for Chrome-family browsers.
    ///
    /// Returns `Some(N)` for `ChromeN` variants and `None` for
    /// non-Chromium browsers. Used by [`ChromiumBrand`] overlays to
    /// compute version-specific identity fields — the `sec-ch-ua`
    /// brand list, the `Edg/N.0.0.0` UA suffix, and the Opera lag
    /// table.
    ///
    /// [`ChromiumBrand`]: crate::ChromiumBrand
    pub fn chromium_major(&self) -> Option<u32> {
        match self {
            Self::Chrome145 => Some(145),
            Self::Chrome146 => Some(146),
            Self::Chrome147 => Some(147),
            Self::Chrome148 => Some(148),
            Self::Chrome149 => Some(149),
            Self::Chrome150 => Some(150),
            Self::Aloha138 => Some(138),
            Self::Brave146 => Some(146),
            _ => None,
        }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::default_browser()
    }
}

impl std::fmt::Display for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chrome145 => write!(f, "Chrome 145"),
            Self::Chrome146 => write!(f, "Chrome 146"),
            Self::Chrome147 => write!(f, "Chrome 147"),
            Self::Chrome148 => write!(f, "Chrome 148"),
            Self::Chrome149 => write!(f, "Chrome 149"),
            Self::Chrome150 => write!(f, "Chrome 150"),
            Self::Aloha138 => write!(f, "Aloha 4.17 (Chromium 138)"),
            Self::Brave146 => write!(f, "Brave (Chromium 146)"),
            Self::Firefox148 => write!(f, "Firefox 148"),
            Self::Firefox150 => write!(f, "Firefox 150"),
            Self::Firefox151 => write!(f, "Firefox 151"),
            Self::Safari18 => write!(f, "Safari 18"),
            Self::OkHttpAndroid10 => write!(f, "OkHttp4 Android 10+"),
            Self::OkHttpAndroid7 => write!(f, "OkHttp4 Android 7-9"),
            Self::SafariIOS15 => write!(f, "Safari iOS 15"),
            Self::SafariIOS17 => write!(f, "Safari iOS 17"),
            Self::SafariIOS18 => write!(f, "Safari iOS 18"),
        }
    }
}
