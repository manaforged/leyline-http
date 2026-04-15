//! Individual cookie — stores all attributes Chrome tracks.

use std::time::SystemTime;

/// SameSite attribute values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

/// A single cookie with all RFC 6265bis attributes.
#[derive(Debug, Clone)]
#[allow(dead_code)] // http_only, same_site, size() used when SameSite request filtering is wired
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    pub expires: Option<SystemTime>,
    pub creation_time: SystemTime,
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
