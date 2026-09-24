use crate::tcp::TcpProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[derive(Default)]
pub enum Platform {
    #[default]
    Windows,
    MacOS,
    Linux,
    Android,
    IOS,
    Host,
}

impl Platform {
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

    bench_pub! {
        fn tcp_profile(&self) -> TcpProfile {
            match self {
                Self::Windows => TcpProfile::WINDOWS,
                Self::MacOS => TcpProfile::MACOS,
                Self::Linux | Self::Android => TcpProfile::LINUX,
                Self::IOS => TcpProfile::IOS,
                Self::Host => Self::detect_host().tcp_profile(),
            }
        }
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
