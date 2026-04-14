//! Platform enum — OS identity for fingerprint consistency.

use leyline_tcp::TcpProfile;

/// Target operating system platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Platform {
    Windows,
    MacOS,
    Linux,
    Android,
    IOS,
}

impl Platform {
    /// The `Sec-CH-UA-Platform` header value.
    pub fn sec_ch_platform(&self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOS => "macOS",
            Self::Linux => "Linux",
            Self::Android => "Android",
            Self::IOS => "iOS",
        }
    }

    /// The `Sec-CH-UA-Mobile` header value.
    pub fn mobile_flag(&self) -> &'static str {
        match self {
            Self::Android | Self::IOS => "?1",
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
        }
    }
}

impl Default for Platform {
    fn default() -> Self {
        Self::Windows
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.sec_ch_platform())
    }
}
