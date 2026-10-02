mod sessions;
mod state;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::core::block::BlockRules;
use crate::core::config::ProxyConfig;
use crate::core::session::Identity;

use sessions::IdentitySessions;
pub(crate) use state::Lease;
use state::PoolState;

const DEFAULT_BAN_AFTER: u32 = 3;
const DEFAULT_BAN_FOR: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Entry {
    config: ProxyConfig,
    label: String,
    identity: Option<Identity>,
}

#[derive(Debug, Clone)]
struct Settings {
    sticky_for: Duration,
    ban_after: u32,
    ban_for: Duration,
    block_rules: Option<BlockRules>,
}

#[derive(Debug, Clone)]
pub struct ProxyPool {
    entries: Arc<[Entry]>,
    settings: Settings,
    state: Arc<Mutex<PoolState>>,
    sessions: Arc<IdentitySessions>,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProxyHealth {
    pub proxy: String,
    pub in_use: u32,
    pub failures: u32,
    pub banned_until: Option<Instant>,
}

impl ProxyPool {
    pub fn new<P: Into<ProxyConfig>>(proxies: impl IntoIterator<Item = P>) -> Self {
        Self::from_entries(proxies.into_iter().map(|proxy| entry(proxy.into(), None)))
    }

    pub fn identified<P: Into<ProxyConfig>>(
        proxies: impl IntoIterator<Item = (P, Identity)>,
    ) -> Self {
        Self::from_entries(
            proxies
                .into_iter()
                .map(|(proxy, identity)| entry(proxy.into(), Some(identity))),
        )
    }

    pub(crate) fn configs(&self) -> impl Iterator<Item = &ProxyConfig> {
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
                block_rules: None,
            },
            state: Arc::new(Mutex::new(state)),
            sessions: Arc::default(),
        }
    }

    pub fn sticky_for(mut self, duration: Duration) -> Self {
        self.settings.sticky_for = duration;
        self
    }

    pub fn ban_after(mut self, failures: u32) -> Self {
        self.settings.ban_after = failures.max(1);
        self
    }

    pub fn ban_for(mut self, duration: Duration) -> Self {
        self.settings.ban_for = duration;
        self
    }

    pub fn rotate_on_block(mut self, rules: BlockRules) -> Self {
        match &mut self.settings.block_rules {
            Some(held) => held.extend(rules),
            None => self.settings.block_rules = Some(rules),
        }
        self
    }

    #[must_use]
    pub fn stats(&self) -> Vec<ProxyHealth> {
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

fn entry(config: ProxyConfig, identity: Option<Identity>) -> Entry {
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
