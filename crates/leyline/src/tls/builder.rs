//! Shared BoringSSL `SslContextBuilder` factory for fingerprint-correct TLS handshakes.

use leyline_bssl::ssl::{
    CertificateCompressionAlgorithm, CertificateCompressor, SslContextBuilder, SslMethod,
    SslOptions, SslVerifyMode,
};

use crate::profile::BrowserProfile;

use crate::tls::error::TlsError;
use crate::tls::trust::TlsTrustConfig;

/// Minimum TLS version pinned on the context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TlsMinVersion {
    /// Allow TLS 1.0+ (CFNetwork iOS advertises 1.0/1.1 in supported_versions).
    Tls10,
    /// Allow TLS 1.2+.
    Tls12,
    /// Require TLS 1.3.
    Tls13,
}

/// Build an `SslContextBuilder` preloaded with every TLS-level knob from the given profile.
pub fn build_ssl_context(
    profile: &BrowserProfile,
    min_version: TlsMinVersion,
) -> Result<SslContextBuilder, TlsError> {
    let mut builder = SslContextBuilder::new(SslMethod::tls())?;
    apply_profile(&mut builder, profile, min_version)?;
    Ok(builder)
}

/// Apply every profile-driven TLS knob to an already-constructed `SslContextBuilder`.
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

    let cipher_str = tls.ciphers.join(":");
    builder.set_cipher_list(&cipher_str)?;

    let tls13 = tls13_cipher_ids(&tls.ciphers)?;
    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        all(target_arch = "aarch64", target_os = "macos")
    ))]
    if !tls13.is_empty() {
        builder.set_tls13_cipher_order(&tls13)?;
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "windows",
        all(target_arch = "aarch64", target_os = "macos")
    )))]
    if !tls13.is_empty() && tls.extension_permutation.is_some() {
        return Err(TlsError::Profile(
            "exact TLS order requires a rebuilt BoringSSL bundle for this target".into(),
        ));
    }

    let curves_str = tls
        .curves
        .iter()
        .map(|c| boring_curve_name(c))
        .collect::<Vec<_>>()
        .join(":");
    builder.set_curves_list(&curves_str)?;

    let sigalgs = tls
        .sigalgs
        .iter()
        .map(|name| {
            crate::audit::sigalg_id(name)
                .ok_or_else(|| TlsError::Profile(format!("unknown signature algorithm: {name}")))
        })
        .collect::<Result<Vec<u16>, _>>()?;
    if let Err(e) = builder.set_sigalgs(&sigalgs) {
        if !e.to_string().contains("DUPLICATE_SIGNATURE_ALGORITHM") {
            return Err(TlsError::SslConfig(e.to_string()));
        }
        let mut deduped = sigalgs.clone();
        deduped.dedup();
        builder.set_sigalgs(&deduped)?;
    }

    if tls.ocsp_stapling {
        builder.enable_ocsp_stapling();
    }

    if tls.signed_cert_timestamps {
        builder.enable_signed_cert_timestamps();
    }

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

    if let Some(ref dc) = tls.delegated_credentials {
        builder.set_delegated_credentials(dc)?;
    }

    if let Some(limit) = tls.record_size_limit {
        builder.set_record_size_limit(limit);
    }

    if tls.permute_extensions {
        builder.set_permute_extensions(true);
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "windows",
        all(target_arch = "aarch64", target_os = "macos")
    ))]
    if let Some(order) = &tls.extension_permutation {
        builder.set_extension_order(order)?;
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "windows",
        all(target_arch = "aarch64", target_os = "macos")
    )))]
    if tls.extension_permutation.is_some() {
        return Err(TlsError::Profile(
            "exact TLS extension order requires a rebuilt BoringSSL bundle for this target".into(),
        ));
    }

    builder.set_grease_enabled(tls.grease);

    if !tls.session_tickets {
        builder.set_options(SslOptions::NO_TICKET);
    }

    let min = if min_version == TlsMinVersion::Tls13 {
        TlsMinVersion::Tls13
    } else {
        match profile_min_version(&tls.min_tls_version) {
            Some(declared) => declared,
            None => min_version,
        }
    };
    let min = match min {
        TlsMinVersion::Tls10 => leyline_bssl::ssl::SslVersion::TLS1,
        TlsMinVersion::Tls12 => leyline_bssl::ssl::SslVersion::TLS1_2,
        TlsMinVersion::Tls13 => leyline_bssl::ssl::SslVersion::TLS1_3,
    };
    builder.set_min_proto_version(Some(min))?;

    crate::tls::trust::wire_configured_trust(builder, trust)?;
    builder.set_verify(SslVerifyMode::PEER);

    crate::tls::keylog::install_from_env(builder);

    Ok(())
}

fn tls13_cipher_ids(ciphers: &[String]) -> Result<Vec<u16>, TlsError> {
    ciphers
        .iter()
        .filter_map(|cipher| match cipher.as_str() {
            "TLS_AES_128_GCM_SHA256" => Some(Ok(0x1301)),
            "TLS_AES_256_GCM_SHA384" => Some(Ok(0x1302)),
            "TLS_CHACHA20_POLY1305_SHA256" => Some(Ok(0x1303)),
            _ if cipher.starts_with("TLS_AES_") || cipher.starts_with("TLS_CHACHA20_") => {
                Some(Err(TlsError::Profile(format!(
                    "unknown TLS 1.3 cipher: {cipher}"
                ))))
            }
            _ => None,
        })
        .collect()
}

/// Map profile curve names to BoringSSL curve names.
fn profile_min_version(declared: &Option<String>) -> Option<TlsMinVersion> {
    match declared.as_deref() {
        None => None,
        Some("1.0") => Some(TlsMinVersion::Tls10),
        Some("1.2") => Some(TlsMinVersion::Tls12),
        Some("1.3") => Some(TlsMinVersion::Tls13),
        Some(other) => {
            let _ = other;
            None
        }
    }
}

fn boring_curve_name(name: &str) -> &str {
    match name {
        "X25519_MLKEM768" => "X25519MLKEM768",
        "X25519_KYBER768" => "X25519Kyber768Draft00",
        "X25519" => "X25519",
        "SECP256R1" => "P-256",
        "SECP384R1" => "P-384",
        "SECP521R1" => "P-521",
        other => other,
    }
}

/// Upper bound on a decompressed peer certificate chain.
const MAX_CERT_DECOMPRESSED_BYTES: usize = 1024 * 1024;

/// Decompress a certificate blob with a hard output cap.
fn read_limited_cert<R: std::io::Read>(mut decoder: R) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = decoder.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        if out.len() + n > MAX_CERT_DECOMPRESSED_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "decompressed certificate exceeds limit",
            ));
        }
        out.extend_from_slice(&buf[..n]);
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
        let decoded = read_limited_cert(brotli::Decompressor::new(input, 4096))?;
        output.write_all(&decoded)
    }
}

/// zlib (RFC 1950) cert decompression — advertises codepoint 1, which Firefox sends first in its `compress_certificate` extension.
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
        let decoded = read_limited_cert(flate2::read::ZlibDecoder::new(input))?;
        output.write_all(&decoded)
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
        let decoder = zstd::stream::read::Decoder::new(input)?;
        let decoded = read_limited_cert(decoder)?;
        output.write_all(&decoded)
    }
}

#[cfg(test)]
mod tests;
