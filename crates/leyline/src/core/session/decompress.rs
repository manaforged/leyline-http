#[cfg(any(
    feature = "compression-gzip",
    feature = "compression-brotli",
    feature = "compression-deflate",
    feature = "compression-zstd"
))]
use std::io::Read;

use crate::core::error::{Error, Result};
use crate::core::{CompressionConfig, HeaderStr};

pub(super) fn decompress_body(
    body: Vec<u8>,
    encoding: Option<&str>,
    config: &CompressionConfig,
) -> Result<(Vec<u8>, bool)> {
    let encoding = match encoding {
        Some(e) => e,
        None => return Ok((body, false)),
    };

    // Split on comma for multi-encoding, apply in reverse order.
    // "gzip, br" means gzip was applied first and br second; decode br then gzip.
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
type HeaderPairs = Vec<(HeaderStr, HeaderStr)>;

/// Decompress `body` per its `content-encoding` header and, when bytes were
/// actually decoded, drop the now-stale `content-encoding`/`content-length`
/// headers. Headers are returned unchanged when nothing was decoded.
pub(super) fn decompress_and_strip(
    body: Vec<u8>,
    headers: HeaderPairs,
    config: &CompressionConfig,
) -> Result<(Vec<u8>, HeaderPairs)> {
    let content_encoding = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-encoding"))
        .map(|(_, v)| v.to_lowercase());
    let (body, decoded) = decompress_body(body, content_encoding.as_deref(), config)?;
    let headers = if decoded {
        headers
            .into_iter()
            .filter(|(k, _)| {
                !k.eq_ignore_ascii_case("content-encoding")
                    && !k.eq_ignore_ascii_case("content-length")
            })
            .collect()
    } else {
        headers
    };
    Ok((body, headers))
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
            return Err(Error::Body(format!(
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
            #[cfg(feature = "compression-gzip")]
            {
                let mut decoder = flate2::read::GzDecoder::new(&body[..]);
                read_limited(&mut decoder, "gzip")
            }
            #[cfg(not(feature = "compression-gzip"))]
            {
                Err(Error::Decode(
                    "gzip body received but the compression-gzip feature is not compiled in".into(),
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
                Err(Error::Decode(
                    "brotli body received but the compression-brotli feature is not compiled in"
                        .into(),
                ))
            }
        }
        "zstd" => {
            #[cfg(feature = "compression-zstd")]
            {
                let mut decoder = zstd::Decoder::new(&body[..])
                    .map_err(|e| Error::Decode(format!("zstd: {e}")))?;
                read_limited(&mut decoder, "zstd")
            }
            #[cfg(not(feature = "compression-zstd"))]
            {
                Err(Error::Decode(
                    "zstd body received but the compression-zstd feature is not compiled in".into(),
                ))
            }
        }
        "deflate" => {
            #[cfg(feature = "compression-deflate")]
            {
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
            #[cfg(not(feature = "compression-deflate"))]
            {
                Err(Error::Decode(
                    "deflate body received but the compression-deflate feature is not compiled in"
                        .into(),
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
            .map_err(|e| Error::Decode(format!("{name}: {e}")))?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() > MAX_DECOMPRESSED {
            return Err(Error::Decode(format!(
                "{name}: decompressed size exceeds {MAX_DECOMPRESSED} bytes"
            )));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(feature = "compression-gzip", feature = "compression-brotli"))]
    use std::io::Write;

    #[cfg(all(feature = "compression-gzip", feature = "compression-brotli"))]
    #[test]
    fn decompress_multi_encoding_in_reverse_order() {
        let body = b"browser-shaped bytes";

        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(body).unwrap();
        let gzip_body = gzip.finish().unwrap();

        let mut br = brotli::CompressorReader::new(&gzip_body[..], 4096, 5, 22);
        let mut encoded = Vec::new();
        br.read_to_end(&mut encoded).unwrap();

        let (decoded, decoded_flag) =
            decompress_body(encoded, Some("gzip, br"), &CompressionConfig::default()).unwrap();
        assert!(decoded_flag);
        assert_eq!(decoded, body);
    }

    // A codec whose cargo feature is compiled out must hard-error rather
    // than pass the body through: the caller would otherwise receive
    // compressed bytes flagged `decoded` with content-encoding and
    // content-length already stripped — silent corruption.
    #[cfg(not(feature = "compression-gzip"))]
    #[test]
    fn gzip_without_feature_is_an_error() {
        let err = decompress_body(
            vec![0x1f, 0x8b],
            Some("gzip"),
            &CompressionConfig::default(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Decode(_)), "got: {err:?}");
    }

    #[cfg(not(feature = "compression-brotli"))]
    #[test]
    fn brotli_without_feature_is_an_error() {
        let err =
            decompress_body(vec![0x0b], Some("br"), &CompressionConfig::default()).unwrap_err();
        assert!(matches!(err, Error::Decode(_)), "got: {err:?}");
    }

    #[cfg(not(feature = "compression-deflate"))]
    #[test]
    fn deflate_without_feature_is_an_error() {
        let err = decompress_body(
            vec![0x78, 0x9c],
            Some("deflate"),
            &CompressionConfig::default(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Decode(_)), "got: {err:?}");
    }

    #[cfg(not(feature = "compression-zstd"))]
    #[test]
    fn zstd_without_feature_is_an_error() {
        let err = decompress_body(
            vec![0x28, 0xb5],
            Some("zstd"),
            &CompressionConfig::default(),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Decode(_)), "got: {err:?}");
    }

    // When bytes are actually decoded, the now-stale content-encoding and
    // content-length must be stripped while every other header survives.
    #[cfg(feature = "compression-gzip")]
    #[test]
    fn decompress_and_strip_drops_stale_framing_headers() {
        let body = b"browser-shaped bytes";
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(body).unwrap();
        let gzip_body = gzip.finish().unwrap();

        let headers = vec![
            (
                HeaderStr::from_static("Content-Encoding"),
                HeaderStr::from_static("gzip"),
            ),
            (
                HeaderStr::from_static("Content-Length"),
                HeaderStr::from(gzip_body.len().to_string()),
            ),
            (
                HeaderStr::from_static("Content-Type"),
                HeaderStr::from_static("text/plain"),
            ),
        ];
        let (decoded, headers) =
            decompress_and_strip(gzip_body, headers, &CompressionConfig::default()).unwrap();

        assert_eq!(decoded, body);
        assert!(!headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-encoding")));
        assert!(!headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length")));
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "text/plain"));
    }

    // No content-encoding means nothing was decoded: every header, including
    // any content-length, must pass through untouched.
    #[test]
    fn decompress_and_strip_preserves_headers_when_not_decoded() {
        let headers = vec![
            (
                HeaderStr::from_static("Content-Length"),
                HeaderStr::from_static("5"),
            ),
            (
                HeaderStr::from_static("Content-Type"),
                HeaderStr::from_static("text/plain"),
            ),
        ];
        let (body, out) = decompress_and_strip(
            b"plain".to_vec(),
            headers.clone(),
            &CompressionConfig::default(),
        )
        .unwrap();
        assert_eq!(body, b"plain");
        assert_eq!(out, headers);
    }
}
