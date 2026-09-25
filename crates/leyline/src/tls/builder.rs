#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
use leyline_bssl::ssl::{CertificateCompressionAlgorithm, CertificateCompressor};
use leyline_bssl::ssl::{SslContextBuilder, SslVerifyMode};

use crate::profile::{BrowserProfile, TlsProfile};

use crate::tls::error::TlsError;
use crate::tls::keylog::install_from_env;
use crate::tls::trust::{TlsTrustConfig, wire_configured_trust};

mod phase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TlsMinVersion {
    Tls10,
    Tls12,
    Tls13,
}

#[cfg(any(feature = "unstable-bssl", feature = "bench-internals"))]
pub(crate) fn build_ssl_context(
    profile: &BrowserProfile,
    min_version: TlsMinVersion,
) -> Result<SslContextBuilder, TlsError> {
    let mut builder = SslContextBuilder::new(leyline_bssl::ssl::SslMethod::tls())
        .map_err(TlsError::from_stack)?;
    apply_profile_with_trust(
        &mut builder,
        profile,
        min_version,
        &TlsTrustConfig::default(),
    )?;
    Ok(builder)
}

pub(crate) fn apply_profile_with_trust(
    builder: &mut SslContextBuilder,
    profile: &BrowserProfile,
    min_version: TlsMinVersion,
    trust: &TlsTrustConfig,
) -> Result<(), TlsError> {
    apply_tls_with_trust(builder, &profile.tls, min_version, trust)
}

pub(crate) fn apply_tls_with_trust(
    builder: &mut SslContextBuilder,
    tls: &TlsProfile,
    min_version: TlsMinVersion,
    trust: &TlsTrustConfig,
) -> Result<(), TlsError> {
    phase::ciphers(builder, tls)?;
    phase::curves(builder, tls)?;
    phase::sigalgs(builder, tls)?;
    phase::extensions(builder, tls)?;
    phase::versions(builder, tls, min_version)?;

    wire_configured_trust(builder, trust)?;
    builder.set_verify(SslVerifyMode::PEER);

    install_from_env(builder);

    Ok(())
}

fn tls13_cipher_ids(ciphers: &[String]) -> Result<Vec<u16>, TlsError> {
    ciphers
        .iter()
        .filter_map(|cipher| match crate::iana::cipher_id(cipher) {
            Some(id) if crate::iana::is_tls13_cipher(id) => Some(Ok(id)),
            _ if cipher.starts_with("TLS_AES_") || cipher.starts_with("TLS_CHACHA20_") => {
                Some(Err(TlsError::Profile(format!(
                    "unknown TLS 1.3 cipher: {cipher}"
                ))))
            }
            _ => None,
        })
        .collect()
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

#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
const MAX_CERT_DECOMPRESSED_BYTES: usize = 1024 * 1024;

#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
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

#[cfg(feature = "compression-brotli")]
#[derive(Debug)]
struct BrotliDecompressor;

#[cfg(feature = "compression-brotli")]
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

#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
#[derive(Debug)]
struct ZlibDecompressor;

#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
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

#[cfg(feature = "compression-zstd")]
#[derive(Debug)]
struct ZstdDecompressor;

#[cfg(feature = "compression-zstd")]
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
