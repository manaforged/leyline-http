#[cfg(leyline_any_codec)]
use crate::error::Error;
#[cfg(not(leyline_all_codecs))]
use crate::error::Kind;
use crate::error::Result;
use crate::profile::Encoding;

pub(crate) fn encode(encoding: Encoding, data: &[u8]) -> Result<Vec<u8>> {
    match encoding {
        Encoding::Gzip => gzip(data),
        Encoding::Brotli => brotli(data),
        Encoding::Zstd => zstd(data),
        Encoding::Deflate => deflate(data),
    }
}

#[cfg(feature = "compression-gzip")]
fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(
        Vec::with_capacity(data.len() / 2),
        flate2::Compression::default(),
    );
    enc.write_all(data).map_err(Error::from)?;
    enc.finish().map_err(Error::from)
}

#[cfg(not(feature = "compression-gzip"))]
fn gzip(_data: &[u8]) -> Result<Vec<u8>> {
    Err(Encoding::Gzip.feature_off(Kind::Body, COMPRESSION_REQUESTED))
}

#[cfg(feature = "compression-brotli")]
fn brotli(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::with_capacity(data.len() / 2);
    brotli::CompressorReader::new(data, 4096, 5, 22)
        .read_to_end(&mut out)
        .map_err(Error::from)?;
    Ok(out)
}

#[cfg(not(feature = "compression-brotli"))]
fn brotli(_data: &[u8]) -> Result<Vec<u8>> {
    Err(Encoding::Brotli.feature_off(Kind::Body, COMPRESSION_REQUESTED))
}

#[cfg(feature = "compression-zstd")]
fn zstd(data: &[u8]) -> Result<Vec<u8>> {
    zstd::encode_all(data, 3).map_err(Error::from)
}

#[cfg(not(feature = "compression-zstd"))]
fn zstd(_data: &[u8]) -> Result<Vec<u8>> {
    Err(Encoding::Zstd.feature_off(Kind::Body, COMPRESSION_REQUESTED))
}

#[cfg(feature = "compression-deflate")]
fn deflate(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Write;
    let mut enc = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(data.len() / 2),
        flate2::Compression::default(),
    );
    enc.write_all(data).map_err(Error::from)?;
    enc.finish().map_err(Error::from)
}

#[cfg(not(feature = "compression-deflate"))]
fn deflate(_data: &[u8]) -> Result<Vec<u8>> {
    Err(Encoding::Deflate.feature_off(Kind::Body, COMPRESSION_REQUESTED))
}

#[cfg(not(leyline_all_codecs))]
const COMPRESSION_REQUESTED: &str = "request compression requested";

#[cfg(test)]
mod tests;
