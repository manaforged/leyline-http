use leyline_bssl::ssl::{SslContextBuilder, SslVerifyMode};

#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
use crate::profile::BrowserProfile;
use crate::profile::TlsProfile;

use crate::tls::error::TlsError;
use crate::tls::keylog::install_from_env;
use crate::tls::trust::{Tls, TrustIdentity, wire_configured_trust};

#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
mod cert;
mod phase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TlsMinVersion {
    Tls10,
    Tls12,
    Tls13,
}

#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
impl BrowserProfile {
    pub(crate) fn build_ssl_context(
        &self,
        min_version: TlsMinVersion,
    ) -> Result<SslContextBuilder, TlsError> {
        let mut builder = SslContextBuilder::new(leyline_bssl::ssl::SslMethod::tls())
            .map_err(TlsError::from_stack)?;
        let identity = apply_tls_with_trust(&mut builder, &self.tls, min_version, &Tls::default())?;
        drop(identity);
        Ok(builder)
    }
}

pub(crate) fn apply_tls_with_trust(
    builder: &mut SslContextBuilder,
    tls: &TlsProfile,
    min_version: TlsMinVersion,
    trust: &Tls,
) -> Result<TrustIdentity, TlsError> {
    phase::ciphers(builder, tls)?;
    phase::curves(builder, tls)?;
    phase::sigalgs(builder, tls)?;
    phase::extensions(builder, tls)?;
    phase::versions(builder, tls, min_version, trust.min_tls_version)?;

    let identity = wire_configured_trust(builder, trust)?;
    builder.set_verify(SslVerifyMode::PEER);

    install_from_env(builder);

    Ok(identity)
}

fn partition_ciphers(ciphers: &[String]) -> Result<(Vec<u16>, Vec<&str>), TlsError> {
    let mut tls13 = Vec::new();
    let mut tls12 = Vec::new();
    for cipher in ciphers {
        match crate::iana::cipher_id(cipher) {
            Some(id) if crate::iana::is_tls13_cipher(id) => tls13.push(id),
            _ if cipher.starts_with("TLS_AES_") || cipher.starts_with("TLS_CHACHA20_") => {
                return Err(TlsError::Profile(format!(
                    "unknown TLS 1.3 cipher: {cipher}"
                )));
            }
            _ => tls12.push(cipher.as_str()),
        }
    }
    Ok((tls13, tls12))
}

fn profile_min_version(declared: &Option<String>) -> Result<Option<TlsMinVersion>, TlsError> {
    match declared.as_deref() {
        None => Ok(None),
        Some("1.0") => Ok(Some(TlsMinVersion::Tls10)),
        Some("1.2") => Ok(Some(TlsMinVersion::Tls12)),
        Some("1.3") => Ok(Some(TlsMinVersion::Tls13)),
        Some(other) => Err(TlsError::Profile(format!(
            "unsupported min_tls_version {other:?}; expected \"1.0\", \"1.2\", or \"1.3\""
        ))),
    }
}

#[cfg(not(all(
    feature = "compression-brotli",
    feature = "compression-zstd",
    any(feature = "compression-gzip", feature = "compression-deflate")
)))]
fn missing(algo: &str, feature: &str) -> TlsError {
    TlsError::Profile(format!(
        "profile requests {algo} certificate decompression but the `{feature}` feature is not compiled in"
    ))
}

#[cfg(test)]
mod tests;
