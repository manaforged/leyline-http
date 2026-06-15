//! rustls `ClientConfig` construction for the bare (non-fingerprint) TLS
//! backend.
//!
//! Bridges the same [`TlsTrustConfig`](crate::tls::trust::TlsTrustConfig)
//! the BoringSSL path uses — system roots, explicit CA files/DER, leaf
//! pinning, danger-accept-invalid, and mTLS client identity — onto rustls
//! types. The crypto provider is chosen in exactly one place
//! ([`crypto_provider`]) so the future `tls-rustls-rustcrypto` swap is a
//! single-function change.

use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::tls::error::TlsError;
use crate::tls::trust::{ClientIdentity, TlsTrustConfig};

/// The rustls crypto provider for the bare backend. aws-lc-rs today; the
/// `tls-rustls-rustcrypto` feature will swap this one function for the
/// pure-Rust provider (zero-C / clean musl cross-compile).
fn crypto_provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// Build a rustls [`ClientConfig`] from a [`TlsTrustConfig`], with the
/// given ALPN protocol list (e.g. `[b"h2", b"http/1.1"]`).
pub(crate) fn build_client_config(
    trust: &TlsTrustConfig,
    accept_invalid_certs: bool,
    alpn: &[&[u8]],
) -> Result<ClientConfig, TlsError> {
    let provider = crypto_provider();
    let versions = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::TrustStore(format!("rustls protocol versions: {e}")))?;

    let mut config = if accept_invalid_certs {
        versions
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify::new(provider.clone())))
            .with_no_client_auth()
    } else {
        let roots = Arc::new(build_root_store(trust)?);
        let pins = trust.pinned_leaf_sha256();
        let configured = if pins.is_empty() {
            versions.with_root_certificates(roots)
        } else {
            let inner = WebPkiServerVerifier::builder_with_provider(roots, provider.clone())
                .build()
                .map_err(|e| TlsError::TrustStore(format!("webpki verifier: {e}")))?;
            versions
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinningVerifier {
                    inner,
                    pins: pins.to_vec(),
                }))
        };
        match trust.client_identity() {
            Some(identity) => {
                let (certs, key) = load_client_identity(identity)?;
                configured
                    .with_client_auth_cert(certs, key)
                    .map_err(|e| TlsError::TrustStore(format!("mTLS client identity: {e}")))?
            }
            None => configured.with_no_client_auth(),
        }
    };

    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(config)
}

fn build_root_store(trust: &TlsTrustConfig) -> Result<RootCertStore, TlsError> {
    let mut roots = RootCertStore::empty();
    let mut loaded = 0usize;

    if trust.uses_system_roots() {
        let result = rustls_native_certs::load_native_certs();
        for cert in result.certs {
            if roots.add(cert).is_ok() {
                loaded += 1;
            }
        }
        for err in &result.errors {
            tracing::warn!(target: "leyline::tls", "native trust load issue: {err}");
        }
    }

    if trust.uses_env_roots() {
        if let Ok(file) = std::env::var("SSL_CERT_FILE") {
            let file = file.trim();
            if !file.is_empty() {
                loaded += add_pem_file(&mut roots, Path::new(file));
            }
        }
        if let Ok(dir) = std::env::var("SSL_CERT_DIR") {
            let dir = dir.trim();
            if !dir.is_empty() {
                for path in crate::tls::trust::collect_ca_dir_candidates(Path::new(dir)) {
                    loaded += add_pem_file(&mut roots, &path);
                }
            }
        }
    }

    for path in trust.ca_files() {
        loaded += add_pem_file(&mut roots, path);
    }
    for der in trust.ca_der() {
        if roots.add(CertificateDer::from(der.clone())).is_ok() {
            loaded += 1;
        }
    }

    if loaded == 0 {
        return Err(TlsError::TrustStore(
            "no trust roots available for the rustls backend".into(),
        ));
    }
    Ok(roots)
}

fn add_pem_file(roots: &mut RootCertStore, path: &Path) -> usize {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(target: "leyline::tls", "cannot read CA file {}: {e}", path.display());
            return 0;
        }
    };
    let mut reader = BufReader::new(&data[..]);
    let mut n = 0;
    for cert in rustls_pemfile::certs(&mut reader).flatten() {
        if roots.add(cert).is_ok() {
            n += 1;
        }
    }
    n
}

fn load_client_identity(
    identity: &ClientIdentity,
) -> Result<
    (
        Vec<CertificateDer<'static>>,
        rustls::pki_types::PrivateKeyDer<'static>,
    ),
    TlsError,
> {
    let cert_data = std::fs::read(&identity.certificate_chain_file)
        .map_err(|e| TlsError::TrustStore(format!("read client cert chain: {e}")))?;
    let certs = rustls_pemfile::certs(&mut BufReader::new(&cert_data[..]))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::TrustStore(format!("parse client cert chain: {e}")))?;
    let key_data = std::fs::read(&identity.private_key_file)
        .map_err(|e| TlsError::TrustStore(format!("read client key: {e}")))?;
    let key = rustls_pemfile::private_key(&mut BufReader::new(&key_data[..]))
        .map_err(|e| TlsError::TrustStore(format!("parse client key: {e}")))?
        .ok_or_else(|| TlsError::TrustStore("no private key in client identity file".into()))?;
    Ok((certs, key))
}

/// Pins the leaf certificate SHA-256 on top of full webpki chain +
/// hostname verification (unlike the BoringSSL path, rustls' webpki
/// verifier already checks the SAN, so no separate hostname re-check).
#[derive(Debug)]
struct PinningVerifier {
    inner: Arc<WebPkiServerVerifier>,
    pins: Vec<[u8; 32]>,
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verified = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        let digest = Sha256::digest(end_entity.as_ref());
        if self.pins.iter().any(|p| p.as_slice() == digest.as_slice()) {
            Ok(verified)
        } else {
            Err(rustls::Error::General(
                "leaf certificate is valid but its SHA-256 does not match any pin".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Accepts any server certificate. **Dangerous** — only via
/// `danger_accept_invalid_certs`. Signature checks still run through the
/// provider so a malformed handshake is rejected, but the chain/identity
/// is not validated.
#[derive(Debug)]
struct NoVerify {
    provider: Arc<CryptoProvider>,
}

impl NoVerify {
    fn new(provider: Arc<CryptoProvider>) -> Self {
        Self { provider }
    }
}

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
