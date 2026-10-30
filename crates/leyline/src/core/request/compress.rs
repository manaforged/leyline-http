#[cfg(not(all(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-zstd",
    feature = "compression-deflate"
)))]
use crate::core::error::Cause;
use crate::core::error::{Error, Result};

/// A content encoding for compressing a request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Encoding {
    /// The gzip encoding.
    Gzip,
    /// The Brotli encoding.
    Brotli,
    /// The Zstandard encoding.
    Zstd,
    /// The deflate encoding.
    Deflate,
}

impl Encoding {
    pub(crate) fn header_value(self) -> &'static str {
        match self {
            Encoding::Gzip => "gzip",
            Encoding::Brotli => "br",
            Encoding::Zstd => "zstd",
            Encoding::Deflate => "deflate",
        }
    }

    pub(crate) fn encode(self, data: &[u8]) -> Result<Vec<u8>> {
        match self {
            Encoding::Gzip => gzip(data),
            Encoding::Brotli => brotli(data),
            Encoding::Zstd => zstd(data),
            Encoding::Deflate => deflate(data),
        }
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
    Err(feature_off("gzip", "compression-gzip"))
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
    Err(feature_off("brotli", "compression-brotli"))
}

#[cfg(feature = "compression-zstd")]
fn zstd(data: &[u8]) -> Result<Vec<u8>> {
    zstd::encode_all(data, 3).map_err(Error::from)
}

#[cfg(not(feature = "compression-zstd"))]
fn zstd(_data: &[u8]) -> Result<Vec<u8>> {
    Err(feature_off("zstd", "compression-zstd"))
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
    Err(feature_off("deflate", "compression-deflate"))
}

#[cfg(not(all(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-zstd",
    feature = "compression-deflate"
)))]
fn feature_off(codec: &str, feature: &str) -> Error {
    Error::new(Cause::Body).with_message(format!(
        "{codec} request compression requested but the `{feature}` feature is not compiled in"
    ))
}

#[cfg(test)]
mod tests;
