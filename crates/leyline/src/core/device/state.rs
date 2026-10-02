use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::core::Session;
use crate::util::epoch_plus;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SessionState {
    #[serde(default)]
    tls_sessions: Vec<TlsTicket>,
    #[serde(default)]
    alt_svc: Vec<AltSvcEntry>,
    #[serde(default)]
    hsts: Vec<HstsEntry>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TlsTicket {
    key: String,
    session: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AltSvcEntry {
    host: String,
    port: u16,
    expires: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct HstsEntry {
    host: String,
    #[serde(default)]
    include_subdomains: bool,
    expires: u64,
}

pub(crate) struct StateParts {
    pub(crate) tls_sessions: Vec<(String, Vec<u8>)>,
    pub(crate) alt_svc: Vec<(String, u16, SystemTime)>,
    pub(crate) hsts: Vec<(String, bool, SystemTime)>,
}

impl SessionState {
    pub fn restore_into(&self, session: &Session) {
        session.restore_state(&self.parts());
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tls_sessions.is_empty() && self.alt_svc.is_empty() && self.hsts.is_empty()
    }

    pub(crate) fn from_parts(parts: StateParts) -> Self {
        Self {
            tls_sessions: parts
                .tls_sessions
                .into_iter()
                .map(|(key, der)| TlsTicket {
                    key,
                    session: STANDARD.encode(der),
                })
                .collect(),
            alt_svc: parts
                .alt_svc
                .into_iter()
                .map(|(host, port, at)| AltSvcEntry {
                    host,
                    port,
                    expires: unix_secs(at),
                })
                .collect(),
            hsts: parts
                .hsts
                .into_iter()
                .map(|(host, include_subdomains, at)| HstsEntry {
                    host,
                    include_subdomains,
                    expires: unix_secs(at),
                })
                .collect(),
        }
    }

    fn parts(&self) -> StateParts {
        StateParts {
            tls_sessions: self
                .tls_sessions
                .iter()
                .filter_map(|t| Some((t.key.clone(), STANDARD.decode(&t.session).ok()?)))
                .collect(),
            alt_svc: self
                .alt_svc
                .iter()
                .filter_map(|e| Some((e.host.clone(), e.port, at_secs(e.expires)?)))
                .collect(),
            hsts: self
                .hsts
                .iter()
                .filter_map(|e| Some((e.host.clone(), e.include_subdomains, at_secs(e.expires)?)))
                .collect(),
        }
    }
}

impl std::fmt::Debug for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionState")
            .field("tls_sessions", &self.tls_sessions.len())
            .field("alt_svc", &self.alt_svc.len())
            .field("hsts", &self.hsts.len())
            .finish()
    }
}

pub(crate) fn unix_secs(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn at_secs(secs: u64) -> Option<SystemTime> {
    epoch_plus(Duration::from_secs(secs))
}
