use leyline_bssl::ssl::{
    CertificateCompressionAlgorithm, CertificateCompressor, SslContextBuilder,
};

use crate::tls::error::TlsError;

pub(super) const MAX_CERT_DECOMPRESSED_BYTES: usize = 1024 * 1024;

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

pub(super) fn add_decompressor<C: CertificateCompressor>(
    builder: &mut SslContextBuilder,
    codec: C,
) -> Result<(), TlsError> {
    builder
        .add_certificate_compression_algorithm(codec)
        .map_err(TlsError::from_stack)
}

#[cfg(feature = "compression-brotli")]
pub(super) struct Brotli;

#[cfg(feature = "compression-brotli")]
impl CertificateCompressor for Brotli {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::BROTLI;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W: std::io::Write>(&self, input: &[u8], output: &mut W) -> std::io::Result<()> {
        output.write_all(&read_limited_cert(brotli::Decompressor::new(input, 4096))?)
    }
}

#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
pub(super) struct Zlib;

#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
impl CertificateCompressor for Zlib {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZLIB;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W: std::io::Write>(&self, input: &[u8], output: &mut W) -> std::io::Result<()> {
        output.write_all(&read_limited_cert(flate2::read::ZlibDecoder::new(input))?)
    }
}

#[cfg(feature = "compression-zstd")]
pub(super) struct Zstd;

#[cfg(feature = "compression-zstd")]
impl CertificateCompressor for Zstd {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZSTD;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W: std::io::Write>(&self, input: &[u8], output: &mut W) -> std::io::Result<()> {
        output.write_all(&read_limited_cert(zstd::stream::read::Decoder::new(
            input,
        )?)?)
    }
}
