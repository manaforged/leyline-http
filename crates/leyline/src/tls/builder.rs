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

use leyline_bssl::ssl::{
    CertificateCompressionAlgorithm, CertificateCompressor, SslContextBuilder, SslMethod,
    SslVerifyMode,
};

use crate::profile::BrowserProfile;

use crate::tls::error::TlsError;
use crate::tls::trust::TlsTrustConfig;

/// Minimum TLS version pinned on the context. The TCP path allows 1.2+ to
/// match real browser behaviour against legacy servers; the QUIC path must
/// pin 1.3 per RFC 9001 §4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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
    apply_profile_with_trust(builder, profile, min_version, &TlsTrustConfig::default())
}

/// Apply every profile-driven TLS knob with explicit trust settings.
pub(crate) fn apply_profile_with_trust(
    builder: &mut SslContextBuilder,
    profile: &BrowserProfile,
    min_version: TlsMinVersion,
    trust: &TlsTrustConfig,
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

    // Signature algorithms — raw codepoints so ML-DSA (0x0904/05/06), which
    // BoringSSL has no name for, advertises by value. The same map feeds JA4.
    let sigalgs = tls
        .sigalgs
        .iter()
        .map(|name| {
            crate::audit::sigalg_id(name)
                .ok_or_else(|| TlsError::Profile(format!("unknown signature algorithm: {name}")))
        })
        .collect::<Result<Vec<u16>, _>>()?;
    builder.set_sigalgs(&sigalgs)?;

    // OCSP stapling (status_request extension).
    if tls.ocsp_stapling {
        builder.enable_ocsp_stapling();
    }

    // Signed certificate timestamps.
    if tls.signed_cert_timestamps {
        builder.enable_signed_cert_timestamps();
    }

    // Certificate compression (RFC 8879 `compress_certificate` extension).
    // The set and order of advertised codepoints is fingerprint-bearing —
    // Firefox advertises zlib(1), brotli(2), zstd(3). Each algorithm is
    // registered with a real decompressor so the advertisement is honest and
    // a server that actually compresses its certificate is handled. A
    // genuinely unknown name is a profile typo → error.
    for algo in &tls.cert_compression {
        match algo.as_str() {
            "brotli" => builder.add_certificate_compression_algorithm(BrotliDecompressor)?,
            "zlib" => builder.add_certificate_compression_algorithm(ZlibDecompressor)?,
            "zstd" => builder.add_certificate_compression_algorithm(ZstdDecompressor)?,
            other => {
                return Err(TlsError::Profile(format!(
                    "unknown cert compression algorithm: {other:?}"
                )));
            }
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
        TlsMinVersion::Tls12 => leyline_bssl::ssl::SslVersion::TLS1_2,
        TlsMinVersion::Tls13 => leyline_bssl::ssl::SslVersion::TLS1_3,
    };
    builder.set_min_proto_version(Some(min))?;

    // Trust-root wiring lives in `crate::tls::trust` — env overrides
    // (`SSL_CERT_FILE` / `SSL_CERT_DIR`), the Windows system-store
    // bridge, and the Unix `set_default_verify_paths` fallback are all
    // concentrated there.
    crate::tls::trust::wire_configured_trust(builder, trust)?;
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

/// zlib (RFC 1950) cert decompression — advertises codepoint 1, which
/// Firefox sends first in its `compress_certificate` extension.
#[derive(Debug)]
struct ZlibDecompressor;

impl CertificateCompressor for ZlibDecompressor {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZLIB;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W>(&self, input: &[u8], output: &mut W) -> std::io::Result<()>
    where
        W: std::io::Write,
    {
        let mut decoder = flate2::read::ZlibDecoder::new(input);
        std::io::copy(&mut decoder, output)?;
        Ok(())
    }
}

/// zstd (RFC 8878) cert decompression — advertises codepoint 3.
#[derive(Debug)]
struct ZstdDecompressor;

impl CertificateCompressor for ZstdDecompressor {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZSTD;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W>(&self, input: &[u8], output: &mut W) -> std::io::Result<()>
    where
        W: std::io::Write,
    {
        let mut decoder = zstd::stream::read::Decoder::new(input)?;
        std::io::copy(&mut decoder, output)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leyline_bssl::ssl::CertificateCompressor;
    use std::io::Write;

    // The decompressors aren't decorative: a server that compresses its
    // certificate with the codepoint we advertise must actually be
    // decodable. Round-trip a known payload through each.
    #[test]
    fn zlib_decompressor_round_trips() {
        let original = b"-----BEGIN CERTIFICATE----- leyline zlib roundtrip";
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(original).unwrap();
        let compressed = enc.finish().unwrap();

        let mut out = Vec::new();
        ZlibDecompressor.decompress(&compressed, &mut out).unwrap();
        assert_eq!(out, original);
    }

    #[test]
    fn zstd_decompressor_round_trips() {
        let original = b"-----BEGIN CERTIFICATE----- leyline zstd roundtrip";
        let compressed = zstd::stream::encode_all(&original[..], 3).unwrap();

        let mut out = Vec::new();
        ZstdDecompressor.decompress(&compressed, &mut out).unwrap();
        assert_eq!(out, original);
    }
}
