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
    flags: [u8; 6],
    pins: Vec<[u8; 32]>,
    roots: Vec<[u8; 32]>,
    identity: Vec<[u8; 32]>,
    ech: Option<[u8; 32]>,
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
                u8::from(config.accept_invalid_hostnames),
                u8::from(config.auto_ech),
                config.min_tls_version as u8,
            ],
            pins,
            roots: Vec::new(),
            identity: Vec::new(),
            ech: config
                .ech_config_list
                .as_deref()
                .map(|bytes| Sha256::digest(bytes).into()),
        };
        for root in &config.ca_roots {
            if let super::config::Pem::Bytes(pem) = root {
                key.root_bytes(pem);
            }
        }
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
        if let Some(ech) = self.ech {
            hasher.update([1u8]);
            hasher.update(ech);
        } else {
            hasher.update([0u8]);
        }
        TrustIdentity(hex::encode(hasher.finalize()).into())
    }
}
