use std::path::PathBuf;

use crate::tls::builder::TlsMinVersion;

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TlsTrustConfig {
    pub(crate) use_env_roots: bool,
    pub(crate) use_system_roots: bool,
    pub(crate) ca_files: Vec<PathBuf>,
    pub(crate) ca_der: Vec<Vec<u8>>,
    pub(crate) client_identity: Option<ClientIdentity>,
    pub(crate) pinned_leaf_sha256: Vec<[u8; 32]>,
    pub(crate) accept_invalid_certs: bool,
    pub(crate) min_tls_version: TlsMinVersion,
}

impl Default for TlsTrustConfig {
    fn default() -> Self {
        Self {
            use_env_roots: true,
            use_system_roots: true,
            ca_files: Vec::new(),
            ca_der: Vec::new(),
            client_identity: None,
            pinned_leaf_sha256: Vec::new(),
            accept_invalid_certs: false,
            min_tls_version: TlsMinVersion::Tls10,
        }
    }
}

impl TlsTrustConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn env_roots(mut self, on: bool) -> Self {
        self.use_env_roots = on;
        self
    }

    pub fn system_roots(mut self, on: bool) -> Self {
        self.use_system_roots = on;
        self
    }

    pub fn add_ca_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_files.push(path.into());
        self
    }

    pub fn add_ca_der(mut self, der: impl Into<Vec<u8>>) -> Self {
        self.ca_der.push(der.into());
        self
    }

    pub fn add_pinned_leaf_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.pinned_leaf_sha256.push(sha256);
        self
    }

    pub fn client_identity(
        mut self,
        certificate_chain_file: impl Into<PathBuf>,
        private_key_file: impl Into<PathBuf>,
    ) -> Self {
        self.client_identity = Some(ClientIdentity {
            certificate_chain_file: certificate_chain_file.into(),
            private_key_file: private_key_file.into(),
        });
        self
    }

    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    pub fn min_tls_version(mut self, version: TlsMinVersion) -> Self {
        self.min_tls_version = version;
        self
    }

    pub(crate) fn accepts_invalid_certs(&self) -> bool {
        self.accept_invalid_certs
    }

    pub(crate) fn uses_system_roots(&self) -> bool {
        self.use_system_roots
    }

    pub(crate) fn has_client_identity(&self) -> bool {
        self.client_identity.is_some()
    }

    pub(crate) fn pinned_leaf_sha256(&self) -> &[[u8; 32]] {
        &self.pinned_leaf_sha256
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ClientIdentity {
    pub(crate) certificate_chain_file: PathBuf,
    pub(crate) private_key_file: PathBuf,
}
