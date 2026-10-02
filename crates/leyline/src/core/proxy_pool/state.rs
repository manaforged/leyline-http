use std::collections::HashMap;
use std::time::Instant;

use super::ProxyPool;
use crate::core::Result;
use crate::core::config::{Origin, ProxyConfig};
use crate::core::response::Response;
use crate::core::session::Session;
use crate::util::saturating_after;

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Health {
    pub(super) in_use: u32,
    pub(super) failures: u32,
    pub(super) banned_until: Option<Instant>,
}

#[derive(Debug)]
pub(super) struct PoolState {
    health: Vec<Health>,
    sticky: HashMap<Origin, (usize, Instant)>,
    cursor: usize,
}

impl PoolState {
    pub(super) fn new(len: usize) -> Self {
        Self {
            health: vec![Health::default(); len],
            sticky: HashMap::new(),
            cursor: 0,
        }
    }

    pub(super) fn health(&self) -> &[Health] {
        &self.health
    }

    fn usable(&self, index: usize, now: Instant) -> bool {
        self.health[index]
            .banned_until
            .is_none_or(|until| until <= now)
    }

    fn sticky_index(&self, origin: &Origin, now: Instant) -> Option<usize> {
        let (index, until) = *self.sticky.get(origin)?;
        (until > now && self.usable(index, now)).then_some(index)
    }

    fn next_index(&mut self, avoid: Option<usize>, now: Instant) -> Option<usize> {
        let len = self.health.len();
        let found = (0..len)
            .map(|step| (self.cursor + step) % len)
            .find(|&index| Some(index) != avoid && self.usable(index, now))
            .or_else(|| self.soonest_unbanned(avoid))?;
        self.cursor = found + 1;
        Some(found)
    }

    fn soonest_unbanned(&self, avoid: Option<usize>) -> Option<usize> {
        let len = self.health.len();
        (0..len)
            .filter(|&index| len == 1 || Some(index) != avoid)
            .min_by_key(|&index| self.health[index].banned_until)
    }

    fn stick(&mut self, origin: Origin, index: usize, until: Instant, now: Instant) {
        self.sticky.retain(|_, (_, expiry)| *expiry > now);
        self.sticky.insert(origin, (index, until));
    }

    fn unstick(&mut self, origin: Option<&Origin>, index: usize) {
        if let Some(origin) = origin
            && self
                .sticky
                .get(origin)
                .is_some_and(|(held, _)| *held == index)
        {
            self.sticky.remove(origin);
        }
    }

    fn strike(&mut self, index: usize, pool: &ProxyPool, now: Instant) {
        let health = &mut self.health[index];
        health.failures += 1;
        if health.failures < pool.settings.ban_after {
            return;
        }
        health.failures = 0;
        health.banned_until = Some(saturating_after(now, pool.settings.ban_for));
        self.sticky.retain(|_, (held, _)| *held != index);
    }
}

pub(crate) struct Lease {
    pool: ProxyPool,
    index: usize,
}

impl ProxyPool {
    pub(crate) fn lease(&self, origin: Option<&Origin>, avoid: Option<usize>) -> Option<Lease> {
        if self.entries.is_empty() {
            return None;
        }
        let now = Instant::now();
        let mut state = self.lock();
        let held = origin
            .filter(|_| avoid.is_none())
            .and_then(|origin| state.sticky_index(origin, now));
        let index = match held {
            Some(index) => index,
            None => {
                let index = state.next_index(avoid, now)?;
                self.stick_new(&mut state, origin, index, now);
                index
            }
        };
        state.health[index].in_use += 1;
        drop(state);
        Some(Lease {
            pool: self.clone(),
            index,
        })
    }

    fn stick_new(
        &self,
        state: &mut PoolState,
        origin: Option<&Origin>,
        index: usize,
        now: Instant,
    ) {
        let Some(origin) = origin else {
            return;
        };
        if self.settings.sticky_for.is_zero() {
            return;
        }
        let until = saturating_after(now, self.settings.sticky_for);
        state.stick(origin.clone(), index, until, now);
    }
}

impl super::Settings {
    fn is_block(&self, response: &Response) -> bool {
        self.block_rules
            .as_ref()
            .is_some_and(|rules| rules.check(response).is_some())
    }
}

impl Lease {
    pub(crate) fn index(&self) -> usize {
        self.index
    }

    pub(crate) fn config(&self) -> &ProxyConfig {
        &self.pool.entries[self.index].config
    }

    pub(crate) fn session(&self, base: &Session) -> Result<Session> {
        match self.pool.entries[self.index].identity {
            Some(identity) => self.pool.sessions.session(base, identity),
            None => Ok(base.clone()),
        }
    }

    pub(crate) fn record(&self, result: &Result<Response>, origin: Option<&Origin>) {
        let strike = match result {
            Err(err) if err.is_proxy() => true,
            Ok(resp) => self.pool.settings.is_block(resp),
            Err(_) => return,
        };
        let mut state = self.pool.lock();
        if !strike {
            state.health[self.index].failures = 0;
            return;
        }
        state.unstick(origin, self.index);
        state.strike(self.index, &self.pool, Instant::now());
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.pool.lock();
        let health = &mut state.health[self.index];
        health.in_use = health.in_use.saturating_sub(1);
    }
}
