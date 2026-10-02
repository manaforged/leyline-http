use std::collections::HashMap;
use std::time::SystemTime;

use crate::cookie::record::Cookie;

use super::JarInner;

pub(super) const MAX_COOKIES_PER_DOMAIN: usize = 180;
const EVICT_PER_DOMAIN: usize = 30;
pub(super) const MAX_COOKIES_GLOBAL: usize = 3300;
const EVICT_GLOBAL: usize = 300;

impl JarInner {
    pub(super) fn remove_where(&mut self, mut doomed: impl FnMut(&Cookie) -> bool) -> usize {
        let mut removed = 0;
        for entries in self.cookies.values_mut() {
            let before = entries.len();
            entries.retain(|c| !doomed(c));
            removed += before - entries.len();
        }
        self.prune();
        self.total -= removed;
        removed
    }

    pub(super) fn remove_in(
        &mut self,
        domain: &str,
        mut doomed: impl FnMut(&Cookie) -> bool,
    ) -> usize {
        let Some(entries) = self.cookies.get_mut(domain) else {
            return 0;
        };
        let before = entries.len();
        entries.retain(|c| !doomed(c));
        let removed = before - entries.len();
        if entries.is_empty() {
            self.cookies.remove(domain);
        }
        self.total -= removed;
        removed
    }

    pub(super) fn insert_loaded(&mut self, cookie: Cookie) {
        let domain = cookie.domain.to_lowercase();
        let entries = self.cookies.entry(domain.clone()).or_default();
        let added = match entries.iter().position(|c| c.same_slot(&cookie)) {
            Some(pos) if entries[pos].creation_time > cookie.creation_time => return,
            Some(pos) => {
                entries[pos] = cookie;
                false
            }
            None => {
                entries.push(cookie);
                true
            }
        };
        settle(self, &domain, added);
    }

    pub(super) fn prune(&mut self) {
        self.cookies.retain(|_, entries| !entries.is_empty());
    }
}

pub(super) fn settle(jar: &mut JarInner, domain: &str, added: bool) {
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
        jar.prune();
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
