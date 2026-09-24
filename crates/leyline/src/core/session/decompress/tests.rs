use super::*;
use http::{HeaderName, HeaderValue};
#[cfg(all(feature = "compression-gzip", feature = "compression-brotli"))]
use std::io::{Read, Write};

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

#[cfg(not(feature = "compression-gzip"))]
#[test]
fn gzip_without_feature_is_an_error() {
    let err = decompress_body(
        vec![0x1f, 0x8b],
        Some("gzip"),
        &CompressionConfig::default(),
    )
    .expect_err("disabled response codec must fail");
    assert!(err.is_decode(), "got: {err:?}");
}

#[cfg(not(feature = "compression-brotli"))]
#[test]
fn brotli_without_feature_is_an_error() {
    let err = decompress_body(vec![0x0b], Some("br"), &CompressionConfig::default())
        .expect_err("disabled response codec must fail");
    assert!(err.is_decode(), "got: {err:?}");
}

#[cfg(not(feature = "compression-deflate"))]
#[test]
fn deflate_without_feature_is_an_error() {
    let err = decompress_body(
        vec![0x78, 0x9c],
        Some("deflate"),
        &CompressionConfig::default(),
    )
    .expect_err("disabled response codec must fail");
    assert!(err.is_decode(), "got: {err:?}");
}

#[cfg(not(feature = "compression-zstd"))]
#[test]
fn zstd_without_feature_is_an_error() {
    let err = decompress_body(
        vec![0x28, 0xb5],
        Some("zstd"),
        &CompressionConfig::default(),
    )
    .expect_err("disabled response codec must fail");
    assert!(err.is_decode(), "got: {err:?}");
}

#[cfg(feature = "compression-gzip")]
#[test]
fn decompress_and_strip_drops_stale_framing_headers() {
    let body = b"browser-shaped bytes";
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(body).unwrap();
    let gzip_body = gzip.finish().unwrap();

    let headers = vec![
        (
            HeaderName::from_static("content-encoding"),
            HeaderValue::from_static("gzip"),
        ),
        (
            HeaderName::from_static("content-length"),
            HeaderValue::from(gzip_body.len()),
        ),
        (
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("text/plain"),
        ),
    ];
    let (decoded, headers) =
        decompress_and_strip(gzip_body, headers, &CompressionConfig::default()).unwrap();

    assert_eq!(decoded, body);
    assert!(!headers.iter().any(|(k, _)| *k == "content-encoding"));
    assert!(!headers.iter().any(|(k, _)| *k == "content-length"));
    assert!(
        headers
            .iter()
            .any(|(k, v)| *k == "content-type" && v == "text/plain")
    );
}

#[test]
fn decompress_and_strip_preserves_headers_when_not_decoded() {
    let headers = vec![
        (
            HeaderName::from_static("content-length"),
            HeaderValue::from_static("5"),
        ),
        (
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("text/plain"),
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
