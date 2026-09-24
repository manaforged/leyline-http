#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
use std::io::Write;

use crate::core::CompressionConfig;
use crate::core::error::{Error, Kind, Result};

const MAX_CODINGS: usize = 4;
const MAX_DECOMPRESSED: usize = 100 * 1024 * 1024;

type HeaderPairs = Vec<(http::HeaderName, http::HeaderValue)>;

pub(crate) struct Decoder {
    stages: Vec<Stage>,
    produced: usize,
}

enum Stage {
    Identity,
    #[cfg(feature = "compression-gzip")]
    Gzip(Box<flate2::write::MultiGzDecoder<Vec<u8>>>),
    #[cfg(feature = "compression-deflate")]
    DeflatePending,
    #[cfg(feature = "compression-deflate")]
    Zlib(Box<flate2::write::ZlibDecoder<Vec<u8>>>),
    #[cfg(feature = "compression-deflate")]
    RawDeflate(Box<flate2::write::DeflateDecoder<Vec<u8>>>),
    #[cfg(feature = "compression-brotli")]
    Brotli(Box<brotli::DecompressorWriter<Vec<u8>>>),
    #[cfg(feature = "compression-zstd")]
    Zstd(Box<zstd::stream::write::Decoder<'static, Vec<u8>>>),
}

#[cfg(not(all(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
)))]
fn missing_feature(name: &str, feature: &str) -> Error {
    Error::new(Kind::Decode).with_message(format!(
        "{name} body received but the {feature} feature is not compiled in"
    ))
}

#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
fn decode_error(name: &str, error: std::io::Error) -> Error {
    Error::new(Kind::Decode).with_message(format!("{name}: {error}"))
}

#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
fn pump(writer: &mut dyn Write, input: &[u8], name: &str) -> Result<()> {
    writer.write_all(input).map_err(|e| decode_error(name, e))?;
    writer.flush().map_err(|e| decode_error(name, e))
}

impl Stage {
    fn new(encoding: &str) -> Result<Self> {
        match encoding {
            "gzip" | "x-gzip" => {
                #[cfg(feature = "compression-gzip")]
                {
                    Ok(Self::Gzip(Box::new(flate2::write::MultiGzDecoder::new(
                        Vec::new(),
                    ))))
                }
                #[cfg(not(feature = "compression-gzip"))]
                {
                    Err(missing_feature("gzip", "compression-gzip"))
                }
            }
            "deflate" => {
                #[cfg(feature = "compression-deflate")]
                {
                    Ok(Self::DeflatePending)
                }
                #[cfg(not(feature = "compression-deflate"))]
                {
                    Err(missing_feature("deflate", "compression-deflate"))
                }
            }
            "br" => {
                #[cfg(feature = "compression-brotli")]
                {
                    Ok(Self::Brotli(Box::new(brotli::DecompressorWriter::new(
                        Vec::new(),
                        4096,
                    ))))
                }
                #[cfg(not(feature = "compression-brotli"))]
                {
                    Err(missing_feature("brotli", "compression-brotli"))
                }
            }
            "zstd" => {
                #[cfg(feature = "compression-zstd")]
                {
                    zstd::stream::write::Decoder::new(Vec::new())
                        .map(|d| Self::Zstd(Box::new(d)))
                        .map_err(|e| Error::new(Kind::Decode).with_message(format!("zstd: {e}")))
                }
                #[cfg(not(feature = "compression-zstd"))]
                {
                    Err(missing_feature("zstd", "compression-zstd"))
                }
            }
            _ => Ok(Self::Identity),
        }
    }

    fn write(&mut self, input: &[u8]) -> Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(input.to_vec()),
            #[cfg(feature = "compression-gzip")]
            Self::Gzip(d) => {
                pump(&mut **d, input, "gzip")?;
                Ok(std::mem::take(d.get_mut()))
            }
            #[cfg(feature = "compression-deflate")]
            Self::DeflatePending => {
                let Some(&first) = input.first() else {
                    return Ok(Vec::new());
                };
                *self = if first == 0x78 {
                    Self::Zlib(Box::new(flate2::write::ZlibDecoder::new(Vec::new())))
                } else {
                    Self::RawDeflate(Box::new(flate2::write::DeflateDecoder::new(Vec::new())))
                };
                self.write(input)
            }
            #[cfg(feature = "compression-deflate")]
            Self::Zlib(d) => {
                pump(&mut **d, input, "deflate")?;
                Ok(std::mem::take(d.get_mut()))
            }
            #[cfg(feature = "compression-deflate")]
            Self::RawDeflate(d) => {
                pump(&mut **d, input, "deflate")?;
                Ok(std::mem::take(d.get_mut()))
            }
            #[cfg(feature = "compression-brotli")]
            Self::Brotli(d) => {
                pump(&mut **d, input, "brotli")?;
                Ok(std::mem::take(d.get_mut()))
            }
            #[cfg(feature = "compression-zstd")]
            Self::Zstd(d) => {
                pump(&mut **d, input, "zstd")?;
                Ok(std::mem::take(d.get_mut()))
            }
        }
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(Vec::new()),
            #[cfg(feature = "compression-gzip")]
            Self::Gzip(d) => {
                d.try_finish().map_err(|e| decode_error("gzip", e))?;
                Ok(std::mem::take(d.get_mut()))
            }
            #[cfg(feature = "compression-deflate")]
            Self::DeflatePending => Ok(Vec::new()),
            #[cfg(feature = "compression-deflate")]
            Self::Zlib(d) => {
                d.try_finish().map_err(|e| decode_error("deflate", e))?;
                Ok(std::mem::take(d.get_mut()))
            }
            #[cfg(feature = "compression-deflate")]
            Self::RawDeflate(d) => {
                d.try_finish().map_err(|e| decode_error("deflate", e))?;
                Ok(std::mem::take(d.get_mut()))
            }
            #[cfg(feature = "compression-brotli")]
            Self::Brotli(d) => {
                d.flush().map_err(|e| decode_error("brotli", e))?;
                Ok(std::mem::take(d.get_mut()))
            }
            #[cfg(feature = "compression-zstd")]
            Self::Zstd(d) => {
                d.flush().map_err(|e| decode_error("zstd", e))?;
                Ok(std::mem::take(d.get_mut()))
            }
        }
    }
}

impl Decoder {
    pub(crate) fn new(encoding: Option<&str>, config: &CompressionConfig) -> Result<Option<Self>> {
        let Some(encoding) = encoding else {
            return Ok(None);
        };
        let encodings: Vec<&str> = encoding.split(',').map(str::trim).collect();
        if encodings.len() > MAX_CODINGS {
            return Err(Error::new(Kind::Decode).with_message(format!(
                "content-encoding lists {} codings; at most {MAX_CODINGS} are decoded",
                encodings.len()
            )));
        }
        if !encodings.iter().all(|enc| config.allows(enc)) {
            return Ok(None);
        }
        let stages = encodings
            .iter()
            .rev()
            .copied()
            .map(Stage::new)
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(Self {
            stages,
            produced: 0,
        }))
    }

    pub(crate) fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<()> {
        let mut data = chunk.to_vec();
        for stage in &mut self.stages {
            data = stage.write(&data)?;
        }
        self.emit(data, out)
    }

    pub(crate) fn finish(&mut self, out: &mut Vec<u8>) -> Result<()> {
        for index in 0..self.stages.len() {
            let mut data = self.stages[index].finish()?;
            for stage in &mut self.stages[index + 1..] {
                data = stage.write(&data)?;
            }
            self.emit(data, out)?;
        }
        Ok(())
    }

    fn emit(&mut self, data: Vec<u8>, out: &mut Vec<u8>) -> Result<()> {
        self.produced += data.len();
        if self.produced > MAX_DECOMPRESSED {
            return Err(Error::new(Kind::Decode).with_message(format!(
                "decompressed size exceeds {MAX_DECOMPRESSED} bytes"
            )));
        }
        out.extend_from_slice(&data);
        Ok(())
    }
}

pub(crate) fn decompress_body(
    body: Vec<u8>,
    encoding: Option<&str>,
    config: &CompressionConfig,
) -> Result<(Vec<u8>, bool)> {
    let Some(mut decoder) = Decoder::new(encoding, config)? else {
        return Ok((body, false));
    };
    let mut out = Vec::new();
    decoder.feed(&body, &mut out)?;
    decoder.finish(&mut out)?;
    Ok((out, true))
}

pub(crate) fn decompress_and_strip(
    body: Vec<u8>,
    headers: HeaderPairs,
    config: &CompressionConfig,
) -> Result<(Vec<u8>, HeaderPairs)> {
    let content_encoding = headers
        .iter()
        .find(|(k, _)| *k == "content-encoding")
        .map(|(_, v)| String::from_utf8_lossy(v.as_bytes()).to_lowercase());
    let (body, decoded) = decompress_body(body, content_encoding.as_deref(), config)?;
    let headers = if decoded {
        headers
            .into_iter()
            .filter(|(k, _)| *k != "content-encoding" && *k != "content-length")
            .collect()
    } else {
        headers
    };
    Ok((body, headers))
}

pub(crate) async fn drain_stream_into_vec(
    mut bs: crate::core::body_stream::BodyStream,
) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    while let Some(chunk) = bs.next().await {
        let chunk = chunk.map_err(Error::from)?;
        if out.len() + chunk.len() > MAX_DECOMPRESSED {
            return Err(Error::new(Kind::Body)
                .with_message(format!("response body exceeds {MAX_DECOMPRESSED} bytes")));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
