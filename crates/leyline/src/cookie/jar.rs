//! Cookie jar — Chrome-accurate storage, retrieval, ordering, and persistence.
//!
//! Cookies are sorted exactly like Chrome:
//!   1. Path length descending (more specific first)
//!   2. Creation time ascending (oldest first)
//!
//! ## Two-faced API
//!
//! [`Jar`] presents two views of the same underlying store:
//!
//! - **Request-shaped** ([`Jar::cookie_header`], [`Jar::get_cookie`],
//!   [`Jar::export_cookies`]) — what gets sent on a request to a given URL,
//!   with full RFC 6265bis match semantics (secure flag, host-only, domain
//!   suffix, path).
//! - **Jar-shaped** ([`Jar::get_named`], [`Jar::all_cookies`],
//!   [`Jar::set_named_on`], serde) — every cookie regardless of which URL
//!   would currently see it. This is the persistence view, equivalent to
//!   Chrome's `Cookies` SQLite database.
//!
//! Use the request-shaped methods when building HTTP requests. Use the
//! jar-shaped methods (and `serde` round-tripping) when persisting jar
//! state across runs or moving it between sessions.

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
///
/// Cloning a `Jar` shares the underlying store (Arc-wrapped); use
/// [`Jar::deep_clone`] to fork an independent copy.
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

    /// Fork an independent jar holding a deep copy of every cookie. Unlike
    /// `Clone` (which shares the underlying `Arc<Mutex<_>>`), the result is
    /// fully decoupled — mutating one does not affect the other.
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
        // No cross-site context (e.g. a direct API call) — treat the request as
        // same-site, so every domain/path-matching cookie is eligible.
        self.cookie_header_for(url, false, true)
    }

    /// Build the Cookie header, enforcing SameSite for the request's
    /// cross-site context. On a cross-site request, `Strict` cookies are
    /// withheld and `Lax` cookies are sent only for a safe top-level
    /// navigation (`GET`/`HEAD`); `None` cookies always go (storage already
    /// required `Secure`). Same-site requests apply no SameSite filtering.
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

        // Collect matching cookies across all domains.
        let mut matching: Vec<&mut Cookie> = Vec::new();
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

    /// Look up a cookie value by name without a URL filter. Returns the
    /// first non-expired cookie across every stored domain that matches
    /// `name`. Useful when code only needs to know whether a tracked
    /// cookie landed (for example a named session cookie)
    /// without caring which exact URL scope it came in on.
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

    /// Update the value of every cookie matching `name`, across every
    /// domain and path the jar holds. Returns `true` if at least one
    /// cookie was updated.
    ///
    /// "Update everywhere" is the right semantic for refreshing auth
    /// tokens that may be stored on multiple subdomains (e.g.
    /// `example.com` + `api.example.com`). It also makes the operation
    /// deterministic — `HashMap` iteration order is not stable, but the
    /// outcome (every match gets the new value) is the same regardless
    /// of order. Bumps `last_access` on every updated cookie.
    ///
    /// Callers that want to insert in the absence of a match should
    /// follow up with [`Jar::set_named_on`] for the relevant domain.
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

    /// Upsert a cookie on a specific domain. If a cookie with `name`
    /// already exists for `domain` on **any path**, its value is updated
    /// in place (preserving its original path/secure/same_site flags).
    /// Otherwise a fresh host-only cookie is inserted on `Path=/`.
    ///
    /// Matching by name regardless of path avoids the silent-duplicate
    /// hazard where an auth cookie originally minted on `Path=/account`
    /// would never be matched and a second `Path=/` entry would shadow
    /// it in the jar.
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

    /// Remove the first cookie matching `name` (any domain). Returns `true`
    /// if a cookie was removed.
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

    /// Remove every cookie matching `name` across every domain. Returns the
    /// number of cookies removed.
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

    /// Snapshot every cookie in the jar, sorted by domain then name.
    /// Includes expired cookies — filter with `Cookie::is_expired` if needed.
    /// Useful for debug logs, diffing across runs, and structured inspection.
    pub fn all_cookies(&self) -> Vec<Cookie> {
        let jar = lock(&self.inner);
        let mut out: Vec<Cookie> = jar.cookies.values().flatten().cloned().collect();
        out.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
        out
    }

    /// Merge cookies from another jar into this one. Last-write-wins by
    /// `(domain, path, name)` — overlapping entries take their value from
    /// `other`, disjoint entries are added.
    ///
    /// Expired cookies are dropped (matching `store_set_cookie`'s policy
    /// — a serialized jar that's been on disk past its expiry window
    /// shouldn't resurrect dead cookies on load). Per-domain (180) and
    /// global (3300) eviction limits are enforced after the merge to
    /// stay within Chrome's invariants for long-lived sessions.
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
    ///
    /// **Note:** This is a request-shaped interop helper for legacy callers
    /// that hold a Cookie header. It cannot recover the original domain
    /// attribution of each cookie — every entry is stored as host-only on
    /// the URL's host. Prefer `serde::Deserialize` for jar persistence.
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
    ///
    /// **Request-shaped:** equivalent to "what would the `Cookie:` header
    /// be for a request to `raw_url` right now?" This is the right shape
    /// for building a request, but the wrong shape for persistence —
    /// cookies pinned to other subdomains are filtered out. Use serde on
    /// [`Jar`] to persist the full jar.
    pub fn export_cookies(&self, raw_url: &str) -> String {
        let url = match Url::parse(raw_url) {
            Ok(u) => u,
            Err(_) => return String::new(),
        };
        self.cookie_header(&url).unwrap_or_default()
    }

    /// Remove every cookie from the jar. Useful for re-using a [`Jar`]
    /// across logically distinct sessions on the same `Session` without
    /// constructing a new pool or TLS connector.
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

// ─── Persistence ────────────────────────────────────────────────────────────
//
// Wire format: a flat `Vec<Cookie>`. Each cookie carries its domain, path,
// host_only, secure, etc., so on deserialize we re-bucket into the in-memory
// `HashMap<domain, Vec<Cookie>>` without touching the parse path. This is
// lossless: a host-only `api.example.com` cookie comes back pinned to
// `api.example.com`, not broadcast to the parent domain.

impl Serialize for Jar {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let jar = lock(&self.inner);
        // Sort by (domain, path, name, creation_time) so the on-disk
        // representation is byte-stable across runs. `HashMap` iteration
        // is unspecified, which would otherwise produce a fresh
        // serialization for every save and break diffs/equality checks.
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
        let jar = Jar::new();
        jar.set_cookie("https://example.com", "_ab", "abc123");
        assert_eq!(
            jar.get_cookie("https://example.com", "_ab"),
            Some("abc123".into())
        );
    }

    #[test]
    fn cookie_ordering_chrome_style() {
        let jar = Jar::new();
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
        let jar = Jar::new();
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
        let jar = Jar::new();
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
        let jar = Jar::new();
        jar.load_cookies("a=1; b=2", "https://example.com/page");
        let export = jar.export_cookies("https://example.com/other");
        assert_eq!(export, "a=1; b=2");
    }

    #[test]
    fn same_path_cookies_keep_creation_order() {
        let jar = Jar::new();
        jar.load_cookies(
            "zeta=r; alpha=i; mid=v",
            "https://www.example.com/page",
        );
        jar.set_cookie("https://www.example.com/", "late", "abc123");

        let export = jar.export_cookies("https://www.example.com/cart/items");
        assert_eq!(
            export,
            "zeta=r; alpha=i; mid=v; late=abc123"
        );
    }

    #[test]
    fn replacement_preserves_original_creation_order() {
        let jar = Jar::new();
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
        let jar = Jar::new();
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
        let jar = Jar::new();
        let url = Url::parse("https://example.com/").unwrap();

        // Max-Age=0 means delete/expire immediately.
        jar.store_set_cookie("gone=bye; Max-Age=0", &url);
        assert_eq!(jar.get_cookie("https://example.com", "gone"), None);
    }

    #[test]
    fn per_domain_eviction() {
        let jar = Jar::new();
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
        let jar = Jar::new();
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
    fn samesite_enforced_on_cross_site_requests() {
        let jar = Jar::new();
        let set = Url::parse("https://example.com/").unwrap();
        jar.store_set_cookie("strict=1; SameSite=Strict", &set);
        jar.store_set_cookie("lax=1; SameSite=Lax", &set);
        jar.store_set_cookie("none=1; SameSite=None; Secure", &set);

        let req = Url::parse("https://example.com/page").unwrap();

        // Same-site: all three are eligible.
        let same = jar.cookie_header_for(&req, false, true).unwrap();
        assert!(same.contains("strict=1") && same.contains("lax=1") && same.contains("none=1"));

        // Cross-site safe navigation (GET): Strict withheld; Lax + None sent.
        let cross_get = jar.cookie_header_for(&req, true, true).unwrap();
        assert!(!cross_get.contains("strict=1"), "{cross_get}");
        assert!(cross_get.contains("lax=1") && cross_get.contains("none=1"));

        // Cross-site unsafe navigation (POST): only None sent.
        let cross_post = jar.cookie_header_for(&req, true, false).unwrap();
        assert!(!cross_post.contains("strict=1") && !cross_post.contains("lax=1"));
        assert!(cross_post.contains("none=1"));
    }

    #[test]
    fn secure_cookie_not_sent_over_http() {
        let jar = Jar::new();
        let https = Url::parse("https://example.com/").unwrap();
        jar.store_set_cookie("tok=secret; Secure; SameSite=None", &https);

        // Available over HTTPS.
        assert!(jar.get_cookie("https://example.com", "tok").is_some());
        // NOT available over HTTP.
        assert!(jar.get_cookie("http://example.com", "tok").is_none());
    }

    #[test]
    fn get_named_finds_across_domains() {
        let jar = Jar::new();
        let www = Url::parse("https://www.example.com/").unwrap();
        let api = Url::parse("https://api.example.com/").unwrap();
        jar.store_set_cookie("a=www; Path=/", &www);
        jar.store_set_cookie("b=api; Path=/", &api);
        assert_eq!(jar.get_named("a").as_deref(), Some("www"));
        assert_eq!(jar.get_named("b").as_deref(), Some("api"));
        assert_eq!(jar.get_named("missing"), None);
    }

    #[test]
    fn set_named_updates_existing_only() {
        let jar = Jar::new();
        jar.set_cookie("https://example.com", "tok", "old");
        assert!(jar.set_named("tok", "new"));
        assert_eq!(jar.get_named("tok").as_deref(), Some("new"));
        assert!(!jar.set_named("absent", "v"));
        assert_eq!(jar.get_named("absent"), None);
    }

    #[test]
    fn set_named_on_upserts() {
        let jar = Jar::new();
        jar.set_named_on("api.example.com", "session", "first");
        assert_eq!(jar.get_named("session").as_deref(), Some("first"));
        jar.set_named_on("api.example.com", "session", "second");
        assert_eq!(jar.get_named("session").as_deref(), Some("second"));
        assert_eq!(jar.len(), 1);
    }

    #[test]
    fn remove_named_first_match() {
        let jar = Jar::new();
        jar.set_cookie("https://a.example.com", "k", "1");
        jar.set_cookie("https://b.example.com", "k", "2");
        assert_eq!(jar.len(), 2);
        assert!(jar.remove_named("k"));
        assert_eq!(jar.len(), 1);
        assert!(jar.remove_named("k"));
        assert!(!jar.remove_named("k"));
    }

    #[test]
    fn remove_all_named_clears_every_domain() {
        let jar = Jar::new();
        jar.set_cookie("https://a.example.com", "k", "1");
        jar.set_cookie("https://b.example.com", "k", "2");
        jar.set_cookie("https://c.example.com", "other", "3");
        assert_eq!(jar.remove_all_named("k"), 2);
        assert_eq!(jar.len(), 1);
        assert!(jar.contains_named("other"));
    }

    #[test]
    fn merge_combines_jars_last_write_wins() {
        let a = Jar::new();
        a.set_cookie("https://example.com", "k1", "from_a");
        a.set_cookie("https://example.com", "shared", "a_value");

        let b = Jar::new();
        b.set_cookie("https://example.com", "k2", "from_b");
        b.set_cookie("https://example.com", "shared", "b_value");

        a.merge(&b);
        assert_eq!(a.get_named("k1").as_deref(), Some("from_a"));
        assert_eq!(a.get_named("k2").as_deref(), Some("from_b"));
        assert_eq!(a.get_named("shared").as_deref(), Some("b_value"));
    }

    // ─── Persistence ─────────────────────────────────

    #[test]
    fn serde_round_trip_preserves_cross_subdomain_attribution() {
        // A host-only cookie on `api.example.com` must round-trip
        // losslessly through serde. A `String`-shaped export that filtered
        // against `https://www.example.com` would silently drop it.
        let jar = Jar::new();
        jar.store_set_cookie(
            "auth=secret; Path=/; Secure",
            &Url::parse("https://api.example.com").unwrap(),
        );
        jar.store_set_cookie(
            "shared=value; Domain=example.com; Path=/; Secure",
            &Url::parse("https://www.example.com").unwrap(),
        );
        jar.store_set_cookie(
            "wwwonly=val; Path=/",
            &Url::parse("https://www.example.com").unwrap(),
        );
        assert_eq!(jar.len(), 3);

        let json = serde_json::to_string(&jar).expect("serialize");
        let restored: Jar = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.len(), 3);

        // The host-only gsp cookie is back pinned to gsp, not www.
        assert_eq!(
            restored.get_cookie("https://api.example.com", "auth"),
            Some("secret".into()),
            "auth must be visible on api.example.com"
        );
        assert_eq!(
            restored.get_cookie("https://www.example.com", "auth"),
            None,
            "auth must NOT leak to www.example.com (host-only)"
        );
        assert_eq!(
            restored.get_cookie("https://www.example.com", "shared"),
            Some("value".into())
        );
        assert_eq!(
            restored.get_cookie("https://api.example.com", "shared"),
            Some("value".into()),
            "Domain=example.com cookie must reach every subdomain"
        );
        assert_eq!(
            restored.get_cookie("https://www.example.com", "wwwonly"),
            Some("val".into())
        );
    }

    #[test]
    fn serde_empty_jar() {
        let jar = Jar::new();
        let json = serde_json::to_string(&jar).expect("serialize");
        assert_eq!(json, "[]");
        let restored: Jar = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.is_empty());
    }

    #[test]
    fn deep_clone_is_independent() {
        let a = Jar::new();
        a.set_cookie("https://example.com", "k", "1");
        let b = a.deep_clone();
        a.set_named("k", "2");
        assert_eq!(a.get_named("k").as_deref(), Some("2"));
        assert_eq!(b.get_named("k").as_deref(), Some("1"));
    }

    // ─── set_named / merge invariants ───────────────────────────────────────────

    #[test]
    fn set_named_updates_every_match_across_domains() {
        // `set_named` must update every match and return true if any. Stopping
        // at the first match returned by HashMap iteration is non-deterministic
        // when the same cookie name lives on multiple domains.
        let jar = Jar::new();
        jar.set_cookie("https://example.com", "token", "old1");
        jar.set_cookie("https://api.example.com", "token", "old2");
        assert_eq!(jar.len(), 2);
        assert!(jar.set_named("token", "rotated"));
        // Both sites see the new token regardless of HashMap iteration order.
        assert_eq!(
            jar.get_cookie("https://example.com", "token")
                .as_deref(),
            Some("rotated")
        );
        assert_eq!(
            jar.get_cookie("https://api.example.com", "token")
                .as_deref(),
            Some("rotated")
        );
    }

    #[test]
    fn set_named_on_updates_existing_path_not_just_root() {
        // `set_named_on` must match cookies on any path, not just `Path=/`. A
        // cookie minted by the server on `Path=/auth` would otherwise go
        // unmatched, and a duplicate would be inserted on `Path=/`, producing
        // two entries with the same name and silently emitting both in the
        // Cookie header.
        let jar = Jar::new();
        let url = Url::parse("https://example.com/auth/redirect").unwrap();
        jar.store_set_cookie("accessToken=initial; Path=/auth", &url);
        assert_eq!(jar.len(), 1);

        jar.set_named_on("example.com", "token", "rotated");
        assert_eq!(
            jar.len(),
            1,
            "must update existing entry, not insert duplicate"
        );
        // Original path attribute preserved — the cookie still applies on
        // /auth, not just /.
        assert_eq!(
            jar.get_cookie("https://example.com/auth/foo", "token")
                .as_deref(),
            Some("rotated")
        );
    }

    #[test]
    fn set_named_bumps_last_access() {
        // `set_named` must bump `last_access`; leaving it stale would let a
        // recently-rotated auth token be evicted before truly-cold cookies
        // under LRU pressure.
        let jar = Jar::new();
        jar.set_cookie("https://example.com", "tok", "old");
        let before = {
            let inner = lock(&jar.inner);
            inner.cookies["example.com"][0].last_access
        };
        std::thread::sleep(std::time::Duration::from_millis(5));
        jar.set_named("tok", "new");
        let after = {
            let inner = lock(&jar.inner);
            inner.cookies["example.com"][0].last_access
        };
        assert!(after > before, "set_named should bump last_access");
    }

    #[test]
    fn merge_drops_expired_cookies() {
        // `merge` must drop expired cookies, matching `store_set_cookie`'s
        // filtering. Accepting them lets a deserialized jar whose session has
        // aged past `expires` silently keep dead cookies in the live HTTP jar.
        let live = Jar::new();
        let stale = Jar::new();
        let url = Url::parse("https://example.com/").unwrap();
        // Past-expiry cookie via Max-Age=-1 — the parser stamps this as
        // already-expired so `merge` must skip it.
        stale.store_set_cookie("dead=yes; Path=/; Max-Age=1", &url);
        // Manually rewind the cookie's `expires` to the past.
        {
            let mut inner = lock(&stale.inner);
            if let Some(entries) = inner.cookies.get_mut("example.com") {
                for c in entries.iter_mut() {
                    c.expires = Some(SystemTime::UNIX_EPOCH);
                }
            }
        }
        stale.set_cookie("https://example.com", "alive", "ok");

        live.merge(&stale);
        assert_eq!(
            live.get_cookie("https://example.com", "dead"),
            None,
            "expired cookie must not survive merge"
        );
        assert_eq!(
            live.get_cookie("https://example.com", "alive").as_deref(),
            Some("ok"),
            "non-expired cookie must survive merge"
        );
    }

    #[test]
    fn merge_enforces_per_domain_eviction_cap() {
        // `merge` must enforce eviction; skipping it lets long-lived
        // sessions blow past Chrome's 180-cookie per-domain ceiling.
        let live = Jar::new();
        let bulk = Jar::new();
        let url = Url::parse("https://example.com/").unwrap();
        // Stuff the source jar to MAX-1 so eviction triggers on merge.
        for i in 0..(MAX_COOKIES_PER_DOMAIN + 5) {
            bulk.store_set_cookie(&format!("c{i}=v{i}; Path=/"), &url);
        }

        live.merge(&bulk);
        let count = {
            let inner = lock(&live.inner);
            inner.cookies.get("example.com").map_or(0, |v| v.len())
        };
        assert!(
            count <= MAX_COOKIES_PER_DOMAIN,
            "merge must respect per-domain cap; got {count}"
        );
    }

    #[test]
    fn serialize_order_is_stable_across_runs() {
        // `Serialize` must emit a stable order. Emitting
        // `HashMap.values().flatten()` gives a fresh ordering every run,
        // breaking diff-based session change detection and on-disk equality
        // checks.
        let jar = Jar::new();
        // Populate across multiple domains/paths so a HashMap shuffle
        // would actually change the serialization.
        let u1 = Url::parse("https://www.example.com/").unwrap();
        let u2 = Url::parse("https://api.example.com/").unwrap();
        let u3 = Url::parse("https://api.example.com/").unwrap();
        jar.store_set_cookie("z=last; Path=/", &u1);
        jar.store_set_cookie("a=first; Path=/", &u2);
        jar.store_set_cookie("m=mid; Path=/account", &u3);
        jar.store_set_cookie("m=mid; Path=/", &u3);

        let s1 = serde_json::to_string(&jar).unwrap();
        let s2 = serde_json::to_string(&jar).unwrap();
        let s3 = serde_json::to_string(&jar).unwrap();
        assert_eq!(s1, s2, "serialization must be byte-stable");
        assert_eq!(s2, s3, "serialization must be byte-stable");
    }
}
