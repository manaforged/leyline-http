use std::marker::PhantomData;

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

pub(super) trait CertCodec: Send + Sync + 'static {
    const ALGORITHM: CertificateCompressionAlgorithm;

    fn decode(input: &[u8]) -> std::io::Result<Vec<u8>>;
}

pub(super) struct CertDecompressor<C>(pub(super) PhantomData<C>);

impl<C: CertCodec> CertificateCompressor for CertDecompressor<C> {
    const ALGORITHM: CertificateCompressionAlgorithm = C::ALGORITHM;
    const CAN_COMPRESS: bool = false;
    const CAN_DECOMPRESS: bool = true;

    fn decompress<W>(&self, input: &[u8], output: &mut W) -> std::io::Result<()>
    where
        W: std::io::Write,
    {
        output.write_all(&C::decode(input)?)
    }
}

pub(super) fn add_decompressor<C: CertCodec>(
    builder: &mut SslContextBuilder,
) -> Result<(), TlsError> {
    builder
        .add_certificate_compression_algorithm(CertDecompressor::<C>(PhantomData))
        .map_err(TlsError::from_stack)
}

#[cfg(feature = "compression-brotli")]
pub(super) struct Brotli;

#[cfg(feature = "compression-brotli")]
impl CertCodec for Brotli {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::BROTLI;

    fn decode(input: &[u8]) -> std::io::Result<Vec<u8>> {
        read_limited_cert(brotli::Decompressor::new(input, 4096))
    }
}

#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
pub(super) struct Zlib;

#[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
impl CertCodec for Zlib {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZLIB;

    fn decode(input: &[u8]) -> std::io::Result<Vec<u8>> {
        read_limited_cert(flate2::read::ZlibDecoder::new(input))
    }
}

#[cfg(feature = "compression-zstd")]
pub(super) struct Zstd;

#[cfg(feature = "compression-zstd")]
impl CertCodec for Zstd {
    const ALGORITHM: CertificateCompressionAlgorithm = CertificateCompressionAlgorithm::ZSTD;

    fn decode(input: &[u8]) -> std::io::Result<Vec<u8>> {
        read_limited_cert(zstd::stream::read::Decoder::new(input)?)
    }
}
