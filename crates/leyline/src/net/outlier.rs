use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

const CONSECUTIVE_FAILURES: u32 = 5;
const BASE_EJECTION: Duration = Duration::from_secs(30);
const MAX_EJECTION: Duration = Duration::from_secs(300);
const MAX_TRACKED: usize = 4096;

/// Moves addresses that keep failing to connect to the back of the dial order for a while.
#[derive(Debug, Default)]
pub(crate) struct Outliers {
    records: Mutex<HashMap<SocketAddr, Record>>,
}

#[derive(Debug, Default, Clone, Copy)]
struct Record {
    failures: u32,
    ejections: u32,
    until: Option<Instant>,
}

impl Outliers {
    pub(crate) fn failed(&self, addr: SocketAddr) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if records.len() >= MAX_TRACKED && !records.contains_key(&addr) {
            let now = Instant::now();
            records.retain(|_, record| record.until.is_some_and(|until| until > now));
            if records.len() >= MAX_TRACKED {
                return;
            }
        }
        let record = records.entry(addr).or_default();
        record.failures += 1;
        if record.failures >= CONSECUTIVE_FAILURES {
            record.failures = 0;
            record.ejections = record.ejections.saturating_add(1);
            let time = BASE_EJECTION
                .saturating_mul(record.ejections)
                .min(MAX_EJECTION);
            record.until = Some(Instant::now() + time);
        }
    }

    pub(crate) fn succeeded(&self, addr: SocketAddr) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !records.is_empty() {
            records.remove(&addr);
        }
    }

    pub(crate) fn order(&self, mut addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
        let records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if records.is_empty() {
            return addrs;
        }
        let now = Instant::now();
        let ejected = |addr: &SocketAddr| {
            records
                .get(addr)
                .and_then(|record| record.until)
                .is_some_and(|until| until > now)
        };
        addrs.sort_by_key(ejected);
        addrs
    }
}

#[cfg(test)]
#[path = "outlier_tests.rs"]
mod tests;
