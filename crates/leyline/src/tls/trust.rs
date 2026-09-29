use std::sync::{Arc, Mutex};

use leyline_bssl::ssl::{
    NameType, Ssl, SslAlert, SslContextBuilder, SslFiletype, SslRef, SslVerifyError, SslVerifyMode,
};
use leyline_bssl::x509::{X509, X509Purpose, X509StoreContext};
use sha2::{Digest, Sha256};

use crate::tls::error::TlsError;

mod config;
mod env;
#[cfg(not(target_os = "macos"))]
mod system;

#[cfg(test)]
use env::collect_ca_dir_candidates;
use env::wire_env_trust;
#[cfg(not(target_os = "macos"))]
use system::wire_system_trust_cached;

pub use config::TlsTrustConfig;

pub(crate) type VerificationFailure = Arc<Mutex<Option<TrustFailure>>>;

#[derive(Clone, Copy, Debug)]
pub(crate) enum TrustFailure {
    Certificate,
    Hostname,
    Pinning,
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
            context.set_purpose(X509Purpose::SSL_SERVER)?;
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
    crate::util::lock(failure).take()
}

fn record_verification_failure(failure: &VerificationFailure, reason: TrustFailure) {
    *crate::util::lock(failure) = Some(reason);
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
