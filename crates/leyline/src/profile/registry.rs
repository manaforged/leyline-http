use std::collections::HashMap;
use std::fs::{read_dir, read_to_string};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::profile::Browser;
use crate::profile::types::BrowserProfile;

#[derive(Debug)]
#[non_exhaustive]
pub enum ProfileError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Parse {
        path: Option<PathBuf>,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    Empty {
        path: PathBuf,
    },
}

impl ProfileError {
    pub(crate) fn parse(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Parse {
            path: None,
            source: source.into(),
        }
    }

    fn at(self, file: &Path) -> Self {
        match self {
            Self::Parse { source, .. } => Self::Parse {
                path: Some(file.to_path_buf()),
                source,
            },
            other => other,
        }
    }
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::Parse { path, source } => match path {
                Some(path) => write!(f, "invalid profile {}: {source}", path.display()),
                None => write!(f, "invalid profile: {source}"),
            },
            Self::Empty { path } => write!(
                f,
                "{} holds no <family>/<version>.toml profile; a profile directory contains one \
                 subdirectory per family, each with one TOML file per version",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ProfileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source.as_ref()),
            Self::Empty { .. } => None,
        }
    }
}

pub struct ProfileRegistry {
    profiles: HashMap<(String, u32), BrowserProfile>,
}

static BUILTIN: LazyLock<ProfileRegistry> = LazyLock::new(ProfileRegistry::builtin);

impl ProfileRegistry {
    #[must_use]
    pub fn global() -> &'static Self {
        &BUILTIN
    }

    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    pub fn builtin() -> Self {
        let mut reg = Self::new();
        for browser in Browser::all() {
            reg.load_toml(browser.profile_source());
        }
        reg
    }

    pub fn load(dir: &Path) -> Result<Self, ProfileError> {
        let mut reg = Self::new();
        for family in sorted(dir)? {
            if !family.is_dir() {
                continue;
            }
            for file in sorted(&family)? {
                if file.extension().is_none_or(|ext| ext != "toml") {
                    continue;
                }
                let text = read_to_string(&file).map_err(|source| ProfileError::Io {
                    path: file.clone(),
                    source,
                })?;
                let profile = BrowserProfile::from_toml(&text).map_err(|e| e.at(&file))?;
                let key = (profile.meta.browser.clone(), profile.meta.version);
                drop(reg.profiles.insert(key, profile));
            }
        }
        if reg.is_empty() {
            return Err(ProfileError::Empty {
                path: dir.to_path_buf(),
            });
        }
        Ok(reg)
    }

    fn load_toml(&mut self, toml_str: &str) {
        let profile = BrowserProfile::from_toml(toml_str)
            .expect("built-in profile is statically valid (profile_validation)");
        let key = (profile.meta.browser.clone(), profile.meta.version);
        self.profiles.insert(key, profile);
    }

    pub fn get(&self, browser: &str, version: u32) -> Option<&BrowserProfile> {
        self.profiles.get(&(browser.to_string(), version))
    }

    pub fn get_browser(&self, browser: Browser) -> Option<&BrowserProfile> {
        let (name, version) = browser.profile_key();
        self.get(name, version)
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

fn sorted(dir: &Path) -> Result<Vec<PathBuf>, ProfileError> {
    let entries = read_dir(dir).map_err(|source| ProfileError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ProfileError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        paths.push(entry.path());
    }
    paths.sort();
    Ok(paths)
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests;
