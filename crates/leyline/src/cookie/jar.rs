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
            // Leave Secure Cookies Alone covers deletions too: a non-Secure
            // deletion cannot remove a Secure cookie.
            let mut jar = lock(&self.inner);
            let domain = cookie.domain.to_lowercase();
            if let Some(entries) = jar.cookies.get_mut(&domain) {
                if let Some(pos) = entries
                    .iter()
                    .position(|c| c.name == cookie.name && c.path == cookie.path)
                {
                    if entries[pos].secure && url.scheme() != "https" {
                        return;
                    }
                    entries.remove(pos);
                    jar.total -= 1;
                }
            }
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
            // RFC 6265bis §5.7 "Leave Secure Cookies Alone": a non-Secure
            // cookie must never overwrite a Secure cookie, from any origin.
            // Without this, one plaintext response can fixate the session
            // cookie an HTTPS origin later reads.
            if entries[pos].secure && !cookie.secure {
                return;
            }
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
    /// cookie landed (e.g. a named session cookie set by a challenge
    /// page) without caring which exact URL scope it came in on.
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
    ///
    /// **Trusted-caller contract.** `domain` is the caller's own input and is
    /// NOT re-validated against the Public Suffix List, because the inserted
    /// cookie is always **host-only** (`host_only: true`): per
    /// [`Cookie::matches`](crate::cookie::record::Cookie::matches) a host-only
    /// cookie is sent only on an exact host match and can never broadcast to
    /// sibling or child domains, so a public-suffix `domain` here yields a
    /// cookie scoped to that exact host — not a supercookie. Applying the
    /// parser's PSL guard would in fact be *stricter* than the parse path,
    /// which does not run that guard on host-only cookies either
    /// (`cookie/parse.rs`), and would wrongly reject legitimate host-only
    /// cookies such as `localhost` used in local development. The network
    /// trust boundary — where an attacker-supplied `Domain=` attribute could
    /// broaden scope — is [`Jar::store_set_cookie`], which enforces PSL,
    /// domain-match, and the eviction caps. This jar-shaped setter is the
    /// local-state / persistence view (see the type-level docs) and does not
    /// enforce the per-domain / global caps; a caller inserting unbounded
    /// distinct names owns that discipline.
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

    /// Remove cookies named `name` whose domain is `host` itself or a parent
    /// suffix of it (e.g. removing for `store.example.com` also clears a stale
    /// entry mis-hosted on `example.com`), while preserving same-named cookies
    /// on sibling hosts like `www.example.com`. Returns the number removed.
    ///
    /// This is the host-scoped counterpart to [`Jar::remove_all_named`]: use it
    /// to refresh a host-bound cookie (e.g. `session`) on one host without
    /// evicting an independent sibling host's copy.
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
mod tests;
