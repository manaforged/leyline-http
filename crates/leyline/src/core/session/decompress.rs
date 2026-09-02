#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
use std::io::Read;

use crate::core::CompressionConfig;
use crate::core::error::{Error, Kind, Result};

pub(super) fn decompress_body(
    body: Vec<u8>,
    encoding: Option<&str>,
    config: &CompressionConfig,
) -> Result<(Vec<u8>, bool)> {
    let encoding = match encoding {
        Some(e) => e,
        None => return Ok((body, false)),
    };

    let encodings: Vec<&str> = encoding.split(',').map(|s| s.trim()).collect();
    if !encodings.iter().all(|enc| config.allows(enc)) {
        return Ok((body, false));
    }
    let mut data = body;

    for enc in encodings.iter().rev() {
        data = decompress_single(data, enc)?;
    }

    Ok((data, true))
}

/// Response headers as `(name, value)` pairs in wire order.
type HeaderPairs = Vec<(http::HeaderName, http::HeaderValue)>;

/// Decompress `body` per its `content-encoding` header and, when bytes were actually decoded, drop the now-stale `content-encoding`/`content-length` headers.
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

/// Max decompressed body size (100 MB, same as wire limit).
const MAX_DECOMPRESSED: usize = 100 * 1024 * 1024;

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

fn decompress_single(body: Vec<u8>, encoding: &str) -> Result<Vec<u8>> {
    match encoding {
        "gzip" | "x-gzip" => {
            #[cfg(feature = "compression-gzip")]
            {
                let mut decoder = flate2::read::GzDecoder::new(&body[..]);
                read_limited(&mut decoder, "gzip")
            }
            #[cfg(not(feature = "compression-gzip"))]
            {
                Err(Error::new(Kind::Decode).with_message(
                    "gzip body received but the compression-gzip feature is not compiled in",
                ))
            }
        }
        "br" => {
            #[cfg(feature = "compression-brotli")]
            {
                let mut decoder = brotli::Decompressor::new(&body[..], 4096);
                read_limited(&mut decoder, "brotli")
            }
            #[cfg(not(feature = "compression-brotli"))]
            {
                Err(Error::new(Kind::Decode).with_message(
                    "brotli body received but the compression-brotli feature is not compiled in",
                ))
            }
        }
        "zstd" => {
            #[cfg(feature = "compression-zstd")]
            {
                let mut decoder = zstd::Decoder::new(&body[..])
                    .map_err(|e| Error::new(Kind::Decode).with_message(format!("zstd: {e}")))?;
                read_limited(&mut decoder, "zstd")
            }
            #[cfg(not(feature = "compression-zstd"))]
            {
                Err(Error::new(Kind::Decode).with_message(
                    "zstd body received but the compression-zstd feature is not compiled in",
                ))
            }
        }
        "deflate" => {
            #[cfg(feature = "compression-deflate")]
            {
                let looks_like_zlib = body.first() == Some(&0x78);
                if looks_like_zlib {
                    let mut decoder = flate2::read::ZlibDecoder::new(&body[..]);
                    if let Ok(v) = read_limited(&mut decoder, "deflate") {
                        return Ok(v);
                    }
                }
                let mut decoder = flate2::read::DeflateDecoder::new(&body[..]);
                read_limited(&mut decoder, "deflate")
            }
            #[cfg(not(feature = "compression-deflate"))]
            {
                Err(Error::new(Kind::Decode).with_message(
                    "deflate body received but the compression-deflate feature is not compiled in",
                ))
            }
        }
        "identity" | "" => Ok(body),
        _ => Ok(body),
    }
}

/// Read from a decoder with a size limit (decompression bomb protection).
#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
fn read_limited(reader: &mut impl Read, name: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| Error::new(Kind::Decode).with_message(format!("{name}: {e}")))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() > MAX_DECOMPRESSED {
            return Err(Error::new(Kind::Decode).with_message(format!(
                "{name}: decompressed size exceeds {MAX_DECOMPRESSED} bytes"
            )));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
