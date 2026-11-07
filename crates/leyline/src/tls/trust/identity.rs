use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::Tls;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrustIdentity(Arc<str>);

impl TrustIdentity {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// Everything that decides which peers a TLS context trusts and how it presents itself.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TrustKey {
    flags: [u8; 4],
    pins: Vec<[u8; 32]>,
    roots: Vec<[u8; 32]>,
    identity: Vec<[u8; 32]>,
}

impl TrustKey {
    pub(crate) fn of(config: &Tls) -> Self {
        let mut pins = config.pinned_leaf_sha256.clone();
        pins.sort_unstable();
        pins.dedup();
        let mut key = Self {
            flags: [
                u8::from(config.use_system_roots),
                u8::from(config.use_env_roots),
                u8::from(config.accept_invalid_certs),
                config.min_tls_version as u8,
            ],
            pins,
            roots: Vec::new(),
            identity: Vec::new(),
        };
        for der in &config.ca_der {
            key.root_bytes(der);
        }
        key
    }

    pub(crate) fn root_bytes(&mut self, bytes: &[u8]) {
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        if let Err(at) = self.roots.binary_search(&digest) {
            self.roots.insert(at, digest);
        }
    }

    pub(crate) fn identity_bytes(&mut self, bytes: &[u8]) {
        self.identity.push(Sha256::digest(bytes).into());
    }

    pub(crate) fn identity(&self) -> TrustIdentity {
        let mut hasher = Sha256::new();
        hasher.update(self.flags);
        for section in [&self.pins, &self.roots, &self.identity] {
            hasher.update((section.len() as u64).to_be_bytes());
            for item in section {
                hasher.update(item);
            }
        }
        TrustIdentity(hex::encode(hasher.finalize()).into())
    }
}
