//! Browser enum — maps to a profile in the registry.

/// Identifies a browser for TLS profile selection.
///
/// Each variant maps 1:1 to a TOML profile in `profiles/`. There are exactly
/// 10 profiles. Edge is not a separate variant — use `Chrome147` (Edge shares
/// Chrome's TLS stack).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Browser {
    Chrome145,
    Chrome146,
    Chrome147,
    Firefox148,
    Safari18,
    OkHttpAndroid10,
    OkHttpAndroid7,
    SafariiOS15,
    SafariiOS17,
    SafariiOS18,
}

/// Canonical profile count. Tests assert against this.
pub const PROFILE_COUNT: usize = 10;

/// All browser variants, for iteration.
pub const ALL_BROWSERS: [Browser; PROFILE_COUNT] = [
    Browser::Chrome145,
    Browser::Chrome146,
    Browser::Chrome147,
    Browser::Firefox148,
    Browser::Safari18,
    Browser::OkHttpAndroid10,
    Browser::OkHttpAndroid7,
    Browser::SafariiOS15,
    Browser::SafariiOS17,
    Browser::SafariiOS18,
];

impl Browser {
    /// Profile lookup key: (browser_name, version).
    pub fn profile_key(&self) -> (&'static str, u32) {
        match self {
            Self::Chrome145 => ("chrome", 145),
            Self::Chrome146 => ("chrome", 146),
            Self::Chrome147 => ("chrome", 147),
            Self::Firefox148 => ("firefox", 148),
            Self::Safari18 => ("safari", 18),
            Self::OkHttpAndroid10 => ("okhttp", 10),
            Self::OkHttpAndroid7 => ("okhttp", 7),
            Self::SafariiOS15 => ("safari-ios", 15),
            Self::SafariiOS17 => ("safari-ios", 17),
            Self::SafariiOS18 => ("safari-ios", 18),
        }
    }

    /// Whether this browser caps at TLS 1.2 (no TLS 1.3).
    pub fn max_tls_12(&self) -> bool {
        matches!(self, Self::OkHttpAndroid7)
    }

    /// The default browser for new sessions.
    pub fn default_browser() -> Self {
        Self::Chrome147
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
            Self::Firefox148 => write!(f, "Firefox 148"),
            Self::Safari18 => write!(f, "Safari 18"),
            Self::OkHttpAndroid10 => write!(f, "OkHttp4 Android 10+"),
            Self::OkHttpAndroid7 => write!(f, "OkHttp4 Android 7-9"),
            Self::SafariiOS15 => write!(f, "Safari iOS 15"),
            Self::SafariiOS17 => write!(f, "Safari iOS 17"),
            Self::SafariiOS18 => write!(f, "Safari iOS 18"),
        }
    }
}
