use super::*;

#[test]
fn header_tokens() {
    assert_eq!(ContentEncoding::Gzip.header_value(), "gzip");
    assert_eq!(ContentEncoding::Brotli.header_value(), "br");
    assert_eq!(ContentEncoding::Zstd.header_value(), "zstd");
    assert_eq!(ContentEncoding::Deflate.header_value(), "deflate");
}

// Each roundtrip decodes with the same crate used by the response path,
// proving the encoded bytes are wire-valid for that codec (not merely
// self-consistent). Compressible input so we also assert it shrank.
fn sample() -> Vec<u8> {
    b"the quick brown fox jumps over the lazy dog. ".repeat(16)
}

#[cfg(feature = "compression-gzip")]
#[test]
fn gzip_roundtrips_and_shrinks() {
    use std::io::Read;
    let data = sample();
    let enc = ContentEncoding::Gzip.encode(&data).unwrap();
    assert!(enc.len() < data.len());
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&enc[..])
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, data);
}

#[cfg(feature = "compression-brotli")]
#[test]
fn brotli_roundtrips_and_shrinks() {
    use std::io::Read;
    let data = sample();
    let enc = ContentEncoding::Brotli.encode(&data).unwrap();
    assert!(enc.len() < data.len());
    let mut out = Vec::new();
    brotli::Decompressor::new(&enc[..], 4096)
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, data);
}

#[cfg(feature = "compression-zstd")]
#[test]
fn zstd_roundtrips_and_shrinks() {
    let data = sample();
    let enc = ContentEncoding::Zstd.encode(&data).unwrap();
    assert!(enc.len() < data.len());
    let out = zstd::decode_all(&enc[..]).unwrap();
    assert_eq!(out, data);
}

#[cfg(feature = "compression-deflate")]
#[test]
fn deflate_roundtrips_zlib_wrapped() {
    use std::io::Read;
    let data = sample();
    let enc = ContentEncoding::Deflate.encode(&data).unwrap();
    assert!(enc.len() < data.len());
    // zlib-wrapped DEFLATE: CMF first byte is 0x78 (deflate method, 32K window).
    assert_eq!(enc.first(), Some(&0x78));
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(&enc[..])
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, data);
}
