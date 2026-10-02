use std::time::{Duration, SystemTime};

use url::{Host, Url};

use crate::pool::ExpiringSet;
use crate::util::delta_seconds;

const SECURE_SCHEME: &str = "https";
const PLAIN_SCHEME: &str = "http";
const MAX_AGE: &str = "max-age";
const INCLUDE_SUBDOMAINS: &str = "includesubdomains";

#[derive(Default)]
pub(crate) struct HstsStore {
    exact: ExpiringSet<String>,
    subdomains: ExpiringSet<String>,
}

struct Policy {
    max_age: Duration,
    include_subdomains: bool,
}

impl HstsStore {
    pub(crate) fn note(&mut self, url: &Url, values: &[&str], now: SystemTime) {
        if url.scheme() != SECURE_SCHEME {
            return;
        }
        let (Some(host), Some(first)) = (domain(url), values.first()) else {
            return;
        };
        let Some(policy) = parse(first) else {
            return;
        };
        self.exact.remove(&host);
        self.subdomains.remove(&host);
        let Some(expiry) = Some(policy.max_age)
            .filter(|age| !age.is_zero())
            .and_then(|age| now.checked_add(age))
        else {
            return;
        };
        self.insert(host, policy.include_subdomains, expiry, now);
    }

    pub(crate) fn upgrade(&mut self, url: &Url, now: SystemTime) -> Option<Url> {
        if url.scheme() != PLAIN_SCHEME {
            return None;
        }
        let host = domain(url)?;
        if !self.knows(&host, now) {
            return None;
        }
        let mut upgraded = url.clone();
        upgraded.set_scheme(SECURE_SCHEME).ok()?;
        Some(upgraded)
    }

    pub(crate) fn export(&self, now: SystemTime) -> Vec<(String, bool, SystemTime)> {
        let exact = self
            .exact
            .live(now)
            .map(|(host, expiry)| (host.clone(), false, expiry));
        let subdomains = self
            .subdomains
            .live(now)
            .map(|(host, expiry)| (host.clone(), true, expiry));
        let mut entries: Vec<_> = exact.chain(subdomains).collect();
        entries.sort();
        entries
    }

    pub(crate) fn import(&mut self, entries: &[(String, bool, SystemTime)], now: SystemTime) {
        for (host, include_subdomains, expiry) in entries {
            if *expiry > now {
                self.insert(host.to_ascii_lowercase(), *include_subdomains, *expiry, now);
            }
        }
    }

    fn insert(
        &mut self,
        host: String,
        include_subdomains: bool,
        expiry: SystemTime,
        now: SystemTime,
    ) {
        if include_subdomains {
            self.exact.remove(&host);
            self.subdomains.insert(host, expiry, now);
        } else {
            self.subdomains.remove(&host);
            self.exact.insert(host, expiry, now);
        }
    }

    fn knows(&mut self, host: &str, now: SystemTime) -> bool {
        if self.exact.contains(&host.to_owned(), now) {
            return true;
        }
        let mut rest = host;
        loop {
            if self.subdomains.contains(&rest.to_owned(), now) {
                return true;
            }
            match rest.split_once('.') {
                Some((_, parent)) if !parent.is_empty() => rest = parent,
                _ => return false,
            }
        }
    }
}

fn domain(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Domain(name) => Some(name.trim_end_matches('.').to_ascii_lowercase()),
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
}

fn parse(value: &str) -> Option<Policy> {
    let mut max_age = None;
    let mut include_subdomains = false;
    for directive in value.split(';').map(str::trim).filter(|d| !d.is_empty()) {
        let (name, raw) = match directive.split_once('=') {
            Some((name, raw)) => (name.trim(), Some(raw.trim())),
            None => (directive, None),
        };
        if name.eq_ignore_ascii_case(MAX_AGE) {
            if max_age.is_some() {
                return None;
            }
            max_age = Some(seconds(raw?)?);
        } else if name.eq_ignore_ascii_case(INCLUDE_SUBDOMAINS) {
            if include_subdomains || raw.is_some() {
                return None;
            }
            include_subdomains = true;
        }
    }
    Some(Policy {
        max_age: max_age?,
        include_subdomains,
    })
}

fn seconds(raw: &str) -> Option<Duration> {
    let raw = raw
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(raw);
    delta_seconds(raw).map(|secs| Duration::from_secs(secs.min(u64::from(u32::MAX))))
}
