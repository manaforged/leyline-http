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
//! [`FingerprintConnector`]: crate::tls::FingerprintConnector

use leyline_bssl::ssl::{
    CertificateCompressionAlgorithm, CertificateCompressor, SslContextBuilder, SslMethod,
    SslOptions, SslVerifyMode,
};

use crate::profile::BrowserProfile;

use crate::tls::error::TlsError;
use crate::tls::trust::TlsTrustConfig;

/// Minimum TLS version pinned on the context. The TCP path allows 1.2+ to
/// match real browser behaviour against legacy servers; the QUIC path must
/// pin 1.3 per RFC 9001 §4.2. The cfnetwork-ios profile lowers the TCP floor
/// to 1.0 (CFNetwork iOS advertises TLS 1.0/1.1 in supported_versions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TlsMinVersion {
    /// Allow TLS 1.0+ (CFNetwork iOS advertises 1.0/1.1 in supported_versions).
    Tls10,
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
    // Advertised via set1_sigalgs: the signing-prefs entry point rejects
    // duplicate codepoints, and real captures contain them (CFNetwork sends
    // 0x0805 twice — a wire fingerprint, not a bug).
    let sigalgs = tls
        .sigalgs
        .iter()
        .map(|name| {
            crate::audit::sigalg_id(name)
                .ok_or_else(|| TlsError::Profile(format!("unknown signature algorithm: {name}")))
        })
        .collect::<Result<Vec<u16>, _>>()?;
    // Some BoringSSL builds reject duplicate codepoints in the signing
    // prefs (real captures contain them: CFNetwork advertises 0x0805
    // twice). On those builds, fall back to the deduplicated list — the
    // ClientHello then advertises one 0x0805 instead of two, a documented
    // Linux-side fidelity gap until the prebuilt is regenerated. Builds
    // that accept the capture list keep it byte-for-byte.
    if let Err(e) = builder.set_sigalgs(&sigalgs) {
        if !e.to_string().contains("DUPLICATE_SIGNATURE_ALGORITHM") {
            return Err(TlsError::SslConfig(e.to_string()));
        }
        let mut deduped = sigalgs.clone();
        deduped.dedup();
        builder.set_sigalgs(&deduped)?;
    }

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

    // Session tickets: CFNetwork sends no session_ticket extension on fresh
    // connections; BoringSSL advertises it by default.
    if !tls.session_tickets {
        builder.set_options(SslOptions::NO_TICKET);
    }

    // Minimum TLS version. A profile-declared `[tls] min_tls_version` may
    // LOWER the TCP floor (CFNetwork iOS advertises TLS 1.0/1.1 in
    // supported_versions) but never the QUIC floor (stays 1.3 per RFC 9001).
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

    // Trust-root wiring lives in `crate::tls::trust` — env overrides
    // (`SSL_CERT_FILE` / `SSL_CERT_DIR`), the Windows system-store
    // bridge, and the Unix `set_default_verify_paths` fallback are all
    // concentrated there.
    crate::tls::trust::wire_configured_trust(builder, trust)?;
    builder.set_verify(SslVerifyMode::PEER);

    // Opt-in secret logging (SSLKEYLOGFILE), curl/browser compatible. Last
    // so a keylog install failure can never interfere with trust wiring.
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
/// Resolve a profile-declared `[tls] min_tls_version` string. Unknown values
/// are profile typos — return `None` (transport floor applies) and let the
/// validation tests flag the typo.
fn profile_min_version(declared: &Option<String>) -> Option<TlsMinVersion> {
    match declared.as_deref() {
        None => None,
        Some("1.0") => Some(TlsMinVersion::Tls10),
        Some("1.2") => Some(TlsMinVersion::Tls12),
        Some("1.3") => Some(TlsMinVersion::Tls13),
        Some(other) => {
            // Unknown values are profile typos; return None so the transport
            // floor applies (validation tests flag the typo).
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

/// Upper bound on a decompressed peer certificate chain. RFC 8879 §4
/// requires implementations to cap decompressed size; without this the
/// three decoders below grow the heap without limit while a hostile
/// server feeds a decompression bomb. Generous for post-quantum chains
/// (ML-DSA leaves are ~4–13 KB each) — a legitimate chain stays far
/// below this; a bomb stops here even if BoringSSL's own cap were loose.
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
