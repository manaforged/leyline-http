use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use leyline_bssl::ssl::{
    NameType, Ssl, SslAlert, SslContextBuilder, SslFiletype, SslRef, SslVerifyError, SslVerifyMode,
};
use leyline_bssl::x509::{X509, X509StoreContext};
use sha2::{Digest, Sha256};

use crate::tls::error::TlsError;

mod env;
#[cfg(not(target_os = "macos"))]
mod system;

#[cfg(test)]
use env::collect_ca_dir_candidates;
use env::wire_env_trust;
#[cfg(not(target_os = "macos"))]
use system::wire_system_trust_cached;

#[derive(Debug, Clone)]
pub struct TlsTrustConfig {
    use_env_roots: bool,
    use_system_roots: bool,
    ca_files: Vec<PathBuf>,
    ca_der: Vec<Vec<u8>>,
    client_identity: Option<ClientIdentity>,
    pinned_leaf_sha256: Vec<[u8; 32]>,
    accept_invalid_certs: bool,
}

pub(crate) type VerificationFailure = Arc<Mutex<Option<TrustFailure>>>;

#[derive(Clone, Copy, Debug)]
pub(crate) enum TrustFailure {
    Certificate,
    Hostname,
    Pinning,
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
        }
    }
}

impl TlsTrustConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn without_env_roots(mut self) -> Self {
        self.use_env_roots = false;
        self
    }

    pub fn without_system_roots(mut self) -> Self {
        self.use_system_roots = false;
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

pub(crate) fn install_verifier_ctx(
    builder: &mut SslContextBuilder,
    pins: &[[u8; 32]],
    host: Option<&str>,
    system_roots: bool,
) {
    let pins = pins.to_vec();
    let host = host.map(str::to_owned);
    builder.set_custom_verify_callback(SslVerifyMode::PEER, move |ssl| {
        let hostname = host
            .as_deref()
            .or_else(|| ssl.servername(NameType::HOST_NAME))
            .ok_or(SslVerifyError::Invalid(SslAlert::CERTIFICATE_UNKNOWN))?;
        verify(ssl, hostname, &pins, system_roots)
            .map_err(|_| SslVerifyError::Invalid(SslAlert::CERTIFICATE_UNKNOWN))
    });
}

fn trusted(ssl: &SslRef, _host: &str, _system_roots: bool) -> Result<bool, TrustFailure> {
    let cert = ssl.peer_certificate().ok_or(TrustFailure::Certificate)?;
    let chain = ssl.peer_cert_chain().ok_or(TrustFailure::Certificate)?;
    let configured = X509StoreContext::new()
        .map_err(|_| TrustFailure::Certificate)?
        .init(ssl.ssl_context().cert_store(), &cert, chain, |context| {
            Ok(context.verify_cert()? && context.verify_result().is_ok())
        })
        .map_err(|_| TrustFailure::Certificate)?;
    if configured {
        return Ok(true);
    }
    #[cfg(target_os = "macos")]
    if _system_roots {
        return super::macos_trust::verify(ssl, _host).map_err(|error| {
            tracing::debug!(target: "leyline::tls::trust", %error, "macOS certificate evaluation failed");
            TrustFailure::Certificate
        });
    }
    Ok(false)
}

fn verify(
    ssl: &SslRef,
    host: &str,
    pins: &[[u8; 32]],
    system_roots: bool,
) -> Result<(), TrustFailure> {
    let cert = ssl.peer_certificate().ok_or(TrustFailure::Certificate)?;
    let hostname_matches = match host.parse::<std::net::IpAddr>() {
        Ok(_) => cert.check_ip_asc(host),
        Err(_) => cert.check_host(host),
    }
    .map_err(|_| TrustFailure::Hostname)?;
    if !hostname_matches {
        return Err(TrustFailure::Hostname);
    }
    if !trusted(ssl, host, system_roots)? {
        return Err(TrustFailure::Certificate);
    }
    if !pins.is_empty() {
        let der = cert.to_der().map_err(|_| TrustFailure::Certificate)?;
        let digest: [u8; 32] = Sha256::digest(&der).into();
        if !pins.contains(&digest) {
            return Err(TrustFailure::Pinning);
        }
    }
    Ok(())
}

pub(crate) fn take_verification_failure(failure: &VerificationFailure) -> Option<TrustFailure> {
    failure.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn record_verification_failure(failure: &VerificationFailure, reason: TrustFailure) {
    *failure.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ClientIdentity {
    pub certificate_chain_file: PathBuf,
    pub private_key_file: PathBuf,
}

pub(crate) fn wire_configured_trust(
    builder: &mut SslContextBuilder,
    config: &TlsTrustConfig,
) -> Result<(), TlsError> {
    #[cfg(target_os = "macos")]
    if config.use_env_roots {
        wire_env_trust(builder);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let env_roots_loaded = config.use_env_roots && wire_env_trust(builder);
        if config.use_system_roots
            && let Err(err) = wire_system_trust_cached(builder, config, env_roots_loaded)
        {
            tracing::warn!(
                target: "leyline::tls::trust",
                %err,
                "system trust roots could not be loaded; certificate verification will fail"
            );
        }
    }

    for path in &config.ca_files {
        builder.set_ca_file(path).map_err(TlsError::from_stack)?;
    }

    if !config.ca_der.is_empty() {
        let store = builder.cert_store_mut();
        for der in &config.ca_der {
            store
                .add_cert(X509::from_der(der).map_err(TlsError::from_stack)?)
                .map_err(TlsError::from_stack)?;
        }
    }

    #[cfg(target_os = "macos")]
    if config.use_system_roots {
        install_verifier_ctx(builder, &config.pinned_leaf_sha256, None, true);
    }

    if let Some(identity) = &config.client_identity {
        builder
            .set_certificate_chain_file(&identity.certificate_chain_file)
            .map_err(TlsError::from_stack)?;
        builder
            .set_private_key_file(&identity.private_key_file, SslFiletype::PEM)
            .map_err(TlsError::from_stack)?;
    }

    Ok(())
}

pub(crate) fn install_verifier(
    ssl: &mut Ssl,
    pins: &[[u8; 32]],
    host: &str,
    system_roots: bool,
) -> VerificationFailure {
    let pins = pins.to_vec();
    let host = host.to_owned();
    let failure = Arc::new(Mutex::new(None));
    let callback_failure = failure.clone();
    ssl.set_custom_verify_callback(SslVerifyMode::PEER, move |ssl| {
        verify(ssl, &host, &pins, system_roots).map_err(|reason| {
            record_verification_failure(&callback_failure, reason);
            SslVerifyError::Invalid(SslAlert::CERTIFICATE_UNKNOWN)
        })
    });
    failure
}

#[cfg(test)]
mod ca_dir_tests;
