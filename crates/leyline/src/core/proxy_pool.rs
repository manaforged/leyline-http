mod sessions;
mod state;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::core::config::Proxy;
use crate::core::session::Identity;
use crate::profile::{Browser, Platform};

use sessions::IdentitySessions;
pub(crate) use state::Lease;
use state::PoolState;

const DEFAULT_BAN_AFTER: u32 = 3;
const DEFAULT_BAN_FOR: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Entry {
    config: Proxy,
    label: String,
    identity: Option<Identity>,
}

#[derive(Debug, Clone)]
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
    proxy: String,
    in_use: u32,
    failures: u32,
    banned_until: Option<Instant>,
}

impl ProxyHealth {
    pub(crate) fn proxy(&self) -> &str {
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

    pub(crate) fn configs(&self) -> impl Iterator<Item = &Proxy> {
        self.entries.iter().map(|entry| &entry.config)
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
            .map(|(entry, health)| ProxyHealth {
                proxy: entry.label.clone(),
                in_use: health.in_use,
                failures: health.failures,
                banned_until: health.banned_until,
            })
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn entry(config: Proxy, identity: Option<Identity>) -> Entry {
    let label = config
        .primary()
        .map(crate::util::redact)
        .unwrap_or_default();
    Entry {
        config,
        label,
        identity,
    }
}
