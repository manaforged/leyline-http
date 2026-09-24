use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use leyline_bssl::ssl::{
    NameType, Ssl, SslAlert, SslContextBuilder, SslFiletype, SslRef, SslVerifyError, SslVerifyMode,
};
use leyline_bssl::x509::{X509, X509StoreContext};
use sha2::{Digest, Sha256};

use crate::tls::error::TlsError;

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

    pub fn client_identity_files(
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

    pub fn uses_env_roots(&self) -> bool {
        self.use_env_roots
    }

    pub fn uses_system_roots(&self) -> bool {
        self.use_system_roots
    }

    pub fn ca_files(&self) -> &[PathBuf] {
        &self.ca_files
    }

    pub fn ca_der_count(&self) -> usize {
        self.ca_der.len()
    }

    pub fn client_identity(&self) -> Option<&ClientIdentity> {
        self.client_identity.as_ref()
    }

    pub fn pinned_leaf_sha256(&self) -> &[[u8; 32]] {
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

pub(crate) fn wire_env_trust(builder: &mut SslContextBuilder) -> bool {
    let mut loaded_any = false;
    if let Ok(file) = std::env::var("SSL_CERT_FILE") {
        let file = file.trim();
        if !file.is_empty() {
            match builder.set_ca_file(file) {
                Ok(()) => {
                    loaded_any = true;
                    tracing::warn!(
                        target: "leyline::tls::trust",
                        ca_file = %file,
                        "SSL_CERT_FILE honoured — environment trust root added"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        target: "leyline::tls::trust",
                        ca_file = %file,
                        err = %e,
                        "SSL_CERT_FILE could not be loaded"
                    );
                }
            }
        }
    }
    if let Ok(dir) = std::env::var("SSL_CERT_DIR") {
        let dir = dir.trim();
        if !dir.is_empty() {
            let dir_path = std::path::PathBuf::from(dir);
            if dir_path.is_dir() {
                let candidates = collect_ca_dir_candidates(&dir_path);
                if !candidates.is_empty() || dir_path.exists() {
                    let mut loaded = 0usize;
                    for p in &candidates {
                        match builder.set_ca_file(p) {
                            Ok(()) => loaded += 1,
                            Err(e) => {
                                tracing::debug!(
                                    target: "leyline::tls::trust",
                                    ca_file = %p.display(),
                                    err = %e,
                                    "SSL_CERT_DIR entry skipped"
                                );
                            }
                        }
                    }
                    if loaded > 0 {
                        loaded_any = true;
                        tracing::warn!(
                            target: "leyline::tls::trust",
                            ca_dir = %dir,
                            files_loaded = loaded,
                            "SSL_CERT_DIR honoured — environment trust roots added"
                        );
                    } else {
                        tracing::warn!(
                            target: "leyline::tls::trust",
                            ca_dir = %dir,
                            "SSL_CERT_DIR contained no loadable certificates"
                        );
                    }
                }
            } else {
                tracing::warn!(
                    target: "leyline::tls::trust",
                    ca_dir = %dir,
                    "SSL_CERT_DIR does not exist"
                );
            }
        }
    }
    loaded_any
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

#[cfg(not(target_os = "macos"))]
pub(crate) fn wire_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    #[cfg(windows)]
    {
        wire_windows_system_trust(builder)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        wire_linux_system_trust(builder)
    }
}

#[cfg(windows)]
fn wire_system_trust_cached(
    builder: &mut SslContextBuilder,
    config: &TlsTrustConfig,
    env_roots_loaded: bool,
) -> Result<(), TlsError> {
    if !env_roots_loaded && config.ca_files.is_empty() && config.ca_der.is_empty() {
        builder.set_cert_store_ref(cached_windows_system_store()?);
        return Ok(());
    }
    wire_system_trust(builder)
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn wire_system_trust_cached(
    builder: &mut SslContextBuilder,
    _config: &TlsTrustConfig,
    _env_roots_loaded: bool,
) -> Result<(), TlsError> {
    wire_system_trust(builder)
}

#[cfg(windows)]
fn cached_windows_system_store() -> Result<&'static leyline_bssl::x509::store::X509Store, TlsError>
{
    use leyline_bssl::x509::store::{X509Store, X509StoreBuilder};
    use std::sync::OnceLock;

    static STORE: OnceLock<X509Store> = OnceLock::new();
    if let Some(s) = STORE.get() {
        return Ok(s);
    }

    let roots = crate::tls::windows_trust::load_system_roots().map_err(|e| {
        TlsError::TrustStore(format!("failed to open Windows system ROOT store: {e}"))
    })?;
    let mut store = X509StoreBuilder::new()
        .map_err(|e| TlsError::TrustStore(format!("X509 store allocation failed: {e}")))?;
    let mut loaded = 0usize;
    for der in &roots {
        if let Ok(cert) = X509::from_der(der) {
            if store.add_cert(cert).is_ok() {
                loaded += 1;
            }
        }
    }
    if loaded == 0 {
        return Err(TlsError::TrustStore(
            "Windows system ROOT store bridged zero certificates".into(),
        ));
    }
    tracing::info!(
        target: "leyline::tls::trust",
        loaded,
        "Windows system ROOT store parsed and cached for reuse"
    );
    Ok(STORE.get_or_init(|| store.build()))
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn wire_linux_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    const BUNDLES: &[&str] = &[
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
        "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
        "/etc/ssl/ca-bundle.pem",
        "/etc/ssl/cert.pem",
    ];
    for &path in BUNDLES {
        if std::path::Path::new(path).exists() {
            return builder
                .set_ca_file(path)
                .map_err(|e| TlsError::TrustStore(format!("failed to load {path}: {e}")));
        }
    }
    Err(TlsError::TrustStore(
        "no supported Linux system CA bundle found".into(),
    ))
}

#[cfg(windows)]
fn wire_windows_system_trust(builder: &mut SslContextBuilder) -> Result<(), TlsError> {
    use leyline_bssl::x509::X509;

    let roots = match crate::tls::windows_trust::load_system_roots() {
        Ok(r) => r,
        Err(e) => {
            return Err(TlsError::TrustStore(format!(
                "failed to open Windows system ROOT store: {e}"
            )));
        }
    };

    let store = builder.cert_store_mut();
    let mut loaded = 0usize;
    let mut skipped = 0usize;
    for der in &roots {
        match X509::from_der(der) {
            Ok(cert) => match store.add_cert(cert) {
                Ok(()) => loaded += 1,
                Err(e) => {
                    skipped += 1;
                    tracing::debug!(
                        target: "leyline::tls::trust",
                        err = %e,
                        "Windows ROOT cert rejected by BoringSSL store"
                    );
                }
            },
            Err(e) => {
                skipped += 1;
                tracing::debug!(
                    target: "leyline::tls::trust",
                    err = %e,
                    "Windows ROOT cert failed DER parse"
                );
            }
        }
    }

    if loaded == 0 {
        return Err(TlsError::TrustStore(format!(
            "Windows system ROOT store bridged zero certificates ({skipped} skipped)"
        )));
    }
    tracing::info!(
        target: "leyline::tls::trust",
        loaded,
        skipped,
        "Windows system ROOT store bridged into BoringSSL"
    );
    Ok(())
}

pub(crate) fn collect_ca_dir_candidates(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let ext_ok = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "pem" | "crt" | "cer"))
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }
        let Ok(resolved) = std::fs::metadata(&p) else {
            continue;
        };
        if !resolved.is_file() {
            continue;
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod ca_dir_tests;
