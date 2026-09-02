//! Profile registry — loads and indexes browser profiles.

use std::collections::HashMap;
use std::fs::{read_dir, read_to_string};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::profile::Browser;
use crate::profile::types::BrowserProfile;

/// Why [`ProfileRegistry::load`] could not build a registry from a directory.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProfileError {
    /// A directory or file under the profile directory could not be read.
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// A `<family>/<version>.toml` file did not parse, or failed the same validation the built-in set runs.
    Parse {
        /// Path of the rejected file, absent when the profile came from a string.
        path: Option<PathBuf>,
        /// Parser or validator message.
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The directory held no `<family>/<version>.toml` file.
    Empty {
        /// Directory that was scanned.
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

/// Registry of all loaded browser profiles, indexed by (browser, version).
pub struct ProfileRegistry {
    profiles: HashMap<(String, u32), BrowserProfile>,
}

static BUILTIN: LazyLock<ProfileRegistry> = LazyLock::new(ProfileRegistry::builtin);

impl ProfileRegistry {
    /// The compiled-in profile set.
    #[must_use]
    pub fn global() -> &'static Self {
        &BUILTIN
    }

    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Load all built-in profiles (compiled in via include_str!).
    pub fn builtin() -> Self {
        let mut reg = Self::new();
        reg.load_toml(include_str!("../../profiles/chrome/145.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/146.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/147.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/148.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/149.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/150.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/151.toml"));
        reg.load_toml(include_str!("../../profiles/chrome/152.toml"));
        reg.load_toml(include_str!("../../profiles/brave/146.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/148.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/149.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/150.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/151.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/152.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/153.toml"));
        reg.load_toml(include_str!("../../profiles/firefox/154.toml"));
        reg.load_toml(include_str!("../../profiles/safari/18.toml"));
        reg.load_toml(include_str!("../../profiles/safari/26.toml"));
        reg.load_toml(include_str!("../../profiles/safari/ios17.toml"));
        reg.load_toml(include_str!("../../profiles/safari/ios18.toml"));
        reg.load_toml(include_str!("../../profiles/okhttp/android10.toml"));
        reg.load_toml(include_str!("../../profiles/cfnetwork/ios18.toml"));
        reg.load_toml(include_str!("../../profiles/cfnetwork/macos26.toml"));
        reg
    }

    /// Load a profile directory laid out as `<family>/<version>.toml` with the same parse and permutation validation as [`ProfileRegistry::builtin`]; fails with [`ProfileError`] on an unreadable directory, an invalid TOML, or no profiles.
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

    /// Parse and insert a TOML profile string.
    fn load_toml(&mut self, toml_str: &str) {
        let profile = BrowserProfile::from_toml(toml_str)
            .expect("built-in profile is statically valid (profile_validation)");
        let key = (profile.meta.browser.clone(), profile.meta.version);
        self.profiles.insert(key, profile);
    }

    /// Look up a profile by browser name and version.
    pub fn get(&self, browser: &str, version: u32) -> Option<&BrowserProfile> {
        self.profiles.get(&(browser.to_string(), version))
    }

    /// Look up a profile by Browser enum.
    pub fn get_browser(&self, browser: Browser) -> Option<&BrowserProfile> {
        let (name, version) = browser.profile_key();
        self.get(name, version)
    }

    /// Number of loaded profiles.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

/// Directory entries in a stable order, so two runs over the same directory load the same profiles.
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
