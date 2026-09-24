use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    #[serde(with = "systime_opt_ms")]
    pub expires: Option<SystemTime>,
    #[serde(with = "systime_ms")]
    pub creation_time: SystemTime,
    #[serde(skip, default = "SystemTime::now")]
    pub(crate) last_access: SystemTime,
    pub host_only: bool,
}

impl Cookie {
    pub fn is_expired(&self) -> bool {
        if let Some(expires) = self.expires {
            SystemTime::now() > expires
        } else {
            false
        }
    }

    pub(crate) fn matches(&self, url_domain: &str, url_path: &str, is_secure: bool) -> bool {
        if self.secure && !is_secure {
            return false;
        }

        if self.host_only {
            if !url_domain.eq_ignore_ascii_case(&self.domain) {
                return false;
            }
        } else {
            let cookie_domain = self.domain.to_lowercase();
            let req_domain = url_domain.to_lowercase();
            if req_domain != cookie_domain && !req_domain.ends_with(&format!(".{}", cookie_domain))
            {
                return false;
            }
        }

        if url_path == self.path {
            return true;
        }
        if url_path.starts_with(&self.path) {
            if self.path.ends_with('/') {
                return true;
            }
            if url_path.as_bytes().get(self.path.len()) == Some(&b'/') {
                return true;
            }
        }

        false
    }
}

mod systime_ms {
    use super::*;

    pub(super) fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let ms = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        s.serialize_i64(ms)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let ms = i64::deserialize(d)?;
        let ms = if ms < 0 { 0 } else { ms as u64 };
        Ok(UNIX_EPOCH + Duration::from_millis(ms))
    }
}

mod systime_opt_ms {
    use super::*;

    pub(super) fn serialize<S: Serializer>(
        t: &Option<SystemTime>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => {
                let ms = t
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                s.serialize_some(&ms)
            }
            None => s.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<SystemTime>, D::Error> {
        let opt = Option::<i64>::deserialize(d)?;
        Ok(opt.map(|ms| {
            let ms = if ms < 0 { 0 } else { ms as u64 };
            UNIX_EPOCH + Duration::from_millis(ms)
        }))
    }
}
