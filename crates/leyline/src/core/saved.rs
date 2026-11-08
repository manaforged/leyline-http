use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cookie::Jar;
use crate::core::atomic::{FileMode, write_atomic};
use crate::core::config::Proxy;
use crate::core::session::Session;
use crate::core::{Identity, SessionBuilder};
use crate::error::{Error, ProfileChanged, Result};
use crate::net::ProxyUrl;
use crate::profile::{BrowserProfile, ChromiumBrand, Platform};
use crate::util::version::{self, Versioned};

mod autosave;
mod secret;
mod state;

pub use state::ConnectionState;
pub(crate) use state::StateParts;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SessionVersion {
    #[default]
    V1,
}

impl Versioned for SessionVersion {
    fn number(self) -> u32 {
        match self {
            SessionVersion::V1 => 1,
        }
    }

    fn from_number(number: u32) -> Option<Self> {
        match number {
            1 => Some(SessionVersion::V1),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct SavedFile {
    #[serde(with = "version")]
    version: SessionVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity: Option<Identity>,
    #[serde(default, skip_serializing)]
    platform: Option<Platform>,
    #[serde(default, skip_serializing)]
    brand: Option<ChromiumBrand>,
    #[serde(default, skip_serializing)]
    profile_toml: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy: Option<ProxyUrl>,
    #[serde(default, skip_serializing)]
    proxy_password_env: Option<String>,
    #[serde(default, skip_serializing)]
    languages: Option<Vec<String>>,
    #[serde(default, skip_serializing)]
    env_proxy: bool,
    #[serde(default, skip_serializing)]
    jar_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    jar: Option<Jar>,
    #[serde(default)]
    state: ConnectionState,
    #[serde(default, skip_serializing)]
    strict: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Saved {
    profile_id: Option<String>,
    state: ConnectionState,
}

impl Session {
    /// Writes the session state as JSON to `path`, without the proxy password.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(Error::from_json)?;
        write_atomic(path.as_ref(), &bytes, FileMode::Private)
    }

    /// Reads a saved session file and returns a `SessionBuilder` that restores it.
    pub fn load(path: impl AsRef<Path>) -> Result<SessionBuilder> {
        let bytes = std::fs::read(path)?;
        let saved: SavedFile = serde_json::from_slice(&bytes).map_err(Error::from_json)?;
        saved.into_builder()
    }
}

impl Serialize for Session {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        SavedFile {
            version: SessionVersion::default(),
            identity: Some(self.identity()),
            platform: None,
            brand: None,
            profile_toml: None,
            profile_id: self.profile_id(),
            proxy: self.proxy_url().map(|proxy| proxy.without_password()),
            proxy_password_env: None,
            languages: None,
            env_proxy: false,
            jar_path: None,
            jar: Some(self.cookies().snapshot()),
            state: self.connection_state(),
            strict: false,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SessionBuilder {
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        SavedFile::deserialize(de)?
            .into_builder()
            .map_err(serde::de::Error::custom)
    }
}

impl SavedFile {
    fn into_builder(mut self) -> Result<SessionBuilder> {
        match self.version {
            SessionVersion::V1 => {}
        }
        self.strict_gaps()?;
        let proxy = self
            .resolved_proxy()?
            .map_or_else(Proxy::new, Proxy::from)
            .env(self.env_proxy);
        let identity = self.identity()?;
        let cookies = match (self.jar_path, self.jar) {
            (Some(path), _) if path.exists() => Jar::load_from(&path)?,
            (_, Some(jar)) => jar,
            (_, None) => Jar::new(),
        };
        Ok(Session::builder()
            .identity(identity)
            .proxy(proxy)
            .cookies(cookies)
            .saved(Saved {
                profile_id: self.profile_id,
                state: self.state,
            }))
    }

    fn identity(&mut self) -> Result<Identity> {
        let platform = self
            .platform
            .or(self.identity.as_ref().map(Identity::platform))
            .ok_or_else(|| Error::config("the saved file has no platform"));
        let mut identity = match (&self.profile_toml, self.identity.take()) {
            (Some(toml), _) => Identity::from_profile(BrowserProfile::from_toml(toml)?, platform?),
            (None, Some(identity)) => identity,
            (None, None) => Identity::plain(platform?),
        };
        if let Some(brand) = self.brand {
            identity = identity.with_brand(brand);
        }
        if let Some(languages) = &self.languages {
            identity = identity.with_languages(languages);
        }
        Ok(identity)
    }

    fn strict_gaps(&self) -> Result<()> {
        if self.strict && self.profile_id.is_none() {
            return Err(Error::config("the strict saved file has no profile_id"));
        }
        if self.strict && self.proxy.is_none() && !self.env_proxy {
            return Err(Error::config("the strict saved file has no proxy"));
        }
        Ok(())
    }
}

impl Saved {
    pub(crate) fn apply(&self, session: &Session) -> Result<()> {
        if let Some(expected) = self.profile_id.as_deref() {
            let actual = session.profile_id();
            if actual.as_deref() != Some(expected) {
                return Err(Error::config(format!(
                    "the session differs from the saved file: profile_id {actual:?} (saved {expected:?})"
                ))
                .with_source(ProfileChanged));
            }
        }
        session.restore(&self.state);
        Ok(())
    }
}
