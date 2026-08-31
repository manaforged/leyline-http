//! Browser enum — maps to a profile in the registry.

/// Identifies a browser for TLS profile selection.
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
    Chrome148,
    /// Google Chrome 149 (Windows/macOS/Linux/Android).
    Chrome149,
    /// Google Chrome 150 (Windows/macOS/Linux/Android).
    Chrome150,
    /// Google Chrome 151.
    Chrome151,
    /// Google Chrome 152.
    Chrome152,
    /// Brave 1.x — Chromium-146-based.
    Brave146,
    /// Mozilla Firefox 148 (Windows/macOS/Linux/Android).
    Firefox148,
    /// Mozilla Firefox 149 (Windows/macOS/Linux/Android).
    Firefox149,
    /// Mozilla Firefox 150 (Windows/macOS/Linux/Android).
    Firefox150,
    /// Mozilla Firefox 151 (Windows/macOS/Linux/Android).
    Firefox151,
    /// Mozilla Firefox 152 (Windows/macOS/Linux/Android).
    Firefox152,
    /// Mozilla Firefox 153.
    Firefox153,
    /// Mozilla Firefox 154.
    Firefox154,
    /// Safari 18 on macOS.
    Safari18,
    /// Safari 26.
    Safari26,
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

/// Canonical profile count.
pub const PROFILE_COUNT: usize = 23;

/// All browser variants, for iteration.
pub const ALL_BROWSERS: [Browser; PROFILE_COUNT] = [
    Browser::Chrome145,
    Browser::Chrome146,
    Browser::Chrome147,
    Browser::Chrome148,
    Browser::Chrome149,
    Browser::Chrome150,
    Browser::Chrome151,
    Browser::Chrome152,
    Browser::Brave146,
    Browser::Firefox148,
    Browser::Firefox149,
    Browser::Firefox150,
    Browser::Firefox151,
    Browser::Firefox152,
    Browser::Firefox153,
    Browser::Firefox154,
    Browser::Safari18,
    Browser::Safari26,
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
            Self::Chrome151 => ("chrome", 151),
            Self::Chrome152 => ("chrome", 152),
            Self::Brave146 => ("brave", 146),
            Self::Firefox148 => ("firefox", 148),
            Self::Firefox149 => ("firefox", 149),
            Self::Firefox150 => ("firefox", 150),
            Self::Firefox151 => ("firefox", 151),
            Self::Firefox152 => ("firefox", 152),
            Self::Firefox153 => ("firefox", 153),
            Self::Firefox154 => ("firefox", 154),
            Self::Safari18 => ("safari", 18),
            Self::Safari26 => ("safari", 26),
            Self::OkHttpAndroid10 => ("okhttp", 10),
            Self::SafariIOS17 => ("safari-ios", 17),
            Self::SafariIOS18 => ("safari-ios", 18),
            Self::CfnetworkIOS18 => ("cfnetwork-ios", 18),
            Self::CfnetworkMacOS26 => ("cfnetwork-macos", 26),
        }
    }

    /// Engine family (`chrome`, `firefox`, `safari-ios`, …).
    #[must_use]
    pub fn family(&self) -> &'static str {
        self.profile_key().0
    }

    /// Representative that owns this browser's ClientHello / JA4.
    #[must_use]
    pub fn hello_rep(self) -> Self {
        match self {
            Self::Chrome145 | Self::Chrome146 => Self::Chrome146,
            Self::Chrome147 | Self::Chrome148 | Self::Chrome149 => Self::Chrome147,
            Self::Chrome150 | Self::Chrome151 | Self::Chrome152 => Self::Chrome152,
            Self::Firefox148 | Self::Firefox149 | Self::Firefox150 => Self::Firefox150,
            Self::Firefox151 | Self::Firefox152 | Self::Firefox153 => Self::Firefox152,
            Self::Firefox154 => Self::Firefox154,
            Self::Safari26 => Self::Safari26,
            Self::Safari18 => Self::Safari18,
            other => other,
        }
    }

    /// Profile in this product line that exists on `platform`.
    #[must_use]
    pub fn for_platform(self, platform: crate::profile::Platform) -> Self {
        use crate::profile::Platform;
        match (self, platform) {
            (Self::Safari18 | Self::Safari26, Platform::IOS) => Self::SafariIOS18,
            (Self::SafariIOS17 | Self::SafariIOS18, Platform::MacOS) => Self::Safari26,
            (Self::CfnetworkMacOS26, Platform::IOS) => Self::CfnetworkIOS18,
            (Self::CfnetworkIOS18, Platform::MacOS) => Self::CfnetworkMacOS26,
            (other, _) => other,
        }
    }

    /// Distinct ClientHello owners in this [`Browser::family`], newest first.
    #[must_use]
    pub fn family_hellos(self) -> &'static [Self] {
        match self.family() {
            "chrome" => &[Self::Chrome152, Self::Chrome147, Self::Chrome146],
            "firefox" => &[Self::Firefox154, Self::Firefox152, Self::Firefox150],
            "safari" => &[Self::Safari26, Self::Safari18],
            "safari-ios" => &[Self::SafariIOS18, Self::SafariIOS17],
            "cfnetwork-ios" => &[Self::CfnetworkIOS18],
            "cfnetwork-macos" => &[Self::CfnetworkMacOS26],
            "brave" => &[Self::Brave146],
            "okhttp" => &[Self::OkHttpAndroid10],
            _ => &[],
        }
    }

    /// Whether this browser caps at TLS 1.2 (no TLS 1.3).
    #[allow(clippy::unused_self)]
    pub fn max_tls_12(&self) -> bool {
        false
    }

    /// Whether this is a Firefox (Gecko) profile.
    #[must_use]
    pub fn is_firefox(&self) -> bool {
        self.family() == "firefox"
    }

    /// The default browser for new sessions.
    pub fn default_browser() -> Self {
        Self::Chrome152
    }

    /// The Firefox profile [`crate::Session::firefox`] and [`crate::SessionBuilder::firefox`] select.
    pub fn default_firefox() -> Self {
        Self::Firefox154
    }

    /// Chromium major version for Chrome-family browsers.
    pub fn chromium_major(&self) -> Option<u32> {
        match self {
            Self::Chrome145 => Some(145),
            Self::Chrome146 => Some(146),
            Self::Chrome147 => Some(147),
            Self::Chrome148 => Some(148),
            Self::Chrome149 => Some(149),
            Self::Chrome150 => Some(150),
            Self::Chrome151 => Some(151),
            Self::Chrome152 => Some(152),
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
            Self::Chrome151 => write!(f, "Chrome 151"),
            Self::Chrome152 => write!(f, "Chrome 152"),
            Self::Brave146 => write!(f, "Brave (Chromium 146)"),
            Self::Firefox148 => write!(f, "Firefox 148"),
            Self::Firefox149 => write!(f, "Firefox 149"),
            Self::Firefox150 => write!(f, "Firefox 150"),
            Self::Firefox151 => write!(f, "Firefox 151"),
            Self::Firefox152 => write!(f, "Firefox 152"),
            Self::Firefox153 => write!(f, "Firefox 153"),
            Self::Firefox154 => write!(f, "Firefox 154"),
            Self::Safari18 => write!(f, "Safari 18"),
            Self::Safari26 => write!(f, "Safari 26"),
            Self::OkHttpAndroid10 => write!(f, "OkHttp4 Android 10+"),
            Self::SafariIOS17 => write!(f, "Safari iOS 17"),
            Self::SafariIOS18 => write!(f, "Safari iOS 18"),
            Self::CfnetworkIOS18 => write!(f, "CFNetwork iOS 18"),
            Self::CfnetworkMacOS26 => write!(f, "CFNetwork macOS 26"),
        }
    }
}
