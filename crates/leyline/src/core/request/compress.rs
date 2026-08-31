//! Request-body compression.

use crate::core::error::{Error, Result};

/// Content codec for an outgoing request body, selected via [`crate::RequestBuilder::compress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContentEncoding {
    /// gzip (RFC 1952).
    Gzip,
    /// Brotli (RFC 7932).
    Brotli,
    /// Zstandard (RFC 8878).
    Zstd,
    /// zlib-wrapped DEFLATE (RFC 1950), per RFC 9110 §8.4.1.2.
    Deflate,
}

impl ContentEncoding {
    /// The `Content-Encoding` header token for this codec.
    pub(crate) fn header_value(self) -> &'static str {
        match self {
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Brotli => "br",
            ContentEncoding::Zstd => "zstd",
            ContentEncoding::Deflate => "deflate",
        }
    }

    /// Compress `data` with this codec.
    pub(crate) fn encode(self, data: &[u8]) -> Result<Vec<u8>> {
        match self {
            ContentEncoding::Gzip => {
                #[cfg(feature = "compression-gzip")]
                {
                    use std::io::Write;
                    let mut enc = flate2::write::GzEncoder::new(
                        Vec::with_capacity(data.len() / 2),
                        flate2::Compression::default(),
                    );
                    enc.write_all(data).map_err(Error::Io)?;
                    enc.finish().map_err(Error::Io)
                }
                #[cfg(not(feature = "compression-gzip"))]
                {
                    Err(feature_off("gzip", "compression-gzip"))
                }
            }
            ContentEncoding::Brotli => {
                #[cfg(feature = "compression-brotli")]
                {
                    use std::io::Read;
                    let mut out = Vec::with_capacity(data.len() / 2);
                    brotli::CompressorReader::new(data, 4096, 5, 22)
                        .read_to_end(&mut out)
                        .map_err(Error::Io)?;
                    Ok(out)
                }
                #[cfg(not(feature = "compression-brotli"))]
                {
                    Err(feature_off("brotli", "compression-brotli"))
                }
            }
            ContentEncoding::Zstd => {
                #[cfg(feature = "compression-zstd")]
                {
                    zstd::encode_all(data, 3).map_err(Error::Io)
                }
                #[cfg(not(feature = "compression-zstd"))]
                {
                    Err(feature_off("zstd", "compression-zstd"))
                }
            }
            ContentEncoding::Deflate => {
                #[cfg(feature = "compression-deflate")]
                {
                    use std::io::Write;
                    let mut enc = flate2::write::ZlibEncoder::new(
                        Vec::with_capacity(data.len() / 2),
                        flate2::Compression::default(),
                    );
                    enc.write_all(data).map_err(Error::Io)?;
                    enc.finish().map_err(Error::Io)
                }
                #[cfg(not(feature = "compression-deflate"))]
                {
                    Err(feature_off("deflate", "compression-deflate"))
                }
            }
        }
    }
}

/// Error for a codec whose cargo feature was compiled out.
#[cfg(not(all(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-zstd",
    feature = "compression-deflate"
)))]
fn feature_off(codec: &str, feature: &str) -> Error {
    Error::Body(format!(
        "{codec} request compression requested but the `{feature}` feature is not compiled in"
    ))
}

#[cfg(test)]
mod tests;
