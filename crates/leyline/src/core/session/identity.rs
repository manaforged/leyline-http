//! Locked browser presentation for a session.

use crate::core::error::{Error, Kind, Result};
use crate::profile::{Browser, Platform, ProfileRegistry};

/// HTTP identity plus the TLS profile that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    http: Browser,
    tls: Browser,
    platform: Platform,
}

impl Identity {
    /// HTTP headers from `browser`; TLS/H2 from that browser's hello owner.
    #[must_use]
    pub fn locked(browser: Browser, platform: Platform) -> Self {
        Self {
            http: browser,
            tls: browser.hello_rep(),
            platform,
        }
    }

    /// Roll TLS/H2 to `tls`.
    pub fn rotate_tls(self, tls: Browser) -> Result<Self> {
        if self.http.family() != tls.family() {
            return Err(Error::new(Kind::Config).with_message(format!(
                "tls rotate {tls} is not the same family as {}",
                self.http
            )));
        }
        Ok(Self {
            tls: tls.hello_rep(),
            ..self
        })
    }

    /// Next distinct ClientHello in this family.
    pub fn rotate_hello(self) -> Result<Self> {
        let hellos = self.http.family_hellos();
        if hellos.len() < 2 {
            return Err(Error::new(Kind::Config)
                .with_message(format!("no other hello in family {}", self.http.family())));
        }
        let cur = self.tls.hello_rep();
        let i = hellos.iter().position(|&b| b == cur).unwrap_or(0);
        self.rotate_tls(hellos[(i + 1) % hellos.len()])
    }

    /// Every distinct-hello stack with this HTTP identity and platform.
    pub fn hello_library(self) -> Result<Vec<Self>> {
        self.http
            .family_hellos()
            .iter()
            .map(|&tls| self.rotate_tls(tls))
            .collect()
    }

    /// Families a jar can pass to on this platform (HTTP + TLS both switch).
    const PASS_REPS: &[Browser] = &[
        Browser::Chrome152,
        Browser::Firefox154,
        Browser::Safari26,
        Browser::SafariIOS18,
    ];

    /// Locked identity in `dest`'s family, same platform.
    pub fn pass(self, dest: Browser) -> Result<Self> {
        if dest.family() == self.http.family() {
            return Err(Error::new(Kind::Config).with_message(format!(
                "cookie pass {dest} is the same family as {} — rotate_tls instead",
                self.http
            )));
        }
        let id = Self::locked(dest, self.platform);
        let _ = id.user_agent()?;
        Ok(id)
    }

    /// Every other family that can carry this platform's HTTP identity.
    #[must_use]
    pub fn pass_library(self) -> Vec<Self> {
        Self::PASS_REPS
            .iter()
            .filter_map(|&dest| self.pass(dest).ok())
            .collect()
    }

    /// Browser that supplies UA, Client Hints, and identity extras.
    #[must_use]
    pub fn http(self) -> Browser {
        self.http
    }

    /// Browser that supplies the TLS ClientHello and H2 settings.
    #[must_use]
    pub fn tls(self) -> Browser {
        self.tls
    }

    /// OS identity for this session.
    #[must_use]
    pub fn platform(self) -> Platform {
        self.platform
    }

    /// `User-Agent` from the HTTP profile for this platform.
    pub fn user_agent(self) -> Result<String> {
        Ok(self.http_platform()?.user_agent.clone())
    }

    /// `sec-ch-ua` from the HTTP profile for this platform.
    pub fn sec_ch_ua(self) -> Result<String> {
        Ok(self.http_platform()?.sec_ch_ua.clone())
    }

    fn http_platform(self) -> Result<&'static crate::profile::PlatformIdentity> {
        let profile = ProfileRegistry::global()
            .get_browser(self.http)
            .ok_or_else(|| {
                Error::new(Kind::Config).with_message(format!("no profile for {}", self.http))
            })?;
        profile.identity_for(self.platform).ok_or_else(|| {
            Error::new(Kind::Config)
                .with_message(format!("no {} identity for {}", self.platform, self.http))
        })
    }
}
