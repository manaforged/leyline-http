use leyline_bssl::ssl::{SslContextBuilder, SslVerifyMode};

#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
use crate::profile::BrowserProfile;
use crate::profile::{TlsMinVersion, TlsProfile};

use crate::tls::error::TlsError;
use crate::tls::keylog::install_from_env;
use crate::tls::trust::{Tls, TrustIdentity, wire_configured_trust};

#[cfg(leyline_any_codec)]
mod cert;
mod phase;

/// The transport a TLS context serves: TCP with the minimum used when the profile declares none, or QUIC, which needs TLS 1.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transport {
    Tcp(TlsMinVersion),
    Quic,
}

/// Builds a TCP TLS context for `profile` with default trust and `min_version` as the fallback floor.
#[cfg(any(leyline_unstable_bssl, feature = "bench-internals"))]
pub(crate) fn ssl_context(
    profile: &BrowserProfile,
    min_version: TlsMinVersion,
) -> Result<SslContextBuilder, TlsError> {
    let mut builder = SslContextBuilder::new(leyline_bssl::ssl::SslMethod::tls())
        .map_err(TlsError::from_stack)?;
    let trust = Tls::default();
    let transport = Transport::Tcp(min_version);
    let identity = apply_tls_with_trust(&mut builder, &profile.tls, transport, &trust)?;
    drop(identity);
    crate::tls::trust::install_verifier_ctx(&mut builder, &trust, None, transport);
    Ok(builder)
}

pub(crate) fn apply_tls_with_trust(
    builder: &mut SslContextBuilder,
    tls: &TlsProfile,
    transport: Transport,
    trust: &Tls,
) -> Result<TrustIdentity, TlsError> {
    phase::ciphers(builder, tls)?;
    phase::curves(builder, tls)?;
    phase::sigalgs(builder, tls)?;
    phase::extensions(builder, tls)?;
    phase::versions(builder, tls, transport, trust.min_tls_version)?;

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
            Some(_) => tls12.push(cipher.as_str()),
            None => return Err(TlsError::Profile(format!("unknown cipher: {cipher}"))),
        }
    }
    Ok((tls13, tls12))
}

#[cfg(not(all(
    feature = "compression-brotli",
    feature = "compression-zstd",
    any(feature = "compression-gzip", feature = "compression-deflate")
)))]
fn missing(algo: &str, codec: crate::profile::Encoding) -> TlsError {
    TlsError::Profile(format!(
        "profile requests {algo} certificate decompression but the `{}` feature is not compiled in",
        codec.feature()
    ))
}

#[cfg(test)]
mod tests;
