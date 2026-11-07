use super::*;

#[test]
fn header_tokens() {
    assert_eq!(Encoding::Gzip.token(), "gzip");
    assert_eq!(Encoding::Brotli.token(), "br");
    assert_eq!(Encoding::Zstd.token(), "zstd");
    assert_eq!(Encoding::Deflate.token(), "deflate");
}

#[cfg(leyline_any_codec)]
fn sample() -> Vec<u8> {
    b"the quick brown fox jumps over the lazy dog. ".repeat(16)
}

#[cfg(feature = "compression-gzip")]
#[test]
fn gzip_roundtrips_and_shrinks() {
    use std::io::Read;
    let data = sample();
    let enc = encode(Encoding::Gzip, &data).unwrap();
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
    let enc = encode(Encoding::Brotli, &data).unwrap();
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
    let enc = encode(Encoding::Zstd, &data).unwrap();
    assert!(enc.len() < data.len());
    let out = zstd::decode_all(&enc[..]).unwrap();
    assert_eq!(out, data);
}

#[cfg(feature = "compression-deflate")]
#[test]
fn deflate_roundtrips_zlib_wrapped() {
    use std::io::Read;
    let data = sample();
    let enc = encode(Encoding::Deflate, &data).unwrap();
    assert!(enc.len() < data.len());
    assert_eq!(enc.first(), Some(&0x78));
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(&enc[..])
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, data);
}

#[cfg(not(leyline_all_codecs))]
#[test]
fn disabled_codecs_return_body_errors() {
    for (encoding, enabled, feature) in [
        (
            Encoding::Gzip,
            cfg!(feature = "compression-gzip"),
            "compression-gzip",
        ),
        (
            Encoding::Brotli,
            cfg!(feature = "compression-brotli"),
            "compression-brotli",
        ),
        (
            Encoding::Zstd,
            cfg!(feature = "compression-zstd"),
            "compression-zstd",
        ),
        (
            Encoding::Deflate,
            cfg!(feature = "compression-deflate"),
            "compression-deflate",
        ),
    ] {
        if !enabled {
            let error = encode(encoding, b"request body").expect_err("disabled codec must fail");
            assert_eq!(error.kind(), crate::error::Kind::Body);
            assert!(error.message().expect("feature guidance").contains(feature));
        }
    }
}
