//! Cookie jar — Chrome-accurate storage, retrieval, and ordering.
//!
//! Cookies are sorted exactly like Chrome:
//!   1. Path length descending (more specific first)
//!   2. Creation time ascending (oldest first)

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use url::Url;

use crate::cookies::cookie::Cookie;
use crate::cookies::parse;

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
pub struct CookieJar {
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

impl CookieJar {
    /// Create an empty cookie jar.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(JarInner {
                cookies: HashMap::new(),
                total: 0,
            })),
        }
    }

    /// Store a Set-Cookie header from a response.
    pub fn store_set_cookie(&self, header: &str, url: &Url) {
        let mut cookie = match parse::parse_set_cookie(header, url) {
            Some(c) => c,
            None => return,
        };

        // Don't store expired cookies (Max-Age=0 means delete).
        if cookie.is_expired() {
            self.remove(&cookie.domain, &cookie.name, &cookie.path);
            return;
        }

        let mut jar = lock(&self.inner);
        let domain = cookie.domain.to_lowercase();

        let entries = jar.cookies.entry(domain.clone()).or_default();

        // Replace existing cookie with same name+domain+path. Browsers keep
        // the original creation time on replacement, which preserves Cookie
        // header order for refreshed auth/session cookies.
        let mut added = false;
        if let Some(pos) = entries
            .iter()
            .position(|c| c.name == cookie.name && c.path == cookie.path)
        {
            cookie.creation_time = entries[pos].creation_time;
            entries[pos] = cookie;
        } else {
            entries.push(cookie);
            added = true;
        }

        // Enforce per-domain limit (180).
        let mut evicted = 0;
        if entries.len() > MAX_COOKIES_PER_DOMAIN {
            evict_lru(entries, EVICT_PER_DOMAIN);
            evicted = EVICT_PER_DOMAIN;
        }

        // Update total count.
        if added {
            jar.total += 1;
        }
        jar.total -= evicted;

        // Enforce global limit (3300).
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
    ///
    /// Returns cookies sorted exactly like Chrome:
    ///   1. Path length descending
    ///   2. Creation time ascending (oldest first)
    pub fn cookie_header(&self, url: &Url) -> Option<String> {
        let domain = url.host_str().unwrap_or("");
        let path = url.path();
        let is_secure = url.scheme() == "https";

        let mut jar = lock(&self.inner);
        let now = SystemTime::now();

        // Collect matching cookies across all domains.
        let mut matching: Vec<&mut Cookie> = Vec::new();
        for entries in jar.cookies.values_mut() {
            for cookie in entries.iter_mut() {
                if cookie.is_expired() {
                    continue;
                }
                if cookie.matches(domain, path, is_secure) {
                    cookie.last_access = now;
                    matching.push(cookie);
                }
            }
        }

        if matching.is_empty() {
            return None;
        }

        // Chrome sort: path length desc, then creation time asc.
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

    /// Remove every cookie from the jar. Useful for re-using a
    /// [`super::CookieJar`] across logically distinct sessions on the
    /// same `Session` without constructing a new pool or TLS connector.
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

    fn remove(&self, domain: &str, name: &str, path: &str) {
        let mut jar = lock(&self.inner);
        let domain = domain.to_lowercase();
        if let Some(entries) = jar.cookies.get_mut(&domain) {
            if let Some(pos) = entries
                .iter()
                .position(|c| c.name == name && c.path == path)
            {
                entries.remove(pos);
                jar.total -= 1;
            }
        }
    }
}

impl Default for CookieJar {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CookieJar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let jar = lock(&self.inner);
        f.debug_struct("CookieJar")
            .field("domains", &jar.cookies.len())
            .field("total", &jar.total)
            .finish()
    }
}

/// Evict the N least-recently-accessed cookies from a domain's list.
fn evict_lru(cookies: &mut Vec<Cookie>, count: usize) {
    // Sort by last_access ascending, remove the oldest.
    cookies.sort_by_key(|a| a.last_access);
    cookies.drain(..count.min(cookies.len()));
}

/// Evict N cookies globally, targeting least-recently-accessed.
fn evict_global(all: &mut HashMap<String, Vec<Cookie>>, count: usize) {
    // Collect all cookies with their domain key, sort by LRU.
    let mut all_cookies: Vec<(String, usize, SystemTime)> = Vec::new();
    for (domain, entries) in all.iter() {
        for (i, cookie) in entries.iter().enumerate() {
            all_cookies.push((domain.clone(), i, cookie.last_access));
        }
    }
    all_cookies.sort_by_key(|a| a.2);

    // Remove the oldest `count` cookies.
    let to_remove = count.min(all_cookies.len());
    // Collect indices to remove, grouped by domain (reverse order to avoid shifting).
    let mut removals: HashMap<String, Vec<usize>> = HashMap::new();
    for (domain, idx, _) in &all_cookies[..to_remove] {
        removals.entry(domain.clone()).or_default().push(*idx);
    }
    for (domain, mut indices) in removals {
        indices.sort_unstable_by(|a, b| b.cmp(a)); // reverse order
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
mod tests {
    use super::*;

    #[test]
    fn basic_set_and_get() {
        let jar = CookieJar::new();
        jar.set_cookie("https://example.com", "_ab", "abc123");
        assert_eq!(
            jar.get_cookie("https://example.com", "_ab"),
            Some("abc123".into())
        );
    }

    #[test]
    fn cookie_ordering_chrome_style() {
        let jar = CookieJar::new();
        let url = Url::parse("https://example.com/app/page").unwrap();

        // Set cookies with different paths and creation times.
        // Use store_set_cookie directly for control.
        jar.store_set_cookie("a=1; Path=/", &url);
        jar.store_set_cookie("b=2; Path=/app", &url);
        jar.store_set_cookie("c=3; Path=/app/page", &url);

        let header = jar.cookie_header(&url).unwrap();
        // Order: /app/page (longest path) first, then /app, then /
        assert!(
            header.starts_with("c=3"),
            "expected c=3 first, got: {header}"
        );
        assert!(header.contains("b=2"));
        assert!(header.ends_with("a=1"), "expected a=1 last, got: {header}");
    }

    #[test]
    fn creation_time_ordering() {
        let jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();

        // Same path, different creation times. Oldest should come first.
        jar.store_set_cookie("first=1; Path=/", &url);
        std::thread::sleep(std::time::Duration::from_millis(10));
        jar.store_set_cookie("second=2; Path=/", &url);

        let header = jar.cookie_header(&url).unwrap();
        // Same path length → creation time ascending → first before second.
        assert!(
            header.find("first=1").unwrap() < header.find("second=2").unwrap(),
            "older cookie should come first: {header}"
        );
    }

    #[test]
    fn set_cookie_response() {
        let jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();

        jar.store_response_cookies(
            &[
                "session=abc; Path=/; Secure; HttpOnly",
                "theme=dark; Path=/",
            ],
            &url,
        );

        let header = jar.cookie_header(&url).unwrap();
        assert!(header.contains("session=abc"));
        assert!(header.contains("theme=dark"));
    }

    #[test]
    fn load_and_export() {
        let jar = CookieJar::new();
        jar.load_cookies("a=1; b=2", "https://example.com/page");
        let export = jar.export_cookies("https://example.com/other");
        assert_eq!(export, "a=1; b=2");
    }

    #[test]
    fn same_path_cookies_keep_creation_order() {
        let jar = CookieJar::new();
        jar.load_cookies(
            "zeta=r; alpha=i; mid=v",
            "https://www.example.com/page",
        );
        jar.set_cookie("https://www.example.com/", "late", "v");

        let export = jar.export_cookies("https://www.example.com/cart/items");
        assert_eq!(
            export,
            "zeta=r; alpha=i; mid=v; late=v"
        );
    }

    #[test]
    fn replacement_preserves_original_creation_order() {
        let jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();
        jar.store_set_cookie("first=old; Path=/", &url);
        std::thread::sleep(std::time::Duration::from_millis(10));
        jar.store_set_cookie("second=2; Path=/", &url);
        std::thread::sleep(std::time::Duration::from_millis(10));
        jar.store_set_cookie("first=new; Path=/", &url);

        let header = jar.cookie_header(&url).unwrap();
        assert_eq!(header, "first=new; second=2");
    }

    #[test]
    fn longer_path_cookies_precede_same_path_creation_order() {
        let jar = CookieJar::new();
        let url = Url::parse("https://example.com/cart/items").unwrap();
        jar.store_response_cookies(
            &["root=1; Path=/", "deep=1; Path=/cart", "tail=1; Path=/"],
            &url,
        );

        let export = jar.export_cookies("https://example.com/cart/items");
        assert_eq!(export, "deep=1; root=1; tail=1");
    }

    #[test]
    fn expired_cookies_not_returned() {
        let jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();

        // Max-Age=0 means delete/expire immediately.
        jar.store_set_cookie("gone=bye; Max-Age=0", &url);
        assert_eq!(jar.get_cookie("https://example.com", "gone"), None);
    }

    #[test]
    fn per_domain_eviction() {
        let jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();

        // Insert 181 cookies — should trigger eviction.
        for i in 0..=MAX_COOKIES_PER_DOMAIN {
            jar.store_set_cookie(&format!("c{}=v{}; Path=/", i, i), &url);
        }

        let inner = lock(&jar.inner);
        let count = inner
            .cookies
            .get("example.com")
            .map(|v| v.len())
            .unwrap_or(0);
        assert!(
            count <= MAX_COOKIES_PER_DOMAIN,
            "expected <= {MAX_COOKIES_PER_DOMAIN}, got {count}"
        );
    }

    #[test]
    fn samesite_none_requires_secure() {
        let jar = CookieJar::new();
        let url = Url::parse("https://example.com/").unwrap();

        jar.store_set_cookie("bad=val; SameSite=None", &url);
        assert_eq!(jar.get_cookie("https://example.com", "bad"), None);

        jar.store_set_cookie("good=val; SameSite=None; Secure", &url);
        assert_eq!(
            jar.get_cookie("https://example.com", "good"),
            Some("val".into())
        );
    }

    #[test]
    fn secure_cookie_not_sent_over_http() {
        let jar = CookieJar::new();
        let https = Url::parse("https://example.com/").unwrap();
        jar.store_set_cookie("tok=secret; Secure; SameSite=None", &https);

        // Available over HTTPS.
        assert!(jar.get_cookie("https://example.com", "tok").is_some());
        // NOT available over HTTP.
        assert!(jar.get_cookie("http://example.com", "tok").is_none());
    }
}
