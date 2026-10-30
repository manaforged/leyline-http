use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cookie::record::Cookie;
use crate::core::saved::FileVersion;
use crate::core::{Error, Result};
use crate::util::lock;

use super::Jar;

impl Jar {
    pub(crate) fn load_from(path: impl AsRef<Path>) -> Result<Jar> {
        let bytes = std::fs::read(path)?;
        let file: JarFileIn = serde_json::from_slice(&bytes).map_err(Error::from_json)?;
        match file.version {
            FileVersion::V1 => Ok(file.cookies),
        }
    }
}

#[derive(Deserialize)]
struct JarFileIn {
    version: FileVersion,
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
