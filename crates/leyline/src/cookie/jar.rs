//! Cookie jar — Chrome-accurate storage, retrieval, ordering, and persistence.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::Url;

use crate::cookie::parse;
use crate::cookie::record::Cookie;

/// Chrome's per-domain cookie limit.
const MAX_COOKIES_PER_DOMAIN: usize = 180;
/// How many to evict when the per-domain limit is hit.
const EVICT_PER_DOMAIN: usize = 30;
/// Chrome's global cookie limit.
const MAX_COOKIES_GLOBAL: usize = 3300;
/// How many to evict when the global limit is hit.
const EVICT_GLOBAL: usize = 300;

/// Thread-safe cookie jar with Chrome-accurate behavior.
#[derive(Clone)]
pub struct Jar {
    inner: Arc<Mutex<JarInner>>,
}

struct JarInner {
    /// All cookies, keyed by registrable domain.
    cookies: HashMap<String, Vec<Cookie>>,
    /// Total cookie count across all domains.
    total: usize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Jar {
    /// Create an empty cookie jar.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(JarInner {
                cookies: HashMap::new(),
                total: 0,
            })),
        }
    }

    /// Fork an independent jar holding a deep copy of every cookie.
    pub fn deep_clone(&self) -> Self {
        let cookies = {
            let jar = lock(&self.inner);
            jar.cookies.clone()
        };
        let total = cookies.values().map(|v| v.len()).sum();
        Self {
            inner: Arc::new(Mutex::new(JarInner { cookies, total })),
        }
    }

    /// Store a Set-Cookie header from a response.
    pub fn store_set_cookie(&self, header: &str, url: &Url) {
        let mut cookie = match parse::parse_set_cookie(header, url) {
            Some(c) => c,
            None => return,
        };

        if cookie.is_expired() {
            let mut jar = lock(&self.inner);
            let domain = cookie.domain.to_lowercase();
            if let Some(entries) = jar.cookies.get_mut(&domain)
                && let Some(pos) = entries
                    .iter()
                    .position(|c| c.name == cookie.name && c.path == cookie.path)
            {
                if entries[pos].secure && url.scheme() != "https" {
                    return;
                }
                entries.remove(pos);
                jar.total -= 1;
            }
            return;
        }

        let mut jar = lock(&self.inner);
        let domain = cookie.domain.to_lowercase();

        let entries = jar.cookies.entry(domain.clone()).or_default();

        let mut added = false;
        if let Some(pos) = entries
            .iter()
            .position(|c| c.name == cookie.name && c.path == cookie.path)
        {
            if entries[pos].secure && !cookie.secure {
                return;
            }
            cookie.creation_time = entries[pos].creation_time;
            entries[pos] = cookie;
        } else {
            entries.push(cookie);
            added = true;
        }

        let mut evicted = 0;
        if entries.len() > MAX_COOKIES_PER_DOMAIN {
            evict_lru(entries, EVICT_PER_DOMAIN);
            evicted = EVICT_PER_DOMAIN;
        }

        if added {
            jar.total += 1;
        }
        jar.total -= evicted;

        if jar.total > MAX_COOKIES_GLOBAL {
            evict_global(&mut jar.cookies, EVICT_GLOBAL);
            jar.total = jar.cookies.values().map(|v| v.len()).sum();
        }
    }

    /// Store multiple Set-Cookie headers from a response.
    pub fn store_response_cookies(&self, headers: &[&str], url: &Url) {
        for header in headers {
            self.store_set_cookie(header, url);
        }
    }

    /// Build the Cookie header value for a request.
    pub fn cookie_header(&self, url: &Url) -> Option<String> {
        self.cookie_header_for(url, false, true)
    }

    /// Build the Cookie header, enforcing SameSite for the request's cross-site context.
    pub(crate) fn cookie_header_for(
        &self,
        url: &Url,
        cross_site: bool,
        safe_method: bool,
    ) -> Option<String> {
        use crate::cookie::record::SameSite;
        let domain = url.host_str().unwrap_or("");
        let path = url.path();
        let is_secure = url.scheme() == "https";

        let mut jar = lock(&self.inner);
        let now = SystemTime::now();

        let mut matching: Vec<&mut Cookie> = Vec::with_capacity(8);
        for entries in jar.cookies.values_mut() {
            for cookie in entries.iter_mut() {
                if cookie.is_expired() {
                    continue;
                }
                if !cookie.matches(domain, path, is_secure) {
                    continue;
                }
                if cross_site {
                    match cookie.same_site {
                        SameSite::Strict => continue,
                        SameSite::Lax if !safe_method => continue,
                        _ => {}
                    }
                }
                cookie.last_access = now;
                matching.push(cookie);
            }
        }

        if matching.is_empty() {
            return None;
        }

        matching.sort_by(|a, b| {
            b.path
                .len()
                .cmp(&a.path.len())
                .then(a.creation_time.cmp(&b.creation_time))
        });

        let header = matching
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; ");

        Some(header)
    }

    /// Look up a cookie value by name without a URL filter.
    pub fn get_named(&self, name: &str) -> Option<String> {
        let jar = lock(&self.inner);
        for entries in jar.cookies.values() {
            for cookie in entries {
                if cookie.name == name && !cookie.is_expired() {
                    return Some(cookie.value.clone());
                }
            }
        }
        None
    }

    /// True if any non-expired cookie with `name` exists in the jar.
    pub fn contains_named(&self, name: &str) -> bool {
        self.get_named(name).is_some()
    }

    /// Update the value of every cookie matching `name`, across every domain and path the jar holds.
    pub fn set_named(&self, name: &str, value: &str) -> bool {
        let mut jar = lock(&self.inner);
        let now = SystemTime::now();
        let mut updated = false;
        for entries in jar.cookies.values_mut() {
            for cookie in entries.iter_mut() {
                if cookie.name == name {
                    cookie.value = value.to_string();
                    cookie.last_access = now;
                    updated = true;
                }
            }
        }
        updated
    }

    /// Upsert a cookie on a specific domain.
    pub fn set_named_on(&self, domain: &str, name: &str, value: &str) {
        let mut jar = lock(&self.inner);
        let key = domain.to_lowercase();
        let entries = jar.cookies.entry(key.clone()).or_default();
        let now = SystemTime::now();
        let mut updated = false;
        for c in entries.iter_mut() {
            if c.name == name {
                c.value = value.to_string();
                c.last_access = now;
                updated = true;
            }
        }
        if updated {
            return;
        }
        entries.push(Cookie {
            name: name.to_string(),
            value: value.to_string(),
            domain: key,
            path: "/".to_string(),
            secure: false,
            http_only: false,
            same_site: crate::cookie::record::SameSite::Lax,
            expires: None,
            creation_time: now,
            last_access: now,
            host_only: true,
        });
        jar.total += 1;
    }

    /// Remove the first cookie matching `name` (any domain).
    pub fn remove_named(&self, name: &str) -> bool {
        let mut jar = lock(&self.inner);
        for entries in jar.cookies.values_mut() {
            if let Some(pos) = entries.iter().position(|c| c.name == name) {
                entries.remove(pos);
                jar.total -= 1;
                return true;
            }
        }
        false
    }

    /// Remove every cookie matching `name` across every domain.
    pub fn remove_all_named(&self, name: &str) -> usize {
        let mut jar = lock(&self.inner);
        let mut removed = 0;
        for entries in jar.cookies.values_mut() {
            let before = entries.len();
            entries.retain(|c| c.name != name);
            removed += before - entries.len();
        }
        jar.total -= removed;
        removed
    }

    /// Remove cookies named `name` whose domain is `host` itself or a parent suffix of it (e.g. removing for `store.example.com` also clears a stale entry mis-hosted on `example.com`), while preserving same-named cookies on sibling hosts like `www.example.com`.
    pub fn remove_named_for_host(&self, host: &str, name: &str) -> usize {
        let mut jar = lock(&self.inner);
        let host = host.to_lowercase();
        let mut removed = 0;
        for (domain, entries) in jar.cookies.iter_mut() {
            if host == *domain || host.ends_with(&format!(".{domain}")) {
                let before = entries.len();
                entries.retain(|c| c.name != name);
                removed += before - entries.len();
            }
        }
        jar.total -= removed;
        removed
    }

    /// Snapshot every cookie in the jar, sorted by domain then name.
    pub fn all_cookies(&self) -> Vec<Cookie> {
        let jar = lock(&self.inner);
        let mut out: Vec<Cookie> = jar.cookies.values().flatten().cloned().collect();
        out.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Merge cookies from another jar into this one.
    pub fn merge(&self, other: &Jar) {
        let snapshots: Vec<Cookie> = {
            let other_inner = lock(&other.inner);
            other_inner.cookies.values().flatten().cloned().collect()
        };
        let mut jar = lock(&self.inner);
        for c in snapshots {
            if c.is_expired() {
                continue;
            }
            let key = c.domain.to_lowercase();
            let entries = jar.cookies.entry(key).or_default();
            if let Some(pos) = entries
                .iter()
                .position(|e| e.name == c.name && e.path == c.path)
            {
                entries[pos] = c;
            } else {
                entries.push(c);
                jar.total += 1;
            }
        }
        let mut evicted = 0;
        for entries in jar.cookies.values_mut() {
            if entries.len() > MAX_COOKIES_PER_DOMAIN {
                evict_lru(entries, EVICT_PER_DOMAIN);
                evicted += EVICT_PER_DOMAIN;
            }
        }
        jar.total = jar.total.saturating_sub(evicted);
        if jar.total > MAX_COOKIES_GLOBAL {
            evict_global(&mut jar.cookies, EVICT_GLOBAL);
            jar.total = jar.cookies.values().map(|v| v.len()).sum();
        }
    }

    /// Get a single cookie value by name for a URL.
    pub fn get_cookie(&self, url: &str, name: &str) -> Option<String> {
        let url = Url::parse(url).ok()?;
        let domain = url.host_str().unwrap_or("");
        let path = url.path();
        let is_secure = url.scheme() == "https";

        let jar = lock(&self.inner);
        for entries in jar.cookies.values() {
            for cookie in entries {
                if cookie.name == name
                    && cookie.matches(domain, path, is_secure)
                    && !cookie.is_expired()
                {
                    return Some(cookie.value.clone());
                }
            }
        }
        None
    }

    /// Set a cookie manually (convenience for testing/setup).
    pub fn set_cookie(&self, url: &str, name: &str, value: &str) {
        let parsed_url = match Url::parse(url) {
            Ok(u) => u,
            Err(_) => return,
        };
        let header = format!("{}={}; Path=/", name, value);
        self.store_set_cookie(&header, &parsed_url);
    }

    /// Load cookies from a Cookie header string (e.g., "a=1; b=2").
    pub fn load_cookies(&self, cookie_str: &str, raw_url: &str) {
        let url = match Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => return,
        };
        for pair in cookie_str.split(';') {
            let pair = pair.trim();
            if let Some(eq) = pair.find('=') {
                let name = &pair[..eq];
                let value = &pair[eq + 1..];
                let header = format!("{}={}; Path=/", name, value);
                self.store_set_cookie(&header, &url);
            }
        }
    }

    /// Export cookies for a URL as a Cookie header string.
    pub fn export_cookies(&self, raw_url: &str) -> String {
        let url = match Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => return String::new(),
        };
        self.cookie_header(&url).unwrap_or_default()
    }

    /// Remove every cookie from the jar.
    pub fn clear(&self) {
        let mut jar = lock(&self.inner);
        jar.cookies.clear();
        jar.total = 0;
    }

    /// True if the jar holds no cookies.
    pub fn is_empty(&self) -> bool {
        lock(&self.inner).total == 0
    }

    /// Number of cookies currently in the jar.
    pub fn len(&self) -> usize {
        lock(&self.inner).total
    }
}

impl Default for Jar {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Jar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let jar = lock(&self.inner);
        f.debug_struct("Jar")
            .field("domains", &jar.cookies.len())
            .field("total", &jar.total)
            .finish()
    }
}

impl Serialize for Jar {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let jar = lock(&self.inner);
        let mut flat: Vec<&Cookie> = jar.cookies.values().flatten().collect();
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
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let cookies: Vec<Cookie> = Vec::deserialize(de)?;
        let mut buckets: HashMap<String, Vec<Cookie>> = HashMap::new();
        let mut total = 0;
        for c in cookies {
            let key = c.domain.to_lowercase();
            buckets.entry(key).or_default().push(c);
            total += 1;
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(JarInner {
                cookies: buckets,
                total,
            })),
        })
    }
}

/// Evict the N least-recently-accessed cookies from a domain's list.
fn evict_lru(cookies: &mut Vec<Cookie>, count: usize) {
    cookies.sort_by_key(|a| a.last_access);
    cookies.drain(..count.min(cookies.len()));
}

/// Evict N cookies globally, targeting least-recently-accessed.
fn evict_global(all: &mut HashMap<String, Vec<Cookie>>, count: usize) {
    let mut all_cookies: Vec<(String, usize, SystemTime)> = Vec::new();
    for (domain, entries) in all.iter() {
        for (i, cookie) in entries.iter().enumerate() {
            all_cookies.push((domain.clone(), i, cookie.last_access));
        }
    }
    all_cookies.sort_by_key(|a| a.2);

    let to_remove = count.min(all_cookies.len());
    let mut removals: HashMap<String, Vec<usize>> = HashMap::new();
    for (domain, idx, _) in &all_cookies[..to_remove] {
        removals.entry(domain.clone()).or_default().push(*idx);
    }
    for (domain, mut indices) in removals {
        indices.sort_unstable_by(|a, b| b.cmp(a));
        if let Some(entries) = all.get_mut(&domain) {
            for idx in indices {
                if idx < entries.len() {
                    entries.remove(idx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
