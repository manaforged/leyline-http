use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tokio::sync::watch;
use url::Url;

use crate::IntoUrl;
use crate::cookie::Cookie;
use crate::cookie::parse;
use crate::util::lock;

use self::evict::settle;

mod evict;
mod persist;

#[doc = include_str!("../../docs/jar.md")]
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
    /// Creates an empty cookie jar.
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

    /// Returns an independent copy of the jar's current cookies.
    pub fn snapshot(&self) -> Self {
        let jar = lock(&self.inner);
        Self::from_buckets(jar.cookies.clone(), jar.total)
    }

    /// Copies every cookie from `other` into this jar, replacing cookies with the same name and path.
    pub fn merge(&self, other: &Jar) {
        if Arc::ptr_eq(&self.inner, &other.inner) {
            return;
        }
        let incoming: Vec<Cookie> = lock(&other.inner)
            .cookies
            .values()
            .flatten()
            .filter(|c| c.within_limits())
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

    /// Parses a `Set-Cookie` header value received from `url` and stores the cookie, and ignores a `url` that does not parse.
    pub fn store(&self, url: impl IntoUrl, set_cookie: &str) {
        if let Ok(url) = url.into_url() {
            self.store_from(&url, set_cookie);
        }
    }

    fn store_from(&self, url: &Url, set_cookie: &str) {
        self.put(url, parse::parse_set_cookie(set_cookie, url));
    }

    /// Stores `cookie` as if a response from `url` had set it, with the host of `url` as its domain when the cookie has none, and ignores a cookie that `url` may not set and a `url` that does not parse.
    pub fn insert(&self, url: impl IntoUrl, cookie: Cookie) {
        if let Ok(url) = url.into_url()
            && plain_token(&cookie.name)
            && !cookie.name.contains('=')
            && plain_token(&cookie.value)
        {
            self.put(&url, parse::admit_cookie(&cookie, &url));
        }
    }

    fn put(&self, url: &Url, cookie: Option<Cookie>) {
        if cookie.is_some_and(|cookie| self.apply(cookie, url)) {
            self.changed();
        }
    }

    fn apply(&self, mut cookie: Cookie, url: &Url) -> bool {
        if !cookie.within_limits() {
            return false;
        }
        if !cookie.secure
            && !parse::secure_origin(url)
            && lock(&self.inner)
                .cookies
                .values()
                .flatten()
                .any(|existing| cookie.shadows_secure(existing))
        {
            return false;
        }

        let mut jar = lock(&self.inner);
        let domain = cookie.domain.to_lowercase();
        if cookie.is_expired() {
            return jar.expire(&domain, &cookie, parse::secure_origin(url));
        }

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
            self.store_from(url, header);
        }
    }

    /// Returns the `Cookie` header value to send to `url`, or `None` when no cookie matches or `url` does not parse.
    pub fn header_for(&self, url: impl IntoUrl) -> Option<String> {
        self.cookie_header_for(&url.into_url().ok()?, false, true)
    }

    pub(crate) fn is_empty(&self) -> bool {
        lock(&self.inner).total == 0
    }

    pub(crate) fn cookie_header_for(
        &self,
        url: &Url,
        cross_site: bool,
        lax_allowed: bool,
    ) -> Option<String> {
        use crate::cookie::record::SameSite;
        let mut jar = lock(&self.inner);
        if jar.total == 0 {
            return None;
        }
        let now = SystemTime::now();
        let sent = sent_to(url);

        let mut matching: Vec<&mut Cookie> = Vec::with_capacity(8);
        for entries in jar.cookies.values_mut() {
            for cookie in entries.iter_mut() {
                if !sent(&*cookie) {
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

    /// Removes every cookie with the given name on all domains and returns the count removed.
    pub fn remove_everywhere(&self, name: &str) -> usize {
        let removed = lock(&self.inner).remove_where(|c| c.name == name);
        self.changed_if(removed)
    }

    /// Removes cookies with the given name that match the host of `url` and returns the count removed, zero when `url` does not parse.
    pub fn remove(&self, url: impl IntoUrl, name: &str) -> usize {
        let Ok(url) = url.into_url() else {
            return 0;
        };
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

    /// Returns every stored cookie, sorted by domain and then name.
    pub fn all(&self) -> Vec<Cookie> {
        let jar = lock(&self.inner);
        let mut out: Vec<Cookie> = jar.cookies.values().flatten().cloned().collect();
        out.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Returns the value of the unexpired cookie named `name` that would be sent to `url`, or `None` when there is none or `url` does not parse.
    pub fn get(&self, url: impl IntoUrl, name: &str) -> Option<String> {
        let url = url.into_url().ok()?;
        let sent = sent_to(&url);
        let jar = lock(&self.inner);
        jar.cookies
            .values()
            .flatten()
            .find(|c| c.name == name && sent(c))
            .map(|c| c.value.clone())
    }

    /// Stores a cookie with path `/` for `url`, and ignores names or values that contain `;` or control characters and a `url` that does not parse.
    pub fn set(&self, url: impl IntoUrl, name: &str, value: &str) {
        self.insert(url, Cookie::new(name, value));
    }

    /// Removes every cookie from the jar.
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

fn sent_to(url: &Url) -> impl Fn(&Cookie) -> bool {
    let domain = url.host_str().unwrap_or("");
    let path = url.path();
    let is_secure = parse::secure_origin(url);
    move |cookie: &Cookie| cookie.matches(domain, path, is_secure) && !cookie.is_expired()
}

fn plain_token(text: &str) -> bool {
    !text.chars().any(|c| c == ';' || c.is_control())
}

#[cfg(test)]
mod tests;
