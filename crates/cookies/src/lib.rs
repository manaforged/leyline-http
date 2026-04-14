//! RFC 6265 cookie jar with load/export/track.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cookie_store::CookieStore;
use url::Url;

/// Thread-safe cookie jar.
#[derive(Clone)]
pub struct CookieJar {
    store: Arc<Mutex<CookieStore>>,
    tracked: Arc<Mutex<HashMap<String, String>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl CookieJar {
    /// Create an empty cookie jar.
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(CookieStore::default())),
            tracked: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Parse a `Cookie` header string and load into the jar.
    pub fn load_cookies(&self, cookie_str: &str, raw_url: &str) {
        if cookie_str.is_empty() {
            return;
        }
        let url = match Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => return,
        };
        let domain = cookie_domain(url.host_str().unwrap_or(""));
        let mut store = lock(&self.store);
        for pair in cookie_str.split(';') {
            let pair = pair.trim();
            if let Some(eq) = pair.find('=') {
                let name = &pair[..eq];
                let value = &pair[eq + 1..];
                let set_cookie = format!("{}={}; Domain={}; Path=/", name, value, domain);
                let _ = store.parse(&set_cookie, &url);
            }
        }
    }

    /// Set a single named cookie.
    pub fn set_cookie(&self, raw_url: &str, name: &str, value: &str) {
        let url = match Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => return,
        };
        let domain = cookie_domain(url.host_str().unwrap_or(""));
        let set_cookie = format!("{}={}; Domain={}; Path=/", name, value, domain);
        let mut store = lock(&self.store);
        let _ = store.parse(&set_cookie, &url);
    }

    /// Get a single cookie value by name.
    pub fn get_cookie(&self, raw_url: &str, name: &str) -> Option<String> {
        let url = Url::parse(raw_url).ok()?;
        let store = lock(&self.store);
        store
            .matches(&url)
            .into_iter()
            .find(|c| c.name() == name)
            .map(|c| c.value().to_string())
    }

    /// Export all cookies for a URL as a `Cookie` header string.
    pub fn export_cookies(&self, raw_url: &str) -> String {
        let url = match Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => return String::new(),
        };
        let store = lock(&self.store);
        let cookies = store.matches(&url);
        cookies
            .iter()
            .map(|c| format!("{}={}", c.name(), c.value()))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Build the `Cookie` header value for a request URL.
    pub fn cookie_header(&self, url: &Url) -> Option<String> {
        let store = lock(&self.store);
        let cookies = store.matches(url);
        if cookies.is_empty() {
            return None;
        }
        Some(
            cookies
                .iter()
                .map(|c| format!("{}={}", c.name(), c.value()))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    /// Store Set-Cookie headers from a response.
    pub fn store_response_cookies(&self, set_cookie_values: &[&str], url: &Url) {
        let mut store = lock(&self.store);
        for val in set_cookie_values {
            let _ = store.parse(val, url);
        }
    }

    /// Set a tracked cookie (separate from the jar, for cross-request tracking).
    pub fn set_tracked(&self, name: &str, value: &str) {
        lock(&self.tracked).insert(name.to_string(), value.to_string());
    }

    /// Get a tracked cookie value.
    pub fn get_tracked(&self, name: &str) -> Option<String> {
        lock(&self.tracked).get(name).cloned()
    }
}

impl Default for CookieJar {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CookieJar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CookieJar").finish_non_exhaustive()
    }
}

fn cookie_domain(host: &str) -> String {
    let h = host.strip_prefix("www.").unwrap_or(host);
    if h.contains('.') {
        format!(".{}", h)
    } else {
        h.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_and_export() {
        let jar = CookieJar::new();
        jar.load_cookies("a=1; b=2", "https://example.com/page");
        let export = jar.export_cookies("https://example.com/other");
        assert!(export.contains("a=1"));
        assert!(export.contains("b=2"));
    }

    #[test]
    fn load_cookies_without_spaces() {
        let jar = CookieJar::new();
        jar.load_cookies("a=1;b=2;c=3", "https://example.com");
        assert_eq!(jar.get_cookie("https://example.com", "a"), Some("1".into()));
        assert_eq!(jar.get_cookie("https://example.com", "b"), Some("2".into()));
        assert_eq!(jar.get_cookie("https://example.com", "c"), Some("3".into()));
    }

    #[test]
    fn set_and_get() {
        let jar = CookieJar::new();
        jar.set_cookie("https://example.com", "_ab", "abc123");
        assert_eq!(
            jar.get_cookie("https://example.com", "_ab"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn tracked_cookies() {
        let jar = CookieJar::new();
        jar.set_tracked("session_id", "xyz");
        assert_eq!(jar.get_tracked("session_id"), Some("xyz".to_string()));
    }
}
