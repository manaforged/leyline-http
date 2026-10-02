use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::util::epoch_plus;

const MAX_LIFETIME: Duration = Duration::from_secs(400 * 24 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

#[derive(Clone, Serialize, Deserialize)]
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
    #[serde(with = "systime_ms", default = "SystemTime::now")]
    pub(crate) last_access: SystemTime,
    pub host_only: bool,
}

impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("value", &"***")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("same_site", &self.same_site)
            .field("expires", &self.expires)
            .field("host_only", &self.host_only)
            .finish_non_exhaustive()
    }
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
        } else if !domain_match(url_domain, &self.domain) {
            return false;
        }
        path_match(url_path, &self.path)
    }

    pub(crate) fn same_slot(&self, other: &Cookie) -> bool {
        self.name == other.name && self.path == other.path && self.host_only == other.host_only
    }

    pub(crate) fn shadows_secure(&self, existing: &Cookie) -> bool {
        existing.secure
            && existing.name == self.name
            && (domain_match(&self.domain, &existing.domain)
                || domain_match(&existing.domain, &self.domain))
            && (path_match(&self.path, &existing.path) || path_match(&existing.path, &self.path))
    }
}

fn domain_match(host: &str, domain: &str) -> bool {
    let host = host.to_lowercase();
    let domain = domain.to_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn path_match(request_path: &str, cookie_path: &str) -> bool {
    if request_path == cookie_path {
        return true;
    }
    request_path.starts_with(cookie_path)
        && (cookie_path.ends_with('/')
            || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/'))
}

pub(crate) fn capped_expiry(now: SystemTime, requested: Option<SystemTime>) -> SystemTime {
    let cap = now + MAX_LIFETIME;
    requested.map_or(cap, |at| at.min(cap))
}

fn from_unix_millis(ms: i64) -> Option<SystemTime> {
    let ms = if ms < 0 { 0 } else { ms as u64 };
    epoch_plus(Duration::from_millis(ms))
}

fn unix_millis(t: &SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
    })
}

mod systime_ms {
    use super::*;

    pub(super) fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(unix_millis(t))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let ms = i64::deserialize(d)?;
        Ok(from_unix_millis(ms).unwrap_or_else(SystemTime::now))
    }
}

mod systime_opt_ms {
    use super::*;

    pub(super) fn serialize<S: Serializer>(
        t: &Option<SystemTime>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => s.serialize_some(&unix_millis(t)),
            None => s.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<SystemTime>, D::Error> {
        let opt = Option::<i64>::deserialize(d)?;
        Ok(opt.map(|ms| capped_expiry(SystemTime::now(), from_unix_millis(ms))))
    }
}
