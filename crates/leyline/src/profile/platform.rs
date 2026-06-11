//! Platform enum — OS identity for fingerprint consistency.

use crate::tcp::TcpProfile;

/// Target operating system platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(Default)]
pub enum Platform {
    /// Microsoft Windows (`Sec-CH-UA-Platform: "Windows"`).
    #[default]
    Windows,
    /// Apple macOS (`Sec-CH-UA-Platform: "macOS"`).
    MacOS,
    /// Linux desktop (`Sec-CH-UA-Platform: "Linux"`).
    Linux,
    /// Google Android (mobile flag `?1`).
    Android,
    /// Apple iOS / iPadOS (mobile flag `?1`).
    IOS,
    /// Detect the host OS at build time (`cfg!(target_os)`), falling back
    /// to [`Platform::Windows`] for an unrecognised target. This is the
    /// default for a **bare** (non-impersonating) session — an internal
    /// call from a Linux box honestly looks like Linux. Resolved to a
    /// concrete variant by [`Platform::resolve`] before any use, so it
    /// never reaches the wire as-is.
    Host,
}

impl Platform {
    /// Concrete host OS from the compile target. Never returns
    /// [`Platform::Host`]; unknown targets fall back to Windows.
    pub fn detect_host() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOS
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(target_os = "android")]
        {
            Self::Android
        }
        #[cfg(target_os = "ios")]
        {
            Self::IOS
        }
        #[cfg(not(any(
            target_os = "windows",
            target_os = "macos",
            target_os = "linux",
            target_os = "android",
            target_os = "ios"
        )))]
        {
            Self::Windows
        }
    }

    /// Resolve [`Platform::Host`] to the concrete host OS; a no-op for every
    /// explicit variant. Call before reading any platform-derived value so
    /// `Host` never leaks into a fingerprint.
    #[must_use]
    pub fn resolve(self) -> Self {
        match self {
            Self::Host => Self::detect_host(),
            other => other,
        }
    }

    /// The `Sec-CH-UA-Platform` header value.
    pub fn sec_ch_platform(&self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOS => "macOS",
            Self::Linux => "Linux",
            Self::Android => "Android",
            Self::IOS => "iOS",
            Self::Host => Self::detect_host().sec_ch_platform(),
        }
    }

    /// The `Sec-CH-UA-Mobile` header value.
    pub fn mobile_flag(&self) -> &'static str {
        match self {
            Self::Android | Self::IOS => "?1",
            Self::Host => Self::detect_host().mobile_flag(),
            _ => "?0",
        }
    }

    /// Map to the TCP fingerprint profile for this OS.
    pub fn tcp_profile(&self) -> TcpProfile {
        match self {
            Self::Windows => TcpProfile::WINDOWS,
            Self::MacOS => TcpProfile::MACOS,
            Self::Linux | Self::Android => TcpProfile::LINUX,
            Self::IOS => TcpProfile::IOS,
            Self::Host => Self::detect_host().tcp_profile(),
        }
    }

    /// Profile identity lookup key (lowercase).
    pub fn identity_key(&self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOS => "macos",
            Self::Linux => "linux",
            Self::Android => "android",
            Self::IOS => "ios",
            Self::Host => Self::detect_host().identity_key(),
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.sec_ch_platform())
    }
}
