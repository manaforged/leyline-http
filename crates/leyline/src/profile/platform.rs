use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::tcp::TcpProfile;

#[derive(Deserialize)]
struct PlatformData {
    tcp: TcpProfile,
}

static PLATFORMS: LazyLock<HashMap<String, PlatformData>> = LazyLock::new(|| {
    toml::from_str(include_str!("../../profiles/platforms.toml"))
        .expect("built-in platform table is statically valid")
});

#[doc = include_str!("../../docs/platform.md")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(Default)]
pub enum Platform {
    /// Microsoft Windows, the default platform.
    #[default]
    Windows,
    /// Apple macOS.
    MacOS,
    /// Desktop Linux.
    Linux,
    /// Google Android.
    Android,
    /// Apple iOS.
    IOS,
    /// The operating system this program runs on, detected at use.
    Host,
}

const PLATFORM_ALL: &[Platform] = &[
    Platform::Windows,
    Platform::MacOS,
    Platform::Linux,
    Platform::Android,
    Platform::IOS,
    Platform::Host,
];

impl Platform {
    /// Returns every platform, including `Host`.
    #[must_use]
    pub fn all() -> &'static [Platform] {
        PLATFORM_ALL
    }

    /// Returns the platform's stable id, such as `"host"` for `Host`.
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::Host => "host",
            other => other.identity_key(),
        }
    }

    pub(crate) fn detect_host() -> Self {
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

    #[must_use]
    pub(crate) fn resolve(self) -> Self {
        match self {
            Self::Host => Self::detect_host(),
            other => other,
        }
    }

    pub(crate) fn sec_ch_platform(&self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOS => "macOS",
            Self::Linux => "Linux",
            Self::Android => "Android",
            Self::IOS => "iOS",
            Self::Host => Self::detect_host().sec_ch_platform(),
        }
    }

    pub(crate) fn mobile_flag(&self) -> &'static str {
        match self {
            Self::Android | Self::IOS => "?1",
            Self::Host => Self::detect_host().mobile_flag(),
            _ => "?0",
        }
    }

    /// Returns the platform's TCP fingerprint, or the default profile when the table has none.
    #[must_use]
    pub fn tcp_profile(&self) -> TcpProfile {
        PLATFORMS
            .get(self.identity_key())
            .map(|data| data.tcp.clone())
            .unwrap_or_default()
    }

    bench_pub! {
        fn identity_key(&self) -> &'static str {
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
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.sec_ch_platform())
    }
}

impl std::str::FromStr for Platform {
    type Err = crate::Error;

    fn from_str(id: &str) -> Result<Self, Self::Err> {
        PLATFORM_ALL
            .iter()
            .copied()
            .find(|platform| platform.id().eq_ignore_ascii_case(id))
            .ok_or_else(|| {
                crate::profile::browser::unknown(
                    "platform",
                    id,
                    PLATFORM_ALL.iter().map(Platform::id),
                )
            })
    }
}

string_id_serde!(Platform);
