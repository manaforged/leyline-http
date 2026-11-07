use std::sync::{Arc, Mutex};

use leyline_bssl::ssl::{
    NameType, Ssl, SslAlert, SslContextBuilder, SslRef, SslVerifyError, SslVerifyMode,
};
use leyline_bssl::x509::{X509, X509Purpose, X509StoreContext};
use sha2::{Digest, Sha256};

use crate::tls::builder::Transport;
use crate::tls::error::TlsError;

mod config;
mod env;
mod files;
mod identity;
#[cfg(not(target_os = "macos"))]
mod system;

#[cfg(test)]
use env::collect_ca_dir_candidates;
use env::{env_roots_set, wire_env_trust};
#[cfg(not(target_os = "macos"))]
use system::wire_system_trust;

pub use config::Tls;
pub(crate) use identity::{TrustIdentity, TrustKey};

pub(crate) type VerificationFailure = Arc<Mutex<Option<TrustFailure>>>;

#[derive(Clone, Copy, Debug)]
pub(crate) enum TrustFailure {
    Certificate,
    Hostname,
    Pinning,
}

fn needs_verifier(trust: &Tls, host: Option<&str>, transport: Transport) -> bool {
    let unchecked_ip = transport == Transport::Quic
        && host.is_some_and(|host| host.parse::<std::net::IpAddr>().is_ok());
    !trust.accept_invalid_certs
        && (!trust.pinned_leaf_sha256.is_empty()
            || unchecked_ip
            || cfg!(target_os = "macos") && trust.use_system_roots)
}

fn verify_callback(
    trust: &Tls,
    host: Option<&str>,
) -> (
    impl Fn(&mut SslRef) -> Result<(), SslVerifyError> + Send + Sync + 'static,
    VerificationFailure,
) {
    let pins = trust.pinned_leaf_sha256.clone();
    let system_roots = trust.use_system_roots;
    let host = host.map(str::to_owned);
    let failure = VerificationFailure::default();
    let slot = failure.clone();
    let callback = move |ssl: &mut SslRef| {
        let hostname = host
            .as_deref()
            .or_else(|| ssl.servername(NameType::HOST_NAME))
            .ok_or(SslVerifyError::Invalid(SslAlert::CERTIFICATE_UNKNOWN))?;
        verify(ssl, hostname, &pins, system_roots).map_err(|reason| {
            tracing::debug!(target: "leyline::tls::trust", ?reason, "peer certificate rejected");
            *crate::util::lock(&slot) = Some(reason);
            SslVerifyError::Invalid(SslAlert::CERTIFICATE_UNKNOWN)
        })
    };
    (callback, failure)
}

/// Installs the custom verifier on a context when `trust` needs one for `transport`.
#[cfg(any(feature = "http3", feature = "bench-internals", leyline_unstable_bssl))]
pub(crate) fn install_verifier_ctx(
    builder: &mut SslContextBuilder,
    trust: &Tls,
    host: Option<&str>,
    transport: Transport,
) {
    if needs_verifier(trust, host, transport) {
        let (callback, _) = verify_callback(trust, host);
        builder.set_custom_verify_callback(SslVerifyMode::PEER, callback);
    }
}

/// Installs the custom verifier on one connection when `trust` needs one, returning where its failure lands.
pub(crate) fn install_verifier(
    ssl: &mut Ssl,
    trust: &Tls,
    host: &str,
    transport: Transport,
) -> Option<VerificationFailure> {
    if !needs_verifier(trust, Some(host), transport) {
        return None;
    }
    let (callback, failure) = verify_callback(trust, Some(host));
    ssl.set_custom_verify_callback(SslVerifyMode::PEER, callback);
    Some(failure)
}

fn rejected(error: &leyline_bssl::error::ErrorStack, failure: TrustFailure) -> TrustFailure {
    tracing::debug!(target: "leyline::tls::trust", %error, ?failure, "certificate check failed");
    failure
}

fn trusted(ssl: &SslRef, _host: &str, _system_roots: bool) -> Result<bool, TrustFailure> {
    let cert = ssl.peer_certificate().ok_or(TrustFailure::Certificate)?;
    let chain = ssl.peer_cert_chain().ok_or(TrustFailure::Certificate)?;
    let configured = X509StoreContext::new()
        .map_err(|error| rejected(&error, TrustFailure::Certificate))?
        .init(ssl.ssl_context().cert_store(), &cert, chain, |context| {
            context.set_purpose(X509Purpose::SSL_SERVER)?;
            Ok(context.verify_cert()? && context.verify_result().is_ok())
        })
        .map_err(|error| rejected(&error, TrustFailure::Certificate))?;
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
    .map_err(|error| rejected(&error, TrustFailure::Hostname))?;
    if !hostname_matches {
        return Err(TrustFailure::Hostname);
    }
    if !trusted(ssl, host, system_roots)? {
        return Err(TrustFailure::Certificate);
    }
    if !pins.is_empty() {
        let der = cert
            .to_der()
            .map_err(|error| rejected(&error, TrustFailure::Certificate))?;
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

pub(crate) fn wire_configured_trust(
    builder: &mut SslContextBuilder,
    config: &Tls,
) -> Result<TrustIdentity, TlsError> {
    let mut key = TrustKey::of(config);
    #[cfg(target_os = "macos")]
    if config.use_env_roots {
        wire_env_trust(builder, &mut key);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let env_roots_loaded = config.use_env_roots && wire_env_trust(builder, &mut key);
        if config.use_system_roots
            && let Err(err) = wire_system_trust(builder, config, env_roots_loaded)
        {
            tracing::warn!(
                target: "leyline::tls::trust",
                %err,
                "system trust roots could not be loaded; certificate verification will fail"
            );
        }
    }

    for path in &config.ca_files {
        files::add_root_file(builder, path, &mut key)?;
    }

    if !config.ca_der.is_empty() {
        let store = builder.cert_store_mut();
        for der in &config.ca_der {
            store
                .add_cert(X509::from_der(der).map_err(TlsError::from_stack)?)
                .map_err(TlsError::from_stack)?;
        }
    }

    if let Some(identity) = &config.client_identity {
        files::set_client_identity(builder, identity, &mut key)?;
    }

    Ok(key.identity())
}

#[cfg(test)]
mod ca_dir_tests;
