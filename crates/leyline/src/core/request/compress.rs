#[cfg(not(all(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-zstd",
    feature = "compression-deflate"
)))]
use crate::core::error::Kind;
use crate::core::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContentEncoding {
    Gzip,
    Brotli,
    Zstd,
    Deflate,
}

impl ContentEncoding {
    pub(crate) fn header_value(self) -> &'static str {
        match self {
            ContentEncoding::Gzip => "gzip",
            ContentEncoding::Brotli => "br",
            ContentEncoding::Zstd => "zstd",
            ContentEncoding::Deflate => "deflate",
        }
    }

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
                    enc.write_all(data).map_err(Error::from)?;
                    enc.finish().map_err(Error::from)
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
                        .map_err(Error::from)?;
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
                    zstd::encode_all(data, 3).map_err(Error::from)
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
                    enc.write_all(data).map_err(Error::from)?;
                    enc.finish().map_err(Error::from)
                }
                #[cfg(not(feature = "compression-deflate"))]
                {
                    Err(feature_off("deflate", "compression-deflate"))
                }
            }
        }
    }
}

#[cfg(not(all(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-zstd",
    feature = "compression-deflate"
)))]
fn feature_off(codec: &str, feature: &str) -> Error {
    Error::new(Kind::Body).with_message(format!(
        "{codec} request compression requested but the `{feature}` feature is not compiled in"
    ))
}

#[cfg(test)]
mod tests;
