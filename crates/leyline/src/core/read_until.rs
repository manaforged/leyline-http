use std::io::Write;

use futures_util::StreamExt;

use crate::core::Response;
use crate::core::error::{Error, Kind, Result};

enum Decoder {
    Identity,
    #[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
    Gzip(Box<flate2::write::MultiGzDecoder<Vec<u8>>>),
    #[cfg(feature = "compression-deflate")]
    Deflate(Box<flate2::write::ZlibDecoder<Vec<u8>>>),
    #[cfg(feature = "compression-brotli")]
    Brotli(Box<brotli::DecompressorWriter<Vec<u8>>>),
    #[cfg(feature = "compression-zstd")]
    Zstd(Box<zstd::stream::write::Decoder<'static, Vec<u8>>>),
}

impl Decoder {
    fn new(encoding: &str) -> Result<Self> {
        match encoding {
            "" | "identity" => Ok(Self::Identity),
            #[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
            "gzip" | "x-gzip" => Ok(Self::Gzip(Box::new(flate2::write::MultiGzDecoder::new(
                Vec::new(),
            )))),
            #[cfg(feature = "compression-deflate")]
            "deflate" => Ok(Self::Deflate(Box::new(flate2::write::ZlibDecoder::new(
                Vec::new(),
            )))),
            #[cfg(feature = "compression-brotli")]
            "br" => Ok(Self::Brotli(Box::new(brotli::DecompressorWriter::new(
                Vec::new(),
                4096,
            )))),
            #[cfg(feature = "compression-zstd")]
            "zstd" => zstd::stream::write::Decoder::new(Vec::new())
                .map(|d| Self::Zstd(Box::new(d)))
                .map_err(|e| Error::new(Kind::Decode).with_message(format!("zstd: {e}"))),
            other => Err(Error::new(Kind::Decode).with_message(format!(
                "read_until: unsupported content-encoding {other:?}"
            ))),
        }
    }

    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<()> {
        let fail = |e: std::io::Error| Error::new(Kind::Decode).with_message(e.to_string());
        match self {
            Self::Identity => out.extend_from_slice(chunk),
            #[cfg(any(feature = "compression-gzip", feature = "compression-deflate"))]
            Self::Gzip(d) => {
                d.write_all(chunk).map_err(fail)?;
                out.append(d.get_mut());
            }
            #[cfg(feature = "compression-deflate")]
            Self::Deflate(d) => {
                d.write_all(chunk).map_err(fail)?;
                out.append(d.get_mut());
            }
            #[cfg(feature = "compression-brotli")]
            Self::Brotli(d) => {
                d.write_all(chunk).map_err(fail)?;
                out.append(d.get_mut());
            }
            #[cfg(feature = "compression-zstd")]
            Self::Zstd(d) => {
                d.write_all(chunk).map_err(fail)?;
                d.flush().map_err(fail)?;
                out.append(d.get_mut());
            }
        }
        Ok(())
    }
}

impl Response {
    pub async fn read_until<F>(self, limit: usize, mut done: F) -> Result<Vec<u8>>
    where
        F: FnMut(&[u8], usize) -> bool,
    {
        let encoding = self
            .header("content-encoding")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let mut decoder = Decoder::new(&encoding)?;
        let mut stream = self.into_stream()?;
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(Error::from)?;
            let from = out.len();
            decoder.feed(&chunk, &mut out)?;
            if done(&out, from) || out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }
}
