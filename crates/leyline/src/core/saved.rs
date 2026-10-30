use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cookie::Jar;
use crate::core::error::{Cause, Error, ProfileChanged, Result};
use crate::core::{Identity, Proxy, ProxyUrl, Session, SessionBuilder};
use crate::profile::{BrowserProfile, ChromiumBrand, Platform};
use crate::util::atomic::{FileMode, write_atomic};

mod autosave;
mod secret;
mod state;
mod version;

pub use state::ConnectionState;
pub(crate) use state::{StateParts, unix_secs};
pub(crate) use version::FileVersion;

#[derive(Serialize)]
struct SavedOut<'a> {
    version: FileVersion,
    identity: &'a Identity,
    platform: Platform,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy: Option<ProxyUrl>,
    jar: Jar,
    state: ConnectionState,
}

#[derive(Deserialize)]
struct SavedIn {
    version: FileVersion,
    #[serde(default)]
    identity: Option<Identity>,
    platform: Platform,
    #[serde(default)]
    brand: Option<ChromiumBrand>,
    #[serde(default)]
    profile_toml: Option<String>,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    proxy: Option<ProxyUrl>,
    #[serde(default)]
    proxy_password_env: Option<String>,
    #[serde(default)]
    languages: Option<Vec<String>>,
    #[serde(default)]
    env_proxy: bool,
    #[serde(default)]
    jar_path: Option<PathBuf>,
    #[serde(default)]
    jar: Option<Jar>,
    #[serde(default)]
    state: ConnectionState,
    #[serde(default)]
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
        let saved: SavedIn = serde_json::from_slice(&bytes).map_err(Error::from_json)?;
        saved.into_builder()
    }
}

impl Serialize for Session {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let identity = self.identity();
        SavedOut {
            version: FileVersion::default(),
            identity: &identity,
            platform: identity.platform(),
            profile_id: self.profile_id(),
            proxy: self.proxy_url().map(|proxy| secret::strip_password(&proxy)),
            jar: self.cookies().snapshot(),
            state: self.connection_state(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SessionBuilder {
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        SavedIn::deserialize(de)?
            .into_builder()
            .map_err(serde::de::Error::custom)
    }
}

impl SavedIn {
    fn into_builder(self) -> Result<SessionBuilder> {
        match self.version {
            FileVersion::V1 => {}
        }
        self.strict_gaps()?;
        let proxy = self
            .resolved_proxy()?
            .map_or_else(Proxy::new, Proxy::from)
            .env(self.env_proxy);
        let mut identity = match (&self.profile_toml, self.identity) {
            (Some(toml), _) => {
                Identity::from_profile(BrowserProfile::from_toml(toml)?, self.platform)
            }
            (None, Some(identity)) => identity,
            (None, None) => Identity::plain(self.platform),
        };
        if let Some(brand) = self.brand {
            identity = identity.with_brand(brand);
        }
        if let Some(languages) = &self.languages {
            identity = identity.with_languages(languages);
        }
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

    fn strict_gaps(&self) -> Result<()> {
        if self.strict && self.profile_id.is_none() {
            return Err(config("the strict saved file has no profile_id"));
        }
        if self.strict && self.proxy.is_none() && !self.env_proxy {
            return Err(config("the strict saved file has no proxy"));
        }
        Ok(())
    }
}

impl Saved {
    pub(crate) fn apply(&self, session: &Session) -> Result<()> {
        if let Some(expected) = self.profile_id.as_deref() {
            let actual = session.profile_id();
            if actual.as_deref() != Some(expected) {
                return Err(config(format!(
                    "the session differs from the saved file: profile_id {actual:?} (saved {expected:?})"
                ))
                .with_source(ProfileChanged));
            }
        }
        session.restore(&self.state);
        Ok(())
    }
}

fn config(message: impl Into<String>) -> Error {
    Error::new(Cause::Config).with_message(message.into())
}
