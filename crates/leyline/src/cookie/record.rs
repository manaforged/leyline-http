//! Individual cookie — stores all attributes Chrome tracks.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// SameSite attribute values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SameSite {
    /// Cookie sent only on same-site requests, never on cross-site.
    Strict,
    /// Cookie sent on same-site requests and top-level cross-site GET navigations.
    Lax,
    /// Cookie sent on every request, including cross-site. Requires `Secure`.
    None,
}

/// A single cookie with all RFC 6265bis attributes.
///
/// Serializable for jar persistence. `creation_time` and `expires` round-trip
/// as unix-millis to keep the on-disk format stable across platforms.
/// `last_access` is a runtime-only LRU bookkeeping field — it skips
/// serialization and resets to "now" on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    /// Cookie name (left side of `name=value`).
    pub name: String,
    /// Cookie value (right side of `name=value`).
    pub value: String,
    /// Registrable domain the cookie is scoped to.
    pub domain: String,
    /// URL path the cookie applies to (RFC 6265bis Section 5.1.4).
    pub path: String,
    /// `Secure` attribute — cookie only sent over HTTPS.
    pub secure: bool,
    /// `HttpOnly` attribute — cookie inaccessible to JavaScript.
    pub http_only: bool,
    /// `SameSite` attribute controlling cross-site request behavior.
    pub same_site: SameSite,
    /// Absolute expiry time, or `None` for session cookies.
    #[serde(with = "systime_opt_ms")]
    pub expires: Option<SystemTime>,
    /// When the cookie was first created. Preserved across same-name replacements.
    #[serde(with = "systime_ms")]
    pub creation_time: SystemTime,
    /// LRU bookkeeping. Skipped on serialize; reset to `now()` on load.
    #[serde(skip, default = "SystemTime::now")]
    pub last_access: SystemTime,
    /// Whether the domain was explicitly set (vs defaulting to request host).
    pub host_only: bool,
}

impl Cookie {
    /// Whether this cookie has expired.
    pub fn is_expired(&self) -> bool {
        if let Some(expires) = self.expires {
            SystemTime::now() > expires
        } else {
            false // session cookie — never expires in storage
        }
    }

    /// Size in bytes (name + value, used for size limit enforcement).
    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        self.name.len() + self.value.len()
    }

    /// Whether this cookie matches a request URL.
    pub fn matches(&self, url_domain: &str, url_path: &str, is_secure: bool) -> bool {
        // Secure check.
        if self.secure && !is_secure {
            return false;
        }

        // Domain check.
        if self.host_only {
            if !url_domain.eq_ignore_ascii_case(&self.domain) {
                return false;
            }
        } else {
            // Domain match: url_domain == domain or ends with .domain
            let cookie_domain = self.domain.to_lowercase();
            let req_domain = url_domain.to_lowercase();
            if req_domain != cookie_domain && !req_domain.ends_with(&format!(".{}", cookie_domain))
            {
                return false;
            }
        }

        // Path match (RFC 6265bis Section 5.1.4).
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

/// Serde helper: `SystemTime` ↔ unix-millis i64.
mod systime_ms {
    use super::*;

    pub fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let ms = t
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        s.serialize_i64(ms)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
        let ms = i64::deserialize(d)?;
        let ms = if ms < 0 { 0 } else { ms as u64 };
        Ok(UNIX_EPOCH + Duration::from_millis(ms))
    }
}

/// Serde helper: `Option<SystemTime>` ↔ optional unix-millis i64.
mod systime_opt_ms {
    use super::*;

    pub fn serialize<S: Serializer>(t: &Option<SystemTime>, s: S) -> Result<S::Ok, S::Error> {
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

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<SystemTime>, D::Error> {
        let opt = Option::<i64>::deserialize(d)?;
        Ok(opt.map(|ms| {
            let ms = if ms < 0 { 0 } else { ms as u64 };
            UNIX_EPOCH + Duration::from_millis(ms)
        }))
    }
}
