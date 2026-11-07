use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cookie::Cookie;
use crate::error::{Error, Result};
use crate::util::lock;
use crate::util::version::{self, Versioned};

use super::Jar;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum JarVersion {
    #[default]
    V1,
}

impl Versioned for JarVersion {
    fn number(self) -> u32 {
        match self {
            JarVersion::V1 => 1,
        }
    }

    fn from_number(number: u32) -> Option<Self> {
        match number {
            1 => Some(JarVersion::V1),
            _ => None,
        }
    }
}

impl Jar {
    pub(crate) fn load_from(path: impl AsRef<Path>) -> Result<Jar> {
        let bytes = std::fs::read(path)?;
        let file: JarFileIn = serde_json::from_slice(&bytes).map_err(Error::from_json)?;
        match file.version {
            JarVersion::V1 => Ok(file.cookies),
        }
    }
}

#[derive(Deserialize)]
struct JarFileIn {
    #[serde(with = "version")]
    version: JarVersion,
    cookies: Jar,
}

impl Serialize for Jar {
    fn serialize<S: Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
        let jar = lock(&self.inner);
        let mut flat: Vec<&Cookie> = jar
            .cookies
            .values()
            .flatten()
            .filter(|c| !c.is_expired())
            .collect();
        flat.sort_by(|a, b| {
            a.domain
                .cmp(&b.domain)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.creation_time.cmp(&b.creation_time))
        });
        flat.serialize(ser)
    }
}

impl<'de> Deserialize<'de> for Jar {
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        let cookies: Vec<Cookie> = Vec::deserialize(de)?;
        let jar = Jar::new();
        {
            let mut inner = lock(&jar.inner);
            for cookie in cookies.into_iter().filter(|c| !c.is_expired()) {
                inner.insert_loaded(cookie);
            }
        }
        Ok(jar)
    }
}

#[cfg(test)]
mod tests;
