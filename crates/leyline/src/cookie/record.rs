use std::time::{Duration, SystemTime};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::util::domain_match;
use crate::util::epoch_plus;

const MAX_LIFETIME: Duration = Duration::from_secs(400 * 24 * 60 * 60);
pub(crate) const MAX_NAME_VALUE_OCTETS: usize = 4096;
pub(crate) const MAX_ATTRIBUTE_OCTETS: usize = 1024;
pub(crate) const DEFAULT_SAME_SITE: SameSite = SameSite::Lax;

/// The `SameSite` attribute of a cookie.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SameSite {
    /// Sends the cookie only on same-site requests.
    Strict,
    /// Sends the cookie on same-site requests and top-level cross-site navigations.
    #[default]
    Lax,
    /// Sends the cookie on all requests, including cross-site ones.
    None,
}

/// A stored cookie and its attributes.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cookie {
    pub(crate) name: String,
    pub(crate) value: String,
    pub(crate) domain: String,
    pub(crate) path: String,
    pub(crate) secure: bool,
    pub(crate) http_only: bool,
    pub(crate) same_site: SameSite,
    #[serde(with = "systime_opt_ms")]
    pub(crate) expires: Option<SystemTime>,
    #[serde(with = "systime_ms")]
    pub(crate) creation_time: SystemTime,
    #[serde(with = "systime_ms", default = "SystemTime::now")]
    pub(crate) last_access: SystemTime,
    pub(crate) host_only: bool,
}

/// Builds a `Cookie` for `Jar::insert` from a name, a value and the attributes set on it.
#[derive(Debug, Clone)]
#[must_use]
pub struct CookieBuilder {
    cookie: Cookie,
}

impl CookieBuilder {
    /// Sets the `Domain` attribute, so the cookie also matches subdomains of `domain`; without it the cookie matches only the host of the URL given to `Jar::insert`.
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.cookie.domain = domain.into();
        self.cookie.host_only = false;
        self
    }

    /// Sets the path the cookie applies to, `/` by default; a path that does not start with `/` becomes the default path of the URL given to `Jar::insert`.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.cookie.path = path.into();
        self
    }

    /// Sets whether the cookie is sent only to secure origins, off by default.
    pub fn secure(mut self, on: bool) -> Self {
        self.cookie.secure = on;
        self
    }

    /// Sets whether the cookie is marked as hidden from scripts, off by default.
    pub fn http_only(mut self, on: bool) -> Self {
        self.cookie.http_only = on;
        self
    }

    /// Sets the `SameSite` policy, `SameSite::Lax` by default.
    pub fn same_site(mut self, same_site: SameSite) -> Self {
        self.cookie.same_site = same_site;
        self
    }

    /// Sets the expiry time, which the jar caps at 400 days from now; `None`, the default, makes a session cookie.
    pub fn expires(mut self, at: impl Into<Option<SystemTime>>) -> Self {
        self.cookie.expires = at.into();
        self
    }

    /// Returns the cookie; `Jar::insert` checks it against its URL and ignores a cookie that URL may not set.
    pub fn build(self) -> Cookie {
        self.cookie
    }
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
    /// Creates a session cookie for `Jar::insert`: path `/`, no expiry, host-only, not secure, not HTTP-only, `SameSite::Lax`, with an empty domain that `insert` fills from its URL; `Cookie::builder` changes these.
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        let now = SystemTime::now();
        Self {
            name: name.into(),
            value: value.into(),
            domain: String::new(),
            path: "/".to_owned(),
            secure: false,
            http_only: false,
            same_site: DEFAULT_SAME_SITE,
            expires: None,
            creation_time: now,
            last_access: now,
            host_only: true,
        }
    }

    /// Returns a builder for a cookie with the attributes of `Cookie::new`, to change before `build`.
    pub fn builder(name: impl Into<String>, value: impl Into<String>) -> CookieBuilder {
        CookieBuilder {
            cookie: Self::new(name, value),
        }
    }

    /// Parses a `Set-Cookie` header value as if received from `url` without storing it, or returns `None` when `url` or `set_cookie` is invalid or rejected for `url`.
    #[must_use]
    pub fn parse(url: impl crate::IntoUrl, set_cookie: &str) -> Option<Self> {
        crate::cookie::parse::parse_set_cookie(set_cookie, &url.into_url().ok()?)
    }

    /// Returns the cookie name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the cookie value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the domain the cookie belongs to, which is empty on a hand-built cookie that set none.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Returns the path the cookie applies to.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns whether the cookie is sent only to secure origins.
    #[must_use]
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// Returns whether the cookie is marked as hidden from scripts.
    #[must_use]
    pub fn http_only(&self) -> bool {
        self.http_only
    }

    /// Returns the `SameSite` policy of the cookie.
    #[must_use]
    pub fn same_site(&self) -> SameSite {
        self.same_site
    }

    /// Returns the expiry time, or `None` for a session cookie.
    #[must_use]
    pub fn expires(&self) -> Option<SystemTime> {
        self.expires
    }

    /// Returns the time the cookie was first stored.
    #[must_use]
    pub fn creation_time(&self) -> SystemTime {
        self.creation_time
    }

    /// Returns whether the cookie matches only the exact host that set it, not its subdomains.
    #[must_use]
    pub fn host_only(&self) -> bool {
        self.host_only
    }

    pub(crate) fn within_limits(&self) -> bool {
        self.name.len() + self.value.len() <= MAX_NAME_VALUE_OCTETS
            && self.domain.len() <= MAX_ATTRIBUTE_OCTETS
    }

    /// Returns `true` when the cookie has an expiry time in the past.
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
        self.name == other.name && self.path == other.path
    }

    pub(crate) fn shadows_secure(&self, existing: &Cookie) -> bool {
        existing.secure
            && existing.name == self.name
            && (domain_match(&self.domain, &existing.domain)
                || domain_match(&existing.domain, &self.domain))
            && (path_match(&self.path, &existing.path) || path_match(&existing.path, &self.path))
    }
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
    i64::try_from(crate::util::since_epoch(*t).as_millis()).unwrap_or(i64::MAX)
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
