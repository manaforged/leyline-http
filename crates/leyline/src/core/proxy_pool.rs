mod sessions;
mod state;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::core::Identity;
use crate::core::config::Proxy;
use crate::profile::{Browser, Platform};

use sessions::IdentitySessions;
pub(crate) use state::Lease;
use state::PoolState;

const DEFAULT_BAN_AFTER: u32 = 3;
const DEFAULT_BAN_FOR: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Entry {
    config: Proxy,
    url: Option<url::Url>,
    identity: Option<Identity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
    sticky_for: Duration,
    ban_after: u32,
    ban_for: Duration,
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyPool {
    entries: Arc<[Entry]>,
    settings: Settings,
    state: Arc<Mutex<PoolState>>,
    sessions: Arc<IdentitySessions>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyHealth {
    proxy: url::Url,
    in_use: u32,
    failures: u32,
    banned_until: Option<Instant>,
}

impl ProxyHealth {
    pub(crate) fn proxy(&self) -> &url::Url {
        &self.proxy
    }

    pub(crate) fn in_use(&self) -> u32 {
        self.in_use
    }

    pub(crate) fn failures(&self) -> u32 {
        self.failures
    }

    pub(crate) fn banned_until(&self) -> Option<Instant> {
        self.banned_until
    }
}

impl ProxyPool {
    pub(crate) fn new<P: Into<Proxy>>(proxies: impl IntoIterator<Item = P>) -> Self {
        Self::from_entries(proxies.into_iter().map(|proxy| entry(proxy.into(), None)))
    }

    pub(crate) fn identified<P: Into<Proxy>>(
        proxies: impl IntoIterator<Item = (P, Browser, Platform)>,
    ) -> Self {
        Self::from_entries(proxies.into_iter().map(|(proxy, browser, platform)| {
            entry(proxy.into(), Some(Identity::new(browser, platform)))
        }))
    }

    pub(crate) fn needs_impersonation(&self) -> bool {
        self.entries.iter().any(|entry| entry.identity.is_some())
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn configs(&self) -> impl Iterator<Item = &Proxy> {
        self.entries.iter().map(|entry| &entry.config)
    }

    fn map_entries(mut self, mut f: impl FnMut(&mut Entry)) -> Self {
        let entries: Arc<[Entry]> = self
            .entries
            .iter()
            .cloned()
            .map(|mut entry| {
                f(&mut entry);
                entry
            })
            .collect();
        self.entries = entries;
        self
    }

    pub(crate) fn with_headers(self, headers: &[(String, String)]) -> Self {
        self.map_entries(|entry| entry.config.push_validated_headers(headers))
    }

    pub(crate) fn with_no_proxy(self, no_proxy: &crate::core::config::NoProxy) -> Self {
        self.map_entries(|entry| entry.config.set_no_proxy(no_proxy.clone()))
    }

    pub(crate) fn bypasses_all_url(&self, url: &url::Url) -> bool {
        !self.entries.is_empty() && self.entries.iter().all(|e| e.config.bypasses_url(url))
    }

    pub(crate) fn lease_for(
        &self,
        url: &url::Url,
        last: Option<usize>,
    ) -> Option<(Option<crate::core::config::Origin>, Lease)> {
        if self.bypasses_all_url(url) {
            return None;
        }
        let origin = crate::core::config::Origin::of(url);
        let lease = self.lease(origin.as_ref(), last)?;
        Some((origin, lease))
    }

    pub(crate) fn acquire_session(
        &self,
        base: &crate::core::session::Session,
        url: &url::Url,
        last: Option<usize>,
    ) -> crate::error::Result<
        Option<(
            Option<crate::core::config::Origin>,
            Lease,
            crate::core::session::Session,
        )>,
    > {
        let Some((origin, lease)) = self.lease_for(url, last) else {
            return Ok(None);
        };
        let session = lease.session(base)?;
        Ok(Some((origin, lease, session)))
    }

    pub(crate) fn for_doh(&self) -> Self {
        let entries: Arc<[Entry]> = self
            .entries
            .iter()
            .map(|entry| Entry {
                config: entry.config.clone(),
                url: entry.url.clone(),
                identity: None,
            })
            .collect();
        Self {
            state: Arc::new(Mutex::new(PoolState::new(entries.len()))),
            entries,
            settings: self.settings.clone(),
            sessions: Arc::default(),
        }
    }

    pub(crate) fn same_for_doh(&self, other: &Self) -> bool {
        self.settings == other.settings
            && self.entries.len() == other.entries.len()
            && self
                .entries
                .iter()
                .zip(other.entries.iter())
                .all(|(a, b)| a.config.same_for_doh(&b.config))
    }

    fn from_entries(entries: impl Iterator<Item = Entry>) -> Self {
        let entries: Arc<[Entry]> = entries.collect();
        let state = PoolState::new(entries.len());
        Self {
            entries,
            settings: Settings {
                sticky_for: Duration::ZERO,
                ban_after: DEFAULT_BAN_AFTER,
                ban_for: DEFAULT_BAN_FOR,
            },
            state: Arc::new(Mutex::new(state)),
            sessions: Arc::default(),
        }
    }

    pub(crate) fn sticky_for(mut self, duration: Duration) -> Self {
        self.settings.sticky_for = duration;
        self
    }

    pub(crate) fn ban_after(mut self, failures: u32) -> Self {
        self.settings.ban_after = failures.max(1);
        self
    }

    pub(crate) fn ban_for(mut self, duration: Duration) -> Self {
        self.settings.ban_for = duration;
        self
    }

    #[must_use]
    pub(crate) fn stats(&self) -> Vec<ProxyHealth> {
        let state = self.lock();
        self.entries
            .iter()
            .zip(state.health())
            .filter_map(|(entry, health)| {
                Some(ProxyHealth {
                    proxy: entry.url.clone()?,
                    in_use: health.in_use,
                    failures: health.failures,
                    banned_until: health.banned_until,
                })
            })
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PoolState> {
        crate::util::lock(&self.state)
    }
}

fn entry(config: Proxy, identity: Option<Identity>) -> Entry {
    let url = config.shown_url();
    Entry {
        config,
        url,
        identity,
    }
}
