use std::sync::Arc;

use sha2::{Digest, Sha256};

use super::TlsTrustConfig;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TrustIdentity(Arc<str>);

impl TrustIdentity {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Default)]
pub(crate) struct LoadedTrust {
    roots: Vec<[u8; 32]>,
    identity: Vec<[u8; 32]>,
}

impl LoadedTrust {
    pub(crate) fn root_bytes(&mut self, bytes: &[u8]) {
        self.roots.push(Sha256::digest(bytes).into());
    }

    pub(crate) fn identity_bytes(&mut self, bytes: &[u8]) {
        self.identity.push(Sha256::digest(bytes).into());
    }

    pub(crate) fn finish(mut self, config: &TlsTrustConfig) -> TrustIdentity {
        let mut pins = config.pinned_leaf_sha256.clone();
        pins.sort_unstable();
        pins.dedup();
        self.roots.sort_unstable();
        self.roots.dedup();
        let mut hasher = Sha256::new();
        hasher.update([
            u8::from(config.use_system_roots),
            u8::from(config.use_env_roots),
            u8::from(config.accept_invalid_certs),
        ]);
        for section in [&pins, &self.roots, &self.identity] {
            hasher.update((section.len() as u64).to_be_bytes());
            for item in section {
                hasher.update(item);
            }
        }
        TrustIdentity(hex::encode(hasher.finalize()).into())
    }
}
