use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use crate::asn1::Asn1Time;
use crate::error::ErrorStack;
use crate::ffi;
use crate::hash::MessageDigest;
use crate::pkey::PKey;
use crate::records::{Records, APPLICATION_DATA, MAX_PLAINTEXT};
use crate::rsa::Rsa;
use crate::ssl::{Ssl, SslContext, SslMethod, SslVerifyMode};
use crate::x509::{X509Name, X509};

use super::CryptoBufferBuilder;

fn self_signed() -> (X509, PKey<crate::pkey::Private>) {
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509Name::builder().unwrap();
    name.append_entry_by_text("CN", "localhost").unwrap();
    let name = name.build();
    let mut builder = X509::builder().unwrap();
    builder.set_version(2).unwrap();
    builder.set_subject_name(&name).unwrap();
    builder.set_issuer_name(&name).unwrap();
    builder.set_pubkey(&key).unwrap();
    let not_before = Asn1Time::days_from_now(0).unwrap();
    let not_after = Asn1Time::days_from_now(1).unwrap();
    builder.set_not_before(&not_before).unwrap();
    builder.set_not_after(&not_after).unwrap();
    builder.sign(&key, MessageDigest::sha256()).unwrap();
    (builder.build(), key)
}

#[test]
fn crypto_buffer_builder_handles_zero_and_initialized_writes() {
    let empty = CryptoBufferBuilder::with_capacity(0)
        .unwrap()
        .build()
        .unwrap();
    // SAFETY: `empty` is a live CRYPTO_BUFFER returned by `build()`.
    unsafe {
        assert_eq!(ffi::CRYPTO_BUFFER_len(empty), 0);
        ffi::CRYPTO_BUFFER_free(empty);
    }

    let mut builder = CryptoBufferBuilder::with_capacity(5).unwrap();
    builder.as_writer().write_all(b"hello").unwrap();
    let filled = builder.build().unwrap();
    // SAFETY: `filled` is a live CRYPTO_BUFFER of 5 initialized bytes.
    unsafe {
        let len = ffi::CRYPTO_BUFFER_len(filled);
        let bytes = std::slice::from_raw_parts(ffi::CRYPTO_BUFFER_data(filled), len);
        assert_eq!(bytes, b"hello");
        ffi::CRYPTO_BUFFER_free(filled);
    }
}

#[test]
fn ech_retry_configs_returns_none_without_ech_rejected_error() {
    let ctx = SslContext::builder(SslMethod::tls()).unwrap().build();
    let ssl = Ssl::new(&ctx).unwrap();
    assert!(ssl.ech_retry_configs(&ErrorStack::get()).is_none());
}

#[test]
fn panicking_custom_verify_callback_fails_handshake_without_aborting() {
    let (cert, key) = self_signed();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        let mut ctx = SslContext::builder(SslMethod::tls()).unwrap();
        ctx.set_certificate(&cert).unwrap();
        ctx.set_private_key(&key).unwrap();
        let ssl = Ssl::new(&ctx.build()).unwrap();
        let _ = ssl.accept(sock);
    });

    let sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut ctx = SslContext::builder(SslMethod::tls()).unwrap();
    ctx.set_custom_verify_callback(SslVerifyMode::PEER, |_| panic!("verify boom"));
    let ssl = Ssl::new(&ctx.build()).unwrap();
    assert!(ssl.connect(sock).is_err());
    server.join().unwrap();
}

#[test]
fn records_open_rejects_short_slice_and_seal_rejects_oversized_plaintext() {
    let (cert, key) = self_signed();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        let mut ctx = SslContext::builder(SslMethod::tls()).unwrap();
        ctx.set_certificate(&cert).unwrap();
        ctx.set_private_key(&key).unwrap();
        let ssl = Ssl::new(&ctx.build()).unwrap();
        let mut stream = ssl.accept(sock).unwrap();
        let mut buf = [0u8; 4];
        let _ = stream.read(&mut buf);
    });

    let sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut ctx = SslContext::builder(SslMethod::tls()).unwrap();
    ctx.set_verify(SslVerifyMode::NONE);
    let ssl = Ssl::new(&ctx.build()).unwrap();
    let stream = ssl.connect(sock).unwrap();
    let mut records = Records::new(stream.ssl()).expect("TLS 1.3 records");

    let mut short = [0u8; 5];
    assert!(records.open(&mut short).is_err());

    let mut out = Vec::new();
    assert!(records
        .seal(APPLICATION_DATA, &vec![0u8; MAX_PLAINTEXT + 1], &mut out)
        .is_err());

    drop(stream);
    server.join().unwrap();
}
