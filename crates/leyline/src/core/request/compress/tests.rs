#[cfg(not(leyline_all_codecs))]
use super::encode;
use crate::profile::Encoding;

#[cfg(leyline_any_codec)]
fn sample() -> Vec<u8> {
    b"the quick brown fox jumps over the lazy dog. ".repeat(16)
}

#[cfg(leyline_any_codec)]
async fn sent(encoding: Encoding) -> (String, Vec<u8>) {
    use crate::testing::{Response, Server};
    let server = Server::http(|_: &crate::testing::Request| Response::new(200))
        .await
        .expect("server starts");
    let session = crate::Session::builder()
        .protocol(crate::Protocol::Http1)
        .proxy(crate::Proxy::new().env(false))
        .build()
        .expect("session");
    let response = session
        .post(server.url("/upload"))
        .body(sample())
        .compress_body(encoding)
        .send()
        .await
        .expect("response");
    assert_eq!(response.status(), 200);
    let requests = server.requests().await;
    let request = requests.first().expect("one request");
    let token = request
        .header("content-encoding")
        .expect("content-encoding");
    assert!(request.body.len() < sample().len());
    (token.to_owned(), request.body.clone())
}

#[cfg(feature = "compression-gzip")]
#[tokio::test]
async fn gzip_reaches_the_server_as_gzip() {
    use std::io::Read;
    let (token, wire) = sent(Encoding::Gzip).await;
    assert_eq!(token, "gzip");
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&wire[..])
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, sample());
}

#[cfg(feature = "compression-brotli")]
#[tokio::test]
async fn brotli_reaches_the_server_as_br() {
    use std::io::Read;
    let (token, wire) = sent(Encoding::Brotli).await;
    assert_eq!(token, "br");
    let mut out = Vec::new();
    brotli::Decompressor::new(&wire[..], 4096)
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, sample());
}

#[cfg(feature = "compression-zstd")]
#[tokio::test]
async fn zstd_reaches_the_server_as_zstd() {
    let (token, wire) = sent(Encoding::Zstd).await;
    assert_eq!(token, "zstd");
    assert_eq!(zstd::decode_all(&wire[..]).unwrap(), sample());
}

#[cfg(feature = "compression-deflate")]
#[tokio::test]
async fn deflate_reaches_the_server_zlib_wrapped() {
    use std::io::Read;
    let (token, wire) = sent(Encoding::Deflate).await;
    assert_eq!(token, "deflate");
    assert_eq!(wire.first(), Some(&0x78));
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(&wire[..])
        .read_to_end(&mut out)
        .unwrap();
    assert_eq!(out, sample());
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
