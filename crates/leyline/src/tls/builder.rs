//! Shared BoringSSL `SslContextBuilder` factory for fingerprint-correct
//! TLS handshakes.
//!
//! Every browser profile's cipher list, curve list, signature algorithm list,
//! certificate compression, delegated credentials, record size limit,
//! extension permutation, and GREASE setup is applied here. Both the H2 path
//! (via [`FingerprintConnector`]) and the H3 path (via `leyline-quic`'s
//! quiche integration) call into this factory so they produce ClientHellos
//! from the *same* BoringSSL context — any fingerprint change applied here
//! automatically propagates to both transports.
//!
//! [`FingerprintConnector`]: crate::FingerprintConnector

use btls::ssl::{
    CertificateCompressionAlgorithm, CertificateCompressor, SslContextBuilder, SslMethod,
    SslVerifyMode,
};

use crate::profile::BrowserProfile;

use crate::tls::error::TlsError;

/// Minimum TLS version pinned on the context. The TCP path allows 1.2+ to
/// match real browser behaviour against legacy servers; the QUIC path must
/// pin 1.3 per RFC 9001 §4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMinVersion {
    /// Allow TLS 1.2+. Used for the H1/H2 path.
    Tls12,
    /// Require TLS 1.3. Used for the H3/QUIC path.
    Tls13,
}

/// Build an `SslContextBuilder` preloaded with every TLS-level knob from
/// the given profile. Transport-specific additions (ALPN, session cache
/// callbacks, QUIC method) are the caller's responsibility.
///
/// Most users should reach for the shorter `leyline::tls_context` /
/// `leyline::quic_context` helpers instead of calling this directly.
pub fn build_ssl_context(
    profile: &BrowserProfile,
    min_version: TlsMinVersion,
) -> Result<SslContextBuilder, TlsError> {
    let mut builder = SslContextBuilder::new(SslMethod::tls())?;
    apply_profile(&mut builder, profile, min_version)?;
    Ok(builder)
}

/// Apply every profile-driven TLS knob to an already-constructed
/// `SslContextBuilder`. Used by the connector path (which starts from
/// `SslConnector::builder`) and by the QUIC path (which starts from
/// `SslContextBuilder::new`) so both end up with identical state.
pub(crate) fn apply_profile(
    builder: &mut SslContextBuilder,
    profile: &BrowserProfile,
    min_version: TlsMinVersion,
) -> Result<(), TlsError> {
    let tls = &profile.tls;

    // Cipher suites.
    let cipher_str = tls.ciphers.join(":");
    builder.set_cipher_list(&cipher_str)?;

    // Curves / supported_groups.
    let curves_str = tls
        .curves
        .iter()
        .map(|c| boring_curve_name(c))
        .collect::<Vec<_>>()
        .join(":");
    builder.set_curves_list(&curves_str)?;

    // Signature algorithms.
    let sigalgs_str = tls.sigalgs.join(":");
    builder.set_sigalgs_list(&sigalgs_str)?;

    // OCSP stapling (status_request extension).
    if tls.ocsp_stapling {
        builder.enable_ocsp_stapling();
    }

    // Signed certificate timestamps.
    if tls.signed_cert_timestamps {
        builder.enable_signed_cert_timestamps();
    }

    // Certificate compression (Brotli is the only one real browsers use).
    for algo in &tls.cert_compression {
        if algo == "brotli" {
            builder.add_certificate_compression_algorithm(BrotliDecompressor)?;
        }
    }

    // Delegated credentials (Firefox advertises this extension).
    if let Some(ref dc) = tls.delegated_credentials {
        builder.set_delegated_credentials(dc)?;
    }

    // Record size limit (Firefox sends this extension).
    if let Some(limit) = tls.record_size_limit {
        builder.set_record_size_limit(limit);
    }

    // Extension permutation (Chrome-style random ordering).
    if tls.permute_extensions {
        builder.set_permute_extensions(true);
    }

    // GREASE — always enabled for Chromium/Firefox.
    builder.set_grease_enabled(true);

    // Minimum TLS version.
    let min = match min_version {
        TlsMinVersion::Tls12 => btls::ssl::SslVersion::TLS1_2,
        TlsMinVersion::Tls13 => btls::ssl::SslVersion::TLS1_3,
    };
    builder.set_min_proto_version(Some(min))?;

    // Trust-root wiring lives in `crate::tls::trust` — env overrides
    // (`SSL_CERT_FILE` / `SSL_CERT_DIR`), the Windows system-store
    // bridge, and the Unix `set_default_verify_paths` fallback are all
    // concentrated there.
    if !crate::tls::trust::wire_env_trust(builder) {
        crate::tls::trust::wire_system_trust(builder)?;
    }
    builder.set_verify(SslVerifyMode::PEER);

    Ok(())
}

/// Map profile curve names to BoringSSL curve names.
fn boring_curve_name(name: &str) -> &str {
    match name {
        "X25519_MLKEM768" => "X25519MLKEM768",
        "X25519" => "X25519",
        "SECP256R1" => "P-256",
        "SECP384R1" => "P-384",
        "SECP521R1" => "P-521",
        other => other,
    }
}

/// Brotli cert decompression (advertises `compress_certificate` extension).
#[derive(Debug)]
struct BrotliDecompressor;

// btls 0.5.6 reshaped CertificateCompressor: `algorithm()` became the
// associated const `ALGORITHM`, `CAN_COMPRESS` / `CAN_DECOMPRESS` were
// added as required const flags, and `compress` / `decompress` take a
// generic writer instead of `&mut dyn io::Write`. We only need
// decompression; the trait's default `compress` impl (returns "not
// implemented") is fine, and the `CAN_COMPRESS = false` flag is enough
// for btls to skip registering us on the compress side.
impl CertificateCompressor for BrotliDecompressor {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::BROTLI;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W>(&self, input: &[u8], output: &mut W) -> std::io::Result<()>
    where
        W: std::io::Write,
    {
        let mut decoder = brotli::Decompressor::new(input, 4096);
        std::io::copy(&mut decoder, output)?;
        Ok(())
    }
}
