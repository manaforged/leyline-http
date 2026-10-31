use std::path::PathBuf;

use crate::tls::builder::TlsMinVersion;

/// TLS trust settings: root stores, extra CAs, a client certificate, pins, and the minimum version.
#[derive(Clone)]
#[non_exhaustive]
#[must_use]
pub struct Tls {
    pub(crate) use_env_roots: bool,
    pub(crate) use_system_roots: bool,
    pub(crate) ca_files: Vec<PathBuf>,
    pub(crate) ca_der: Vec<Vec<u8>>,
    pub(crate) client_identity: Option<ClientIdentity>,
    pub(crate) pinned_leaf_sha256: Vec<[u8; 32]>,
    pub(crate) accept_invalid_certs: bool,
    pub(crate) min_tls_version: TlsMinVersion,
}

impl std::fmt::Debug for Tls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ca_der: Vec<usize> = self.ca_der.iter().map(Vec::len).collect();
        let pins: Vec<String> = self.pinned_leaf_sha256.iter().map(hex::encode).collect();
        f.debug_struct("Tls")
            .field("use_env_roots", &self.use_env_roots)
            .field("use_system_roots", &self.use_system_roots)
            .field("ca_files", &self.ca_files)
            .field("ca_der_lengths", &ca_der)
            .field("client_identity", &self.client_identity)
            .field("pinned_leaf_sha256", &pins)
            .field("accept_invalid_certs", &self.accept_invalid_certs)
            .field("min_tls_version", &self.min_tls_version)
            .finish()
    }
}

impl Default for Tls {
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

impl Tls {
    /// Returns the default settings: environment and system roots on, TLS 1.0 minimum.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets whether roots named by the environment load, on by default.
    pub fn env_roots(mut self, on: bool) -> Self {
        self.use_env_roots = on;
        self
    }

    /// Sets whether the system root store loads, on by default.
    pub fn system_roots(mut self, on: bool) -> Self {
        self.use_system_roots = on;
        self
    }

    /// Adds a CA certificate file to trust.
    pub fn ca_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_files.push(path.into());
        self
    }

    /// Adds a DER-encoded CA certificate to trust.
    pub fn ca_der(mut self, der: impl Into<Vec<u8>>) -> Self {
        self.ca_der.push(der.into());
        self
    }

    /// Adds a SHA-256 pin for the server's leaf certificate.
    pub fn pin_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.pinned_leaf_sha256.push(sha256);
        self
    }

    /// Sets the client certificate chain file and private key file.
    pub fn client_cert(
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

    /// Sets whether invalid server certificates are accepted, off by default.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Raises the minimum TLS version to 1.2 unless it is already higher.
    pub fn tls12_min(mut self) -> Self {
        self.min_tls_version = self.min_tls_version.max(TlsMinVersion::Tls12);
        self
    }

    /// Sets the minimum TLS version to 1.3.
    pub fn tls13_only(mut self) -> Self {
        self.min_tls_version = TlsMinVersion::Tls13;
        self
    }

    pub(crate) fn shareable(&self) -> bool {
        self.client_identity.is_none()
            && self.ca_files.is_empty()
            && !(self.use_env_roots && super::env_roots_set())
            && !crate::tls::keylog::requested()
    }

    pub(crate) fn same_roots(&self, other: &Tls) -> bool {
        self.use_env_roots == other.use_env_roots
            && self.use_system_roots == other.use_system_roots
            && self.ca_der == other.ca_der
            && self.pinned_leaf_sha256 == other.pinned_leaf_sha256
            && self.accept_invalid_certs == other.accept_invalid_certs
            && self.min_tls_version == other.min_tls_version
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
