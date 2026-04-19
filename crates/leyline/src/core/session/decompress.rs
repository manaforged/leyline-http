use std::io::Read;

use crate::core::error::{Error, Result};

pub(super) fn decompress_body(body: Vec<u8>, encoding: Option<&str>) -> Result<Vec<u8>> {
    let encoding = match encoding {
        Some(e) => e,
        None => return Ok(body),
    };

    // Split on comma for multi-encoding, apply in reverse order.
    // "gzip, br" means gzip was applied first and br second; decode br then gzip.
    let encodings: Vec<&str> = encoding.split(',').map(|s| s.trim()).collect();
    let mut data = body;

    for enc in encodings.iter().rev() {
        data = decompress_single(data, enc)?;
    }

    Ok(data)
}

/// Max decompressed body size (100 MB, same as wire limit).
const MAX_DECOMPRESSED: usize = 100 * 1024 * 1024;

pub(super) async fn drain_stream_into_vec(
    mut bs: crate::core::body_stream::BodyStream,
) -> Result<Vec<u8>> {
    use futures_util::StreamExt;
    let mut out = Vec::new();
    while let Some(chunk) = bs.next().await {
        let chunk = chunk.map_err(Error::Io)?;
        if out.len() + chunk.len() > MAX_DECOMPRESSED {
            return Err(Error::Http(format!(
                "response body exceeds {MAX_DECOMPRESSED} bytes"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

fn decompress_single(body: Vec<u8>, encoding: &str) -> Result<Vec<u8>> {
    match encoding {
        "gzip" | "x-gzip" => {
            let mut decoder = flate2::read::GzDecoder::new(&body[..]);
            read_limited(&mut decoder, "gzip")
        }
        "br" => {
            let mut decoder = brotli::Decompressor::new(&body[..], 4096);
            read_limited(&mut decoder, "brotli")
        }
        "zstd" => {
            let mut decoder =
                zstd::Decoder::new(&body[..]).map_err(|e| Error::Http(format!("zstd: {e}")))?;
            read_limited(&mut decoder, "zstd")
        }
        "deflate" => {
            // HTTP `Content-Encoding: deflate` is notoriously ambiguous: some
            // servers send raw DEFLATE, most (IIS, nginx, httpbin) send zlib-
            // wrapped DEFLATE. Real Chrome tries zlib first and falls back to
            // raw. Detect zlib by its magic byte (CMF): high nibble is the
            // compression method (8 = deflate), so 0x78 is the common CMF.
            let looks_like_zlib = body.first() == Some(&0x78);
            if looks_like_zlib {
                let mut decoder = flate2::read::ZlibDecoder::new(&body[..]);
                match read_limited(&mut decoder, "deflate") {
                    Ok(v) => return Ok(v),
                    Err(_) => {
                        // Fall through to raw DEFLATE.
                    }
                }
            }
            let mut decoder = flate2::read::DeflateDecoder::new(&body[..]);
            read_limited(&mut decoder, "deflate")
        }
        "identity" | "" => Ok(body),
        _ => Ok(body),
    }
}

/// Read from a decoder with a size limit (decompression bomb protection).
fn read_limited(reader: &mut impl Read, name: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| Error::Http(format!("{name}: {e}")))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() > MAX_DECOMPRESSED {
            return Err(Error::Http(format!(
                "{name}: decompressed size exceeds {MAX_DECOMPRESSED} bytes"
            )));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn decompress_multi_encoding_in_reverse_order() {
        let body = b"browser-shaped bytes";

        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(body).unwrap();
        let gzip_body = gzip.finish().unwrap();

        let mut br = brotli::CompressorReader::new(&gzip_body[..], 4096, 5, 22);
        let mut encoded = Vec::new();
        br.read_to_end(&mut encoded).unwrap();

        let decoded = decompress_body(encoded, Some("gzip, br")).unwrap();
        assert_eq!(decoded, body);
    }
}
