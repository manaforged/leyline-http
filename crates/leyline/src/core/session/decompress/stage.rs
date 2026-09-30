#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
use std::io::Write;

#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
use super::sink::Sink;
use crate::core::error::{Error, Kind, Result};

pub(super) enum Stage {
    Identity,
    #[cfg(feature = "compression-gzip")]
    Gzip(Box<flate2::write::MultiGzDecoder<Sink>>),
    #[cfg(feature = "compression-deflate")]
    DeflatePending {
        limit: usize,
        head: Vec<u8>,
    },
    #[cfg(feature = "compression-deflate")]
    Zlib(Box<flate2::write::ZlibDecoder<Sink>>),
    #[cfg(feature = "compression-deflate")]
    RawDeflate(Box<flate2::write::DeflateDecoder<Sink>>),
    #[cfg(feature = "compression-brotli")]
    Brotli(Box<brotli::DecompressorWriter<Sink>>),
    #[cfg(feature = "compression-zstd")]
    Zstd(Box<zstd::stream::zio::Writer<Sink, zstd::stream::raw::Decoder<'static>>>),
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
    match super::BodyLimit::of_io(&error) {
        Some(limit) => limit.error(),
        None => Error::new(Kind::Decode).with_message(format!("{name}: {error}")),
    }
}

#[cfg(feature = "compression-deflate")]
fn require_end(writer: &mut dyn Write, name: &str) -> Result<()> {
    match writer.write(&[0]) {
        Ok(0) => Ok(()),
        _ => Err(Error::new(Kind::Decode).with_message(format!("{name}: truncated stream"))),
    }
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

#[cfg(feature = "compression-deflate")]
fn is_zlib_header(cmf: u8, flg: u8) -> bool {
    cmf & 0x0f == 8 && cmf >> 4 <= 7 && (u16::from(cmf) << 8 | u16::from(flg)) % 31 == 0
}

impl Stage {
    #[cfg(feature = "compression-deflate")]
    fn finish_pending(&mut self, limit: usize, head: &[u8]) -> Result<Vec<u8>> {
        if head.is_empty() {
            return Ok(Vec::new());
        }
        *self = Self::raw_deflate(limit);
        let mut out = self.write(head)?;
        out.extend(self.finish()?);
        Ok(out)
    }

    #[cfg(feature = "compression-deflate")]
    fn raw_deflate(limit: usize) -> Self {
        Self::RawDeflate(Box::new(flate2::write::DeflateDecoder::new(Sink::new(
            limit,
        ))))
    }

    #[cfg_attr(
        not(any(
            feature = "compression-gzip",
            feature = "compression-brotli",
            feature = "compression-deflate",
            feature = "compression-zstd"
        )),
        expect(unused_variables)
    )]
    pub(super) fn new(encoding: &str, limit: usize) -> Result<Self> {
        match encoding {
            "gzip" | "x-gzip" => {
                #[cfg(feature = "compression-gzip")]
                {
                    Ok(Self::Gzip(Box::new(flate2::write::MultiGzDecoder::new(
                        Sink::new(limit),
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
                    Ok(Self::DeflatePending {
                        limit,
                        head: Vec::new(),
                    })
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
                        Sink::new(limit),
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
                    zstd::stream::raw::Decoder::new()
                        .map(|d| {
                            Self::Zstd(Box::new(zstd::stream::zio::Writer::new(
                                Sink::new(limit),
                                d,
                            )))
                        })
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

    pub(super) fn write(&mut self, input: &[u8]) -> Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(input.to_vec()),
            #[cfg(feature = "compression-gzip")]
            Self::Gzip(d) => {
                pump(&mut **d, input, "gzip")?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-deflate")]
            Self::DeflatePending { limit, head } => {
                head.extend_from_slice(input);
                let [cmf, flg, ..] = head[..] else {
                    return Ok(Vec::new());
                };
                let limit = *limit;
                let head = std::mem::take(head);
                *self = if is_zlib_header(cmf, flg) {
                    Self::Zlib(Box::new(flate2::write::ZlibDecoder::new(Sink::new(limit))))
                } else {
                    Self::raw_deflate(limit)
                };
                self.write(&head)
            }
            #[cfg(feature = "compression-deflate")]
            Self::Zlib(d) => {
                pump(&mut **d, input, "deflate")?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-deflate")]
            Self::RawDeflate(d) => {
                pump(&mut **d, input, "deflate")?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-brotli")]
            Self::Brotli(d) => {
                pump(&mut **d, input, "brotli")?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-zstd")]
            Self::Zstd(d) => {
                pump(&mut **d, input, "zstd")?;
                Ok(std::mem::take(&mut d.writer_mut().buf))
            }
        }
    }

    pub(super) fn finish(&mut self) -> Result<Vec<u8>> {
        match self {
            Self::Identity => Ok(Vec::new()),
            #[cfg(feature = "compression-gzip")]
            Self::Gzip(d) => {
                d.try_finish().map_err(|e| decode_error("gzip", e))?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-deflate")]
            Self::DeflatePending { limit, head } => {
                let (limit, head) = (*limit, std::mem::take(head));
                self.finish_pending(limit, &head)
            }
            #[cfg(feature = "compression-deflate")]
            Self::Zlib(d) => {
                d.try_finish().map_err(|e| decode_error("deflate", e))?;
                require_end(&mut **d, "deflate")?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-deflate")]
            Self::RawDeflate(d) => {
                d.try_finish().map_err(|e| decode_error("deflate", e))?;
                require_end(&mut **d, "deflate")?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-brotli")]
            Self::Brotli(d) => {
                d.close().map_err(|e| decode_error("brotli", e))?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-zstd")]
            Self::Zstd(d) => {
                d.finish().map_err(|e| decode_error("zstd", e))?;
                Ok(std::mem::take(&mut d.writer_mut().buf))
            }
        }
    }
}
