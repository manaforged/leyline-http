use std::path::{Path, PathBuf};

use super::identity::TrustKey;
use crate::profile::TlsMinVersion;

/// TLS trust settings: root stores, extra CAs, a client certificate, pins, ECH, and the minimum version.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
#[must_use]
pub struct Tls {
    pub(crate) use_env_roots: bool,
    pub(crate) use_system_roots: bool,
    pub(super) ca_roots: Vec<Pem>,
    pub(crate) ca_der: Vec<Vec<u8>>,
    pub(super) client_identity: Option<ClientIdentity>,
    pub(crate) pinned_leaf_sha256: Vec<[u8; 32]>,
    pub(crate) accept_invalid_certs: bool,
    pub(crate) accept_invalid_hostnames: bool,
    pub(crate) ech_config_list: Option<Vec<u8>>,
    pub(crate) auto_ech: bool,
    pub(crate) min_tls_version: TlsMinVersion,
}

impl std::fmt::Debug for Tls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ca_der: Vec<usize> = self.ca_der.iter().map(Vec::len).collect();
        let pins: Vec<String> = self.pinned_leaf_sha256.iter().map(hex::encode).collect();
        f.debug_struct("Tls")
            .field("use_env_roots", &self.use_env_roots)
            .field("use_system_roots", &self.use_system_roots)
            .field("ca_roots", &self.ca_roots)
            .field("ca_der_lengths", &ca_der)
            .field("client_identity", &self.client_identity)
            .field("pinned_leaf_sha256", &pins)
            .field("accept_invalid_certs", &self.accept_invalid_certs)
            .field("accept_invalid_hostnames", &self.accept_invalid_hostnames)
            .field(
                "ech_config_list_len",
                &self.ech_config_list.as_ref().map(Vec::len),
            )
            .field("auto_ech", &self.auto_ech)
            .field("min_tls_version", &self.min_tls_version)
            .finish()
    }
}

impl Default for Tls {
    fn default() -> Self {
        Self {
            use_env_roots: true,
            use_system_roots: true,
            ca_roots: Vec::new(),
            ca_der: Vec::new(),
            client_identity: None,
            pinned_leaf_sha256: Vec::new(),
            accept_invalid_certs: false,
            accept_invalid_hostnames: false,
            ech_config_list: None,
            auto_ech: false,
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
    pub fn ca_file(mut self, path: impl AsRef<Path>) -> Self {
        self.ca_roots.push(Pem::File(path.as_ref().to_path_buf()));
        self
    }

    /// Adds one or more PEM-encoded CA certificates in memory to trust.
    pub fn ca_pem(mut self, pem: impl Into<Vec<u8>>) -> Self {
        self.ca_roots.push(Pem::Bytes(pem.into()));
        self
    }

    /// Adds a DER-encoded CA certificate to trust.
    pub fn ca_der(mut self, der: impl Into<Vec<u8>>) -> Self {
        self.ca_der.push(der.into());
        self
    }

    /// Adds a pin: the SHA-256 digest of the server's whole DER-encoded leaf certificate, not of its public key.
    pub fn pin_cert_sha256(mut self, sha256: [u8; 32]) -> Self {
        self.pinned_leaf_sha256.push(sha256);
        self
    }

    /// Sets the client certificate from two PEM files: the certificate chain with the leaf first, then its unencrypted private key.
    pub fn client_cert(
        mut self,
        certificate_chain_file: impl AsRef<Path>,
        private_key_file: impl AsRef<Path>,
    ) -> Self {
        self.client_identity = Some(ClientIdentity::Pem {
            certificate_chain: Pem::File(certificate_chain_file.as_ref().to_path_buf()),
            private_key: Pem::File(private_key_file.as_ref().to_path_buf()),
        });
        self
    }

    /// Sets the client certificate from PEM bytes in memory: the certificate chain with the leaf first, then its unencrypted private key, replacing any previous client certificate.
    pub fn client_cert_pem(mut self, cert: impl Into<Vec<u8>>, key: impl Into<Vec<u8>>) -> Self {
        self.client_identity = Some(ClientIdentity::Pem {
            certificate_chain: Pem::Bytes(cert.into()),
            private_key: Pem::Bytes(key.into()),
        });
        self
    }

    /// Sets the client certificate from a DER-encoded PKCS#12 archive and its password, replacing any previous client certificate.
    pub fn client_cert_pkcs12(
        mut self,
        der: impl Into<Vec<u8>>,
        password: impl Into<String>,
    ) -> Self {
        self.client_identity = Some(ClientIdentity::Pkcs12 {
            der: der.into(),
            password: password.into(),
        });
        self
    }

    /// Sets whether invalid server certificates are accepted, off by default.
    pub fn danger_accept_invalid_certs(mut self, accept: bool) -> Self {
        self.accept_invalid_certs = accept;
        self
    }

    /// Sets whether server certificates with mismatched hostnames are accepted while still verifying trust chains and pins, off by default.
    pub fn danger_accept_invalid_hostnames(mut self, accept: bool) -> Self {
        self.accept_invalid_hostnames = accept;
        self
    }

    /// Sets a raw RFC 9460 `ECHConfigList` to encrypt the ClientHello SNI on TLS 1.3 connections.
    pub fn ech_config_list(mut self, config_list: impl Into<Vec<u8>>) -> Self {
        self.ech_config_list = Some(config_list.into());
        self
    }

    /// Sets whether to query the session's resolver for HTTPS/SVCB ECH records before TLS handshakes, off by default.
    pub fn ech(mut self, enable: bool) -> Self {
        self.auto_ech = enable;
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

    pub(crate) fn shared_key(&self) -> Option<TrustKey> {
        let has_ca_file = self
            .ca_roots
            .iter()
            .any(|root| matches!(root, Pem::File(_)));
        let shareable = self.client_identity.is_none()
            && !has_ca_file
            && !(self.use_env_roots && super::env_roots_set())
            && !crate::tls::keylog::requested();
        shareable.then(|| TrustKey::of(self))
    }

    pub(crate) fn accepts_invalid_certs(&self) -> bool {
        self.accept_invalid_certs
    }

    pub(crate) fn accepts_invalid_hostnames(&self) -> bool {
        self.accept_invalid_hostnames
    }

    pub(crate) fn ech_config(&self) -> Option<&[u8]> {
        self.ech_config_list.as_deref()
    }

    pub(crate) fn auto_ech(&self) -> bool {
        self.auto_ech
    }

    pub(crate) fn has_client_identity(&self) -> bool {
        self.client_identity.is_some()
    }

    pub(crate) fn pinned_leaf_sha256(&self) -> &[[u8; 32]] {
        &self.pinned_leaf_sha256
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum ClientIdentity {
    Pem {
        certificate_chain: Pem,
        private_key: Pem,
    },
    Pkcs12 {
        der: Vec<u8>,
        password: String,
    },
}

impl std::fmt::Debug for ClientIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pem {
                certificate_chain,
                private_key,
            } => f
                .debug_struct("Pem")
                .field("certificate_chain", certificate_chain)
                .field("private_key", private_key)
                .finish(),
            Self::Pkcs12 { .. } => f
                .debug_struct("Pkcs12")
                .field("der", &"***")
                .field("password", &"***")
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum Pem {
    File(PathBuf),
    Bytes(Vec<u8>),
}

impl std::fmt::Debug for Pem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(path) => f.debug_tuple("File").field(path).finish(),
            Self::Bytes(_) => f.write_str("Bytes(***)"),
        }
    }
}
