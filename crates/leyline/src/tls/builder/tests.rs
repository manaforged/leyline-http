use super::*;
use leyline_bssl::ssl::CertificateCompressor;
use std::io::Write;

// The decompressors aren't decorative: a server that compresses its
// certificate with the codepoint we advertise must actually be
// decodable. Round-trip a known payload through each.
#[test]
fn zlib_decompressor_round_trips() {
    let original = b"-----BEGIN CERTIFICATE----- leyline zlib roundtrip";
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(original).unwrap();
    let compressed = enc.finish().unwrap();

    let mut out = Vec::new();
    ZlibDecompressor.decompress(&compressed, &mut out).unwrap();
    assert_eq!(out, original);
}

#[test]
fn zstd_decompressor_round_trips() {
    let original = b"-----BEGIN CERTIFICATE----- leyline zstd roundtrip";
    let compressed = zstd::stream::encode_all(&original[..], 3).unwrap();

    let mut out = Vec::new();
    ZstdDecompressor.decompress(&compressed, &mut out).unwrap();
    assert_eq!(out, original);
}

#[test]
fn decompression_bomb_is_capped() {
    // 8 MiB of zeros compresses to a few KB: a tiny hostile
    // certificate-compression body must not grow the heap unbounded.
    let original = vec![0u8; 8 * 1024 * 1024];
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(&original).unwrap();
    let compressed = enc.finish().unwrap();

    let mut out = Vec::new();
    let err = ZlibDecompressor
        .decompress(&compressed, &mut out)
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(out.len() <= MAX_CERT_DECOMPRESSED_BYTES + 8192);
}
