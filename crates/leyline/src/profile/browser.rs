//! Browser enum — maps to a profile in the registry.

/// Identifies a browser for TLS profile selection.
///
/// Each variant maps 1:1 to a TOML profile in `crates/leyline/profiles/`.
/// There are exactly
/// 25 profiles. Most Chromium siblings (Edge, Opera, Vivaldi) are NOT
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
    /// Google Chrome 149 (Windows/macOS/Linux/Android). TLS/JA4 identical to
    /// 147 (verified against real Chrome 149 via tls.peet.ws); identity is the
    /// 149 UA + 3-brand sec-ch-ua. Not the [`Session::chrome`] default — that
    /// is [`Browser::Chrome150`].
    ///
    /// [`Session::chrome`]: crate::Session::chrome
    Chrome149,
    /// Google Chrome 150 (Windows/macOS/Linux/Android).
    ///
    /// Prepends the ML-DSA post-quantum signature schemes (mldsa44/65/87 =
    /// 0x0904/0905/0906) to the sigalgs list (JA4_3 `cb7bf5808d99`). Requires a
    /// BoringSSL revision with ML-DSA support (>= 3a9254f).
    Chrome150,
    /// Brave 1.x — Chromium-146-based privacy browser (macOS only today).
    Brave146,
    /// Mozilla Firefox 148 (Windows/macOS/Linux/Android).
    Firefox148,
    /// Mozilla Firefox 149 (Windows/macOS/Linux/Android). TLS/JA4 identical
    /// to 150 (rapid releases keep the stack across majors); identity is
    /// the 149 UA token.
    Firefox149,
    /// Mozilla Firefox 150 (Windows/macOS/Linux/Android). Default for
    /// [`Session::firefox`].
    ///
    /// [`Session::firefox`]: crate::Session::firefox
    Firefox150,
    /// Mozilla Firefox 151 (Windows/macOS/Linux/Android). TLS-identical to 152.
    Firefox151,
    /// Mozilla Firefox 152 (Windows/macOS/Linux/Android). Newest bundled
    /// Firefox; pin via the builder. [`Session::firefox`] stays on
    /// [`Browser::Firefox150`] until this major is promoted.
    ///
    /// [`Session::firefox`]: crate::Session::firefox
    Firefox152,
    /// Safari 18 on macOS.
    Safari18,
    /// OkHttp 4.x as shipped on Android 10+.
    OkHttpAndroid10,
    /// Safari on iOS 17.
    SafariIOS17,
    /// Safari on iOS 18.
    SafariIOS18,
    /// CFNetwork/URLSession app stack on iOS 18 (captured first-party).
    CfnetworkIOS18,
    /// CFNetwork/URLSession app stack on macOS 26 (captured first-party).
    CfnetworkMacOS26,
}

/// Canonical profile count. Tests assert against this.
pub const PROFILE_COUNT: usize = 18;

/// All browser variants, for iteration.
pub const ALL_BROWSERS: [Browser; PROFILE_COUNT] = [
    Browser::Chrome145,
    Browser::Chrome146,
    Browser::Chrome147,
    Browser::Chrome148,
    Browser::Chrome149,
    Browser::Chrome150,
    Browser::Brave146,
    Browser::Firefox148,
    Browser::Firefox149,
    Browser::Firefox150,
    Browser::Firefox151,
    Browser::Firefox152,
    Browser::Safari18,
    Browser::OkHttpAndroid10,
    Browser::SafariIOS17,
    Browser::SafariIOS18,
    Browser::CfnetworkIOS18,
    Browser::CfnetworkMacOS26,
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
            Self::Brave146 => ("brave", 146),
            Self::Firefox148 => ("firefox", 148),
            Self::Firefox149 => ("firefox", 149),
            Self::Firefox150 => ("firefox", 150),
            Self::Firefox151 => ("firefox", 151),
            Self::Firefox152 => ("firefox", 152),
            Self::Safari18 => ("safari", 18),
            Self::OkHttpAndroid10 => ("okhttp", 10),
            Self::SafariIOS17 => ("safari-ios", 17),
            Self::SafariIOS18 => ("safari-ios", 18),
            Self::CfnetworkIOS18 => ("cfnetwork-ios", 18),
            Self::CfnetworkMacOS26 => ("cfnetwork-macos", 26),
        }
    }

    /// Engine family (`chrome`, `firefox`, `safari-ios`, …).
    ///
    /// TLS rotate stays inside one family. Chrome 150 and Chrome 146 match;
    /// Chrome and Firefox do not.
    #[must_use]
    pub fn family(&self) -> &'static str {
        self.profile_key().0
    }

    /// Representative that owns this browser's ClientHello / JA4.
    ///
    /// Chrome 145 shares 146; 148 and 149 share 147. Firefox 148, 149,
    /// and 150 share one hello; 151 shares 152. Rotate hellos via these,
    /// not every major.
    #[must_use]
    pub fn hello_rep(self) -> Self {
        match self {
            Self::Chrome145 | Self::Chrome146 => Self::Chrome146,
            Self::Chrome147 | Self::Chrome148 | Self::Chrome149 => Self::Chrome147,
            Self::Firefox148 | Self::Firefox149 | Self::Firefox150 => Self::Firefox150,
            Self::Firefox151 | Self::Firefox152 => Self::Firefox152,
            other => other,
        }
    }

    /// Profile in this product line that exists on `platform`.
    ///
    /// Safari on macOS and Safari on iOS are different [`Browser`] variants.
    /// Same for CFNetwork. Chrome and Firefox keep the same variant; build
    /// fails later if that profile has no identity block for the OS.
    ///
    /// Used by [`crate::SessionBuilder::macos`] / [`crate::SessionBuilder::ios`]
    /// so `.safari().ios()` is Safari on iPhone, not a config error.
    #[must_use]
    pub fn for_platform(self, platform: crate::profile::Platform) -> Self {
        use crate::profile::Platform;
        match (self, platform) {
            (Self::Safari18, Platform::IOS) => Self::SafariIOS18,
            (Self::SafariIOS17 | Self::SafariIOS18, Platform::MacOS) => Self::Safari18,
            (Self::CfnetworkMacOS26, Platform::IOS) => Self::CfnetworkIOS18,
            (Self::CfnetworkIOS18, Platform::MacOS) => Self::CfnetworkMacOS26,
            (other, _) => other,
        }
    }

    /// Distinct ClientHello owners in this [`Browser::family`], newest first.
    #[must_use]
    pub fn family_hellos(self) -> &'static [Self] {
        match self.family() {
            "chrome" => &[Self::Chrome150, Self::Chrome147, Self::Chrome146],
            "firefox" => &[Self::Firefox152, Self::Firefox150],
            "safari" => &[Self::Safari18],
            "safari-ios" => &[Self::SafariIOS18, Self::SafariIOS17],
            "cfnetwork-ios" => &[Self::CfnetworkIOS18],
            "cfnetwork-macos" => &[Self::CfnetworkMacOS26],
            "brave" => &[Self::Brave146],
            "okhttp" => &[Self::OkHttpAndroid10],
            _ => &[],
        }
    }

    /// Whether this browser caps at TLS 1.2 (no TLS 1.3). No bundled
    /// profile is TLS 1.2-capped today (the Android 7 profile was removed).
    #[allow(clippy::unused_self)]
    pub fn max_tls_12(&self) -> bool {
        false
    }

    /// Whether this is a Firefox (Gecko) profile. Firefox emits no `Sec-CH-UA*`
    /// Client Hints and a Gecko `Accept`, so the header presets gate those off
    /// the wire for a Firefox identity (see `Preset::build_headers`).
    #[must_use]
    pub fn is_firefox(&self) -> bool {
        self.family() == "firefox"
    }

    /// The default browser for new sessions.
    ///
    /// Chrome 150 on the stable channel. A bare `Session::chrome()` uses
    /// the latest verified profile (JA4 `t13d1517…cb7bf5808d99`,
    /// live-checked against tls.peet.ws). Pin an older major via the
    /// builder (e.g. `.browser(Browser::Chrome149)`) when you need a
    /// fixed version.
    pub fn default_browser() -> Self {
        Self::Chrome150
    }

    /// The Firefox profile [`crate::Session::firefox`] and
    /// [`crate::SessionBuilder::firefox`] select.
    ///
    /// Stays on Firefox 150 until a live peet.ws pass promotes
    /// [`Browser::Firefox152`]. Pin 152 via `.browser(Browser::Firefox152)`.
    pub fn default_firefox() -> Self {
        Self::Firefox150
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
            Self::Brave146 => write!(f, "Brave (Chromium 146)"),
            Self::Firefox148 => write!(f, "Firefox 148"),
            Self::Firefox149 => write!(f, "Firefox 149"),
            Self::Firefox150 => write!(f, "Firefox 150"),
            Self::Firefox151 => write!(f, "Firefox 151"),
            Self::Firefox152 => write!(f, "Firefox 152"),
            Self::Safari18 => write!(f, "Safari 18"),
            Self::OkHttpAndroid10 => write!(f, "OkHttp4 Android 10+"),
            Self::SafariIOS17 => write!(f, "Safari iOS 17"),
            Self::SafariIOS18 => write!(f, "Safari iOS 18"),
            Self::CfnetworkIOS18 => write!(f, "CFNetwork iOS 18"),
            Self::CfnetworkMacOS26 => write!(f, "CFNetwork macOS 26"),
        }
    }
}
