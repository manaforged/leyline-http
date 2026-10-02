use crate::util::atomic::write_atomic;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::cookie::Jar;
use crate::core::error::{Error, Kind, Result};
use crate::core::{Identity, ProxyConfig, ProxyUrl, Session, SessionBuilder, Tab};
use crate::profile::{BrowserProfile, ChromiumBrand, Platform};

mod autosave;
mod check;
mod secret;
mod state;

pub use autosave::DeviceAutosave;
pub use state::SessionState;
pub(crate) use state::{StateParts, unix_secs};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Device {
    #[serde(default)]
    pub identity: Option<Identity>,
    pub platform: Platform,
    #[serde(default)]
    pub brand: Option<ChromiumBrand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_toml: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub proxy: Option<ProxyUrl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_password_env: Option<String>,
    #[serde(default)]
    pub languages: Option<Vec<String>>,
    #[serde(default)]
    pub env_proxy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar: Option<Jar>,
    #[serde(default)]
    pub state: SessionState,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub app: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub strict: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Url>,
}

impl Device {
    #[must_use]
    pub fn capture(session: &Session, proxy: Option<ProxyUrl>) -> Device {
        let id = session.identity();
        Device {
            identity: id.to_identity(),
            platform: id.platform(),
            brand: id.brand(),
            profile_toml: None,
            profile_id: id.profile_id().map(str::to_owned),
            user_agent: id.identity().map(|_| id.user_agent().to_owned()),
            proxy,
            proxy_password_env: None,
            languages: id.languages().map(<[String]>::to_vec),
            env_proxy: false,
            jar_path: None,
            jar: None,
            state: session.state(),
            app: BTreeMap::new(),
            strict: false,
            page: None,
        }
    }

    pub fn pin_profile(&mut self, session: &Session) -> Result<()> {
        let id = session.identity();
        if id.to_identity().is_some_and(|i| i.http() != i.tls()) {
            return Err(config(
                "a session with a rotated TLS hello cannot pin one profile; keep the identity",
            ));
        }
        let toml = id
            .export_profile()
            .ok_or_else(|| config("the session has no profile source to pin"))?;
        self.profile_toml = Some(toml);
        self.profile_id = id.profile_id().map(str::to_owned);
        Ok(())
    }

    pub fn session_builder(&self) -> Result<SessionBuilder> {
        let mut builder = match (&self.profile_toml, self.identity) {
            (Some(toml), _) => Session::builder()
                .profile(BrowserProfile::from_toml(toml).map_err(|e| config(e.to_string()))?)
                .platform(self.platform),
            (None, Some(identity)) => Session::builder().identity(identity),
            (None, None) => Session::builder().platform(self.platform),
        };
        if let Some(brand) = self.brand {
            builder = builder.brand(brand);
        }
        builder = builder.proxy(self.proxy_config()?);
        if let Some(languages) = &self.languages {
            builder = builder.languages(languages);
        }
        Ok(builder.cookie_jar(self.load_jar()?))
    }

    pub fn open(&self) -> Result<Session> {
        let session = self.session_builder()?.build()?;
        self.check(&session)?;
        self.state.restore_into(&session);
        Ok(session)
    }

    #[must_use]
    pub fn tab(&self, session: &Session) -> Tab {
        let tab = session.tab();
        tab.set_current(self.page.clone());
        tab
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.sanitized()).map_err(Error::from_json)?;
        write_atomic(path.as_ref(), &bytes)
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Device> {
        let bytes = std::fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(Error::from_json)
    }

    fn proxy_config(&self) -> Result<ProxyConfig> {
        Ok(self
            .resolved_proxy()?
            .map_or_else(ProxyConfig::new, ProxyConfig::from)
            .env(self.env_proxy))
    }

    fn load_jar(&self) -> Result<Jar> {
        if let Some(jar) = &self.jar {
            return Ok(jar.clone());
        }
        match &self.jar_path {
            Some(path) if path.exists() => Jar::load_from(path),
            _ => Ok(Jar::new()),
        }
    }
}

fn config(message: impl Into<String>) -> Error {
    Error::new(Kind::Config).with_message(message.into())
}
