#[cfg(leyline_any_codec)]
use std::io::Write;

use super::limit::Cap;
#[cfg(leyline_any_codec)]
use super::sink::Sink;
#[cfg(leyline_any_codec)]
use crate::error::Error;
use crate::error::{Kind, Result};
use crate::profile::Encoding;

#[cfg(feature = "compression-zstd")]
const ZSTD_WINDOW_LOG_MAX: u32 = 23;

pub(super) enum Stage {
    #[cfg(feature = "compression-gzip")]
    Gzip(Box<flate2::write::MultiGzDecoder<Sink>>),
    #[cfg(feature = "compression-deflate")]
    DeflatePending { cap: Cap, head: Vec<u8> },
    #[cfg(feature = "compression-deflate")]
    Zlib(Box<flate2::write::ZlibDecoder<Sink>>),
    #[cfg(feature = "compression-deflate")]
    RawDeflate(Box<flate2::write::DeflateDecoder<Sink>>),
    #[cfg(feature = "compression-brotli")]
    Brotli(Box<brotli::DecompressorWriter<Sink>>),
    #[cfg(feature = "compression-zstd")]
    Zstd(Box<zstd::stream::zio::Writer<Sink, zstd::stream::raw::Decoder<'static>>>),
}

#[cfg(not(leyline_all_codecs))]
const BODY_RECEIVED: &str = "body received";

#[cfg(leyline_any_codec)]
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

#[cfg(leyline_any_codec)]
fn pump(writer: &mut dyn Write, input: &[u8], name: &str) -> Result<()> {
    writer.write_all(input).map_err(|e| decode_error(name, e))?;
    writer.flush().map_err(|e| decode_error(name, e))
}

#[cfg(leyline_any_codec)]
fn settle(pumped: Result<()>, sink: &mut Sink) -> Result<Vec<u8>> {
    let out = std::mem::take(&mut sink.buf);
    if sink.truncated() {
        return Ok(out);
    }
    pumped.map(|()| out)
}

#[cfg(feature = "compression-deflate")]
fn is_zlib_header(cmf: u8, flg: u8) -> bool {
    cmf & 0x0f == 8 && cmf >> 4 <= 7 && (u16::from(cmf) << 8 | u16::from(flg)) % 31 == 0
}

impl Stage {
    #[cfg(feature = "compression-deflate")]
    fn finish_pending(&mut self, cap: Cap, head: &[u8]) -> Result<Vec<u8>> {
        if head.is_empty() {
            return Ok(Vec::new());
        }
        *self = Self::raw_deflate(cap);
        let mut out = self.write(head)?;
        out.extend(self.finish()?);
        Ok(out)
    }

    #[cfg(feature = "compression-deflate")]
    fn deflate_for(cmf: u8, flg: u8, cap: Cap) -> Self {
        if is_zlib_header(cmf, flg) {
            Self::Zlib(Box::new(flate2::write::ZlibDecoder::new(Sink::new(cap))))
        } else {
            Self::raw_deflate(cap)
        }
    }

    #[cfg(feature = "compression-deflate")]
    fn raw_deflate(cap: Cap) -> Self {
        Self::RawDeflate(Box::new(flate2::write::DeflateDecoder::new(Sink::new(cap))))
    }

    #[cfg_attr(
        not(leyline_any_codec),
        expect(
            unused_variables,
            reason = "the cap is read only by an enabled decoder"
        )
    )]
    pub(super) fn new(coding: Encoding, cap: Cap) -> Result<Self> {
        match coding {
            Encoding::Gzip => {
                #[cfg(feature = "compression-gzip")]
                {
                    Ok(Self::Gzip(Box::new(flate2::write::MultiGzDecoder::new(
                        Sink::new(cap),
                    ))))
                }
                #[cfg(not(feature = "compression-gzip"))]
                {
                    Err(coding.feature_off(Kind::Decode, BODY_RECEIVED))
                }
            }
            Encoding::Deflate => {
                #[cfg(feature = "compression-deflate")]
                {
                    Ok(Self::DeflatePending {
                        cap,
                        head: Vec::new(),
                    })
                }
                #[cfg(not(feature = "compression-deflate"))]
                {
                    Err(coding.feature_off(Kind::Decode, BODY_RECEIVED))
                }
            }
            Encoding::Brotli => {
                #[cfg(feature = "compression-brotli")]
                {
                    Ok(Self::Brotli(Box::new(brotli::DecompressorWriter::new(
                        Sink::new(cap),
                        4096,
                    ))))
                }
                #[cfg(not(feature = "compression-brotli"))]
                {
                    Err(coding.feature_off(Kind::Decode, BODY_RECEIVED))
                }
            }
            Encoding::Zstd => {
                #[cfg(feature = "compression-zstd")]
                {
                    zstd::stream::raw::Decoder::new()
                        .and_then(|mut d| {
                            d.set_parameter(zstd::stream::raw::DParameter::WindowLogMax(
                                ZSTD_WINDOW_LOG_MAX,
                            ))?;
                            Ok(d)
                        })
                        .map(|d| {
                            Self::Zstd(Box::new(zstd::stream::zio::Writer::new(Sink::new(cap), d)))
                        })
                        .map_err(|e| {
                            Error::new(Kind::Decode).with_message(format!("{}: {e}", coding.name()))
                        })
                }
                #[cfg(not(feature = "compression-zstd"))]
                {
                    Err(coding.feature_off(Kind::Decode, BODY_RECEIVED))
                }
            }
        }
    }

    pub(super) fn write(&mut self, input: &[u8]) -> Result<Vec<u8>> {
        match self {
            #[cfg(not(leyline_any_codec))]
            _ => {
                let _ = input;
                match *self {}
            }
            #[cfg(feature = "compression-gzip")]
            Self::Gzip(d) => {
                let pumped = pump(&mut **d, input, Encoding::Gzip.name());
                settle(pumped, d.get_mut())
            }
            #[cfg(feature = "compression-deflate")]
            Self::DeflatePending { cap, head } => {
                let cap = *cap;
                if head.is_empty() && input.len() >= 2 {
                    *self = Self::deflate_for(input[0], input[1], cap);
                    return self.write(input);
                }
                head.extend_from_slice(input);
                let [cmf, flg, ..] = head[..] else {
                    return Ok(Vec::new());
                };
                let head = std::mem::take(head);
                *self = Self::deflate_for(cmf, flg, cap);
                self.write(&head)
            }
            #[cfg(feature = "compression-deflate")]
            Self::Zlib(d) => {
                let pumped = pump(&mut **d, input, Encoding::Deflate.name());
                settle(pumped, d.get_mut())
            }
            #[cfg(feature = "compression-deflate")]
            Self::RawDeflate(d) => {
                let pumped = pump(&mut **d, input, Encoding::Deflate.name());
                settle(pumped, d.get_mut())
            }
            #[cfg(feature = "compression-brotli")]
            Self::Brotli(d) => {
                let pumped = pump(&mut **d, input, Encoding::Brotli.name());
                settle(pumped, d.get_mut())
            }
            #[cfg(feature = "compression-zstd")]
            Self::Zstd(d) => {
                let pumped = pump(&mut **d, input, Encoding::Zstd.name());
                settle(pumped, d.writer_mut())
            }
        }
    }

    pub(super) fn finish(&mut self) -> Result<Vec<u8>> {
        match self {
            #[cfg(not(leyline_any_codec))]
            _ => match *self {},
            #[cfg(feature = "compression-gzip")]
            Self::Gzip(d) => {
                d.try_finish()
                    .map_err(|e| decode_error(Encoding::Gzip.name(), e))?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-deflate")]
            Self::DeflatePending { cap, head } => {
                let (cap, head) = (*cap, std::mem::take(head));
                self.finish_pending(cap, &head)
            }
            #[cfg(feature = "compression-deflate")]
            Self::Zlib(d) => {
                d.try_finish()
                    .map_err(|e| decode_error(Encoding::Deflate.name(), e))?;
                require_end(&mut **d, Encoding::Deflate.name())?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-deflate")]
            Self::RawDeflate(d) => {
                d.try_finish()
                    .map_err(|e| decode_error(Encoding::Deflate.name(), e))?;
                require_end(&mut **d, Encoding::Deflate.name())?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-brotli")]
            Self::Brotli(d) => {
                d.close()
                    .map_err(|e| decode_error(Encoding::Brotli.name(), e))?;
                Ok(std::mem::take(&mut d.get_mut().buf))
            }
            #[cfg(feature = "compression-zstd")]
            Self::Zstd(d) => {
                d.finish()
                    .map_err(|e| decode_error(Encoding::Zstd.name(), e))?;
                Ok(std::mem::take(&mut d.writer_mut().buf))
            }
        }
    }
}
