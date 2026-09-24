use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use url::Url;

use crate::cookie::parse;
use crate::cookie::record::Cookie;
use crate::core::{IntoUrl, Result};

const MAX_COOKIES_PER_DOMAIN: usize = 180;
const EVICT_PER_DOMAIN: usize = 30;
const MAX_COOKIES_GLOBAL: usize = 3300;
const EVICT_GLOBAL: usize = 300;

#[derive(Clone)]
pub struct Jar {
    inner: Arc<Mutex<JarInner>>,
}

struct JarInner {
    cookies: HashMap<String, Vec<Cookie>>,
    total: usize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Jar {
    pub fn new() -> Self {
        Self::from_buckets(HashMap::new(), 0)
    }

    fn from_buckets(cookies: HashMap<String, Vec<Cookie>>, total: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(JarInner { cookies, total })),
        }
    }

    pub fn snapshot(&self) -> Self {
        let jar = lock(&self.inner);
        Self::from_buckets(jar.cookies.clone(), jar.total)
    }

    pub fn extend_from(&self, other: &Jar) {
        if Arc::ptr_eq(&self.inner, &other.inner) {
            return;
        }
        let incoming: Vec<Cookie> = lock(&other.inner)
            .cookies
            .values()
            .flatten()
            .cloned()
            .collect();
        let mut jar = lock(&self.inner);
        for cookie in incoming {
            let domain = cookie.domain.to_lowercase();
            let entries = jar.cookies.entry(domain.clone()).or_default();
            let added = match entries
                .iter()
                .position(|c| c.name == cookie.name && c.path == cookie.path)
            {
                Some(pos) => {
                    entries[pos] = cookie;
                    false
                }
                None => {
                    entries.push(cookie);
                    true
                }
            };
            settle(&mut jar, &domain, added);
        }
    }

    pub fn store_set_cookie(&self, set_cookie: &str, url: impl IntoUrl) -> Result<()> {
        self.store(set_cookie, &url.into_url()?);
        Ok(())
    }

    fn store(&self, set_cookie: &str, url: &Url) {
        let mut cookie = match parse::parse_set_cookie(set_cookie, url) {
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
        settle(&mut jar, &domain, added);
    }

    pub(crate) fn store_response_cookies(&self, headers: &[&str], url: &Url) {
        for header in headers {
            self.store(header, url);
        }
    }

    pub fn cookie_header(&self, url: &Url) -> Option<String> {
        self.cookie_header_for(url, false, true)
    }

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

    pub fn remove_named(&self, name: &str) -> usize {
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

    pub fn remove(&self, url: impl IntoUrl, name: &str) -> Result<usize> {
        let url = url.into_url()?;
        let host = url.host_str().unwrap_or("");
        let mut jar = lock(&self.inner);
        let mut removed = 0;
        for entries in jar.cookies.values_mut() {
            let before = entries.len();
            entries.retain(|c| c.name != name || !c.matches(host, &c.path, true));
            removed += before - entries.len();
        }
        jar.total -= removed;
        Ok(removed)
    }

    pub fn all_cookies(&self) -> Vec<Cookie> {
        let jar = lock(&self.inner);
        let mut out: Vec<Cookie> = jar.cookies.values().flatten().cloned().collect();
        out.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
        out
    }

    pub fn get_cookie(&self, url: impl IntoUrl, name: &str) -> Result<Option<String>> {
        let url = url.into_url()?;
        let domain = url.host_str().unwrap_or("");
        let path = url.path();
        let is_secure = url.scheme() == "https";

        let jar = lock(&self.inner);
        Ok(jar
            .cookies
            .values()
            .flatten()
            .find(|c| c.name == name && c.matches(domain, path, is_secure) && !c.is_expired())
            .map(|c| c.value.clone()))
    }

    pub fn set_cookie(&self, url: impl IntoUrl, name: &str, value: &str) -> Result<()> {
        self.store(&format!("{}={}; Path=/", name, value), &url.into_url()?);
        Ok(())
    }

    pub fn load_cookies(&self, cookie_str: &str, url: impl IntoUrl) -> Result<()> {
        let url = url.into_url()?;
        for pair in cookie_str.split(';') {
            if let Some((name, value)) = pair.trim().split_once('=') {
                self.store(&format!("{}={}; Path=/", name, value), &url);
            }
        }
        Ok(())
    }

    pub fn export_cookies(&self, url: impl IntoUrl) -> Result<String> {
        Ok(self.cookie_header(&url.into_url()?).unwrap_or_default())
    }

    pub fn clear(&self) {
        let mut jar = lock(&self.inner);
        jar.cookies.clear();
        jar.total = 0;
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
    fn serialize<S: Serializer>(&self, ser: S) -> std::result::Result<S::Ok, S::Error> {
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
    fn deserialize<D: Deserializer<'de>>(de: D) -> std::result::Result<Self, D::Error> {
        let cookies: Vec<Cookie> = Vec::deserialize(de)?;
        let mut buckets: HashMap<String, Vec<Cookie>> = HashMap::new();
        let mut total = 0;
        for c in cookies {
            let key = c.domain.to_lowercase();
            buckets.entry(key).or_default().push(c);
            total += 1;
        }
        Ok(Self::from_buckets(buckets, total))
    }
}

fn settle(jar: &mut JarInner, domain: &str, added: bool) {
    let mut evicted = 0;
    if let Some(entries) = jar.cookies.get_mut(domain)
        && entries.len() > MAX_COOKIES_PER_DOMAIN
    {
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

fn evict_lru(cookies: &mut Vec<Cookie>, count: usize) {
    cookies.sort_by_key(|a| a.last_access);
    cookies.drain(..count.min(cookies.len()));
}

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
