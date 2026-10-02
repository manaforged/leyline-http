use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tokio::sync::watch;
use url::Url;

use crate::cookie::parse;
use crate::cookie::record::Cookie;
use crate::util::lock;

use self::evict::settle;

mod autosave;
mod evict;
mod persist;

pub use autosave::JarAutosave;

#[derive(Clone)]
pub struct Jar {
    inner: Arc<Mutex<JarInner>>,
    changes: Arc<watch::Sender<u64>>,
}

struct JarInner {
    cookies: HashMap<String, Vec<Cookie>>,
    total: usize,
}

impl Jar {
    pub fn new() -> Self {
        Self::from_buckets(HashMap::new(), 0)
    }

    fn from_buckets(cookies: HashMap<String, Vec<Cookie>>, total: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(JarInner { cookies, total })),
            changes: Arc::new(watch::Sender::new(0)),
        }
    }

    pub(crate) fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn changed(&self) {
        self.changes.send_modify(|generation| *generation += 1);
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
        if incoming.is_empty() {
            return;
        }
        let mut jar = lock(&self.inner);
        for cookie in incoming {
            let domain = cookie.domain.to_lowercase();
            let entries = jar.cookies.entry(domain.clone()).or_default();
            let added = match entries.iter().position(|c| c.same_slot(&cookie)) {
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
        drop(jar);
        self.changed();
    }

    pub fn store_set_cookie(&self, set_cookie: &str, url: &Url) {
        self.store(set_cookie, url);
    }

    fn store(&self, set_cookie: &str, url: &Url) {
        if self.apply(set_cookie, url) {
            self.changed();
        }
    }

    fn apply(&self, set_cookie: &str, url: &Url) -> bool {
        let Some(mut cookie) = parse::parse_set_cookie(set_cookie, url) else {
            return false;
        };
        if !cookie.secure
            && url.scheme() != "https"
            && lock(&self.inner)
                .cookies
                .values()
                .flatten()
                .any(|existing| cookie.shadows_secure(existing))
        {
            return false;
        }

        if cookie.is_expired() {
            let mut jar = lock(&self.inner);
            let domain = cookie.domain.to_lowercase();
            if let Some(entries) = jar.cookies.get_mut(&domain)
                && let Some(pos) = entries.iter().position(|c| c.same_slot(&cookie))
            {
                if entries[pos].secure && url.scheme() != "https" {
                    return false;
                }
                return jar.remove_in(&domain, |c| c.same_slot(&cookie)) > 0;
            }
            return false;
        }

        let mut jar = lock(&self.inner);
        let domain = cookie.domain.to_lowercase();

        let entries = jar.cookies.entry(domain.clone()).or_default();

        let mut added = false;
        if let Some(pos) = entries.iter().position(|c| c.same_slot(&cookie)) {
            if entries[pos].secure && !cookie.secure {
                return false;
            }
            cookie.creation_time = entries[pos].creation_time;
            entries[pos] = cookie;
        } else {
            entries.push(cookie);
            added = true;
        }
        settle(&mut jar, &domain, added);
        true
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
        lax_allowed: bool,
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
                        SameSite::Lax if !lax_allowed => continue,
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
        let removed = lock(&self.inner).remove_where(|c| c.name == name);
        self.changed_if(removed)
    }

    pub fn remove(&self, url: &Url, name: &str) -> usize {
        let host = url.host_str().unwrap_or("");
        let removed =
            lock(&self.inner).remove_where(|c| c.name == name && c.matches(host, &c.path, true));
        self.changed_if(removed)
    }

    fn changed_if(&self, removed: usize) -> usize {
        if removed > 0 {
            self.changed();
        }
        removed
    }

    pub fn all_cookies(&self) -> Vec<Cookie> {
        let jar = lock(&self.inner);
        let mut out: Vec<Cookie> = jar.cookies.values().flatten().cloned().collect();
        out.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
        out
    }

    pub fn get_cookie(&self, url: &Url, name: &str) -> Option<String> {
        let domain = url.host_str().unwrap_or("");
        let path = url.path();
        let is_secure = url.scheme() == "https";

        let jar = lock(&self.inner);
        jar.cookies
            .values()
            .flatten()
            .find(|c| c.name == name && c.matches(domain, path, is_secure) && !c.is_expired())
            .map(|c| c.value.clone())
    }

    pub fn set_cookie(&self, url: &Url, name: &str, value: &str) {
        if plain_token(name) && !name.contains('=') && plain_token(value) {
            self.store(&format!("{name}={value}; Path=/"), url);
        }
    }

    pub fn load_cookies(&self, cookie_str: &str, url: &Url) {
        for pair in cookie_str.split(';') {
            if let Some((name, value)) = pair.trim().split_once('=') {
                self.set_cookie(url, name, value);
            }
        }
    }

    pub fn clear(&self) {
        let removed = {
            let mut jar = lock(&self.inner);
            jar.cookies.clear();
            std::mem::take(&mut jar.total)
        };
        self.changed_if(removed);
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

fn plain_token(text: &str) -> bool {
    !text.chars().any(|c| c == ';' || c.is_control())
}

#[cfg(test)]
mod tests;
